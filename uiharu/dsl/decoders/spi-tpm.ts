// SPDX-License-Identifier: Apache-2.0
/* eslint-disable @typescript-eslint/no-unnecessary-type-assertion -- Loop indices require an explicit IR width. */
/** TPM PTP SPI register transfers and wait-state flow control, ST33TPHF2XSPI
 * datasheet SPI interface: https://www.st.com/resource/en/datasheet/st33tphf2xspi.pdf .
 * Four-byte header, read/write, 1..64 bytes, 24-bit address. Readiness is the last
 * bit of the last address byte or following wait bytes. No TPM packet/auth parsing. */
import type { bool, Provenance, Tick, Transaction, u8, u32, u64 } from 'uiharu'
import { BusDecoder, defineBusDecoder, eventSchema, join } from 'uiharu'
import { spiEvents, type SpiWord } from './spi'
export type End = 'complete' | 'incomplete' | 'aborted' | 'invalid-header' | 'extra-data'
export type Register =
  | 'access'
  | 'interrupt-enable'
  | 'interrupt-vector'
  | 'interrupt-status'
  | 'interface-capability'
  | 'status'
  | 'data-fifo'
  | 'hash-data-or-fifo'
  | 'hash-start'
  | 'hash-end'
  | 'interface-id'
  | 'extended-data-fifo'
  | 'device-vendor-id'
  | 'revision-id'
  | 'first-legacy-address'
  | 'first-legacy-extension'
  | 'second-legacy-address'
  | 'second-legacy-extension'
  | 'vendor-defined'
  | 'reserved'
  | 'unknown'
export type Event =
  | { kind: 'register'; tx: u64; address: u32; locality: u8 | undefined; name: Register }
  | { kind: 'begin'; tx: u64; read: bool; address: u32; length: u8 }
  | { kind: 'wait'; tx: u64; bytes: u32 }
  | { kind: 'data'; tx: u64; offset: u8; value: u8 }
  | { kind: 'end'; tx: u64; reason: End }
export const events = eventSchema<Event>('spi-tpm.events', 1).roles({
  register: 'command',
  begin: 'command',
  wait: 'handshake',
  data: 'data',
  end: 'framing',
})
export type TpmVersion = 'tpm12' | 'tpm20'
export interface Config {
  tpmVersion: TpmVersion
}
class Tpm extends BusDecoder<Config, SpiWord, Event, End> {
  open: u64 | undefined = undefined
  headerAt: Provenance | undefined = undefined
  override hold(): Tick | undefined {
    return this.headerAt?.span.start
  }
  override async recover(reason: End, where: Provenance): Promise<void> {
    this.headerAt = undefined
    if (this.open !== undefined) {
      const tx = this.open
      this.open = undefined
      await this.out.emit({ kind: 'end', tx, reason }, where)
    }
  }
  registerName(address: u32): Register {
    if ((address & 0xff0000) !== 0xd40000) return 'unknown'
    const locality: u8 = ((address >> 12) & 15) as u8
    if (locality > 4) return 'unknown'
    const offset: u32 = address & 4095
    if (offset === 0) return 'access'
    if (offset >= 8 && offset <= 11) return 'interrupt-enable'
    if (offset === 12) return 'interrupt-vector'
    if (offset >= 16 && offset <= 19) return 'interrupt-status'
    if (offset >= 20 && offset <= 23) return 'interface-capability'
    if (offset >= 24 && offset <= 26) return 'status'
    if (offset === 27 && this.config.tpmVersion === 'tpm20') return 'status'
    if (locality === 4) {
      if (offset === 32) return 'hash-end'
      if (offset === 40) return 'hash-start'
      if (offset >= 36 && offset <= 39) return 'hash-data-or-fifo'
    }
    if (offset >= 36 && offset <= 39) return 'data-fifo'
    if (this.config.tpmVersion === 'tpm20') {
      if (offset >= 48 && offset <= 51) return 'interface-id'
      if (offset >= 128 && offset <= 131) return 'extended-data-fifo'
    }
    if (offset >= 3840 && offset <= 3843) return 'device-vendor-id'
    if (offset === 3844) return 'revision-id'
    if (locality === 0 && this.config.tpmVersion === 'tpm12') {
      if (offset === 3968) return 'first-legacy-address'
      if (offset === 3972) return 'first-legacy-extension'
      if (offset === 3976) return 'second-legacy-address'
      if (offset === 3980) return 'second-legacy-extension'
    }
    if (offset >= 3984) return 'vendor-defined'
    return 'reserved'
  }
  override async transaction(tx: Transaction<SpiWord>): Promise<void> {
    this.open = tx.id
    const header = (await tx.item()).mosi!
    this.headerAt = this.consumed()
    const read = (header & 128) !== 0
    const length: u8 = (header & 63) + 1
    let address: u32 = 0
    let ready = false
    for (let i = 0; i < 3; i++) {
      const word = await tx.item()
      address = (address << 8) | word.mosi!
      if (i === 2) ready = (word.miso! & 1) !== 0
    }
    const headerWhere = join(this.headerAt!, this.consumed())
    this.headerAt = undefined
    await this.out.emit({ kind: 'begin', tx: tx.id, read, address, length }, headerWhere)
    if ((header & 64) !== 0) {
      await tx.drain()
      this.open = undefined
      await this.out.emit({ kind: 'end', tx: tx.id, reason: 'invalid-header' })
      return
    }
    let locality: u8 | undefined = undefined
    if ((address & 0xff0000) === 0xd40000 && ((address >> 12) & 15) <= 4)
      locality = ((address >> 12) & 15) as u8
    await this.out.emit(
      { kind: 'register', tx: tx.id, address, locality, name: this.registerName(address) },
      headerWhere,
    )
    let waits: u32 = 0
    while (!ready) {
      ready = ((await tx.item()).miso! & 1) !== 0
      waits += 1
    }
    if (waits > 0) await this.out.emit({ kind: 'wait', tx: tx.id, bytes: waits })
    for (let offset: u8 = 0; offset < length; offset++) {
      const word = await tx.item()
      const value = read ? word.miso! : word.mosi!
      await this.out.emit({ kind: 'data', tx: tx.id, offset: offset as u8, value })
    }
    let reason: End = 'complete'
    if ((await tx.itemOrEnd()) !== undefined) {
      reason = 'extra-data'
      await tx.drain()
    }
    this.open = undefined
    await this.out.emit({ kind: 'end', tx: tx.id, reason })
  }
}
export const tpm = defineBusDecoder({
  name: 'spi-tpm',
  category: 'device',
  config: { defaults: { tpmVersion: 'tpm20' } },
  input: spiEvents,
  requires: () => ({ wordBits: 8, bitOrder: 'msb-first', cs: true, mosi: true, miso: true }),
  output: events,
  props: (config) => ({ tpmVersion: config.tpmVersion }),
  partial: 'drain',
  recovery: {
    incomplete: 'incomplete',
    truncated: 'aborted',
    missing: 'aborted',
    gap: 'aborted',
    end: 'aborted',
  },

  locales: {
    'en-US': {
      name: 'SPI-TPM',
      summary: 'TPM 1.2/2.0 SPI register headers and wait-state flow control',
      fields: { tpmVersion: 'TPM version' },
      events: {
        begin: 'Begin',
        wait: 'Wait',
        data: 'Data',
        end: 'End',
        register: 'Interface register',
      },
      eventFields: {
        tx: 'Transaction',
        read: 'Read',
        address: 'Address',
        length: 'Length',
        bytes: 'Bytes',
        offset: 'Offset',
        value: 'Value',
        reason: 'Reason',
        locality: 'Locality',
        name: 'Register name',
      },
      values: { tpmVersion: { tpm12: 'TPM 1.2', tpm20: 'TPM 2.0' } },
    },
    'zh-CN': {
      name: 'SPI-TPM',
      summary: '解析TPM 1.2/2.0 SPI寄存器头及等待状态流控',
      fields: { tpmVersion: 'TPM版本' },
      events: {
        begin: '开始',
        wait: '等待',
        data: '数据通道',
        end: '结束',
        register: '接口寄存器',
      },
      eventFields: {
        tx: '事务号',
        read: '读取',
        address: '地址',
        length: '长度',
        bytes: '字节数',
        offset: '偏移',
        value: '数值',
        reason: '结束原因',
        locality: '本地区域',
        name: '寄存器名称',
      },
      values: { tpmVersion: { tpm12: 'TPM 1.2', tpm20: 'TPM 2.0' } },
    },
  },
  decoder: Tpm,
})
