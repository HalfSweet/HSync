// SPDX-License-Identifier: Apache-2.0
/* eslint-disable @typescript-eslint/no-empty-object-type -- DSL integer widths and empty configuration records carry IR meaning. */
/** Original AMICCOM A7105 datasheet SPI register and strobe headers, raw FIFO/ID bytes; full named register map and selected mode/calibration/TX/code controls; no over-air interpretation.
 * Public manufacturer specification: https://www.mantech.co.za/Datasheets/Products/a7105-amiccom.pdf .
 * Each transfer is bounded state and streaming data. Partial CS frames are skipped;
 * missing required data closes incomplete/aborted rather than inventing bytes. */
import type { bool, Provenance, Transaction, u8, u16, u32, u64 } from 'uiharu'
import { BusDecoder, defineBusDecoder, eventSchema } from 'uiharu'
import { spiEvents, type SpiWord } from './spi'
export type Operation =
  | 'sleep'
  | 'idle'
  | 'standby'
  | 'pll'
  | 'receive'
  | 'transmit'
  | 'reset-tx-fifo'
  | 'reset-rx-fifo'
  | 'read-register'
  | 'write-register'
export type Register =
  | 'mode'
  | 'mode-control'
  | 'calibration-control'
  | 'fifo-i'
  | 'fifo-ii'
  | 'fifo-data'
  | 'id-data'
  | 'rc-osc-i'
  | 'rc-osc-ii'
  | 'rc-osc-iii'
  | 'cko-control'
  | 'gio1-control'
  | 'gio2-control'
  | 'clock'
  | 'data-rate'
  | 'pll-i'
  | 'pll-ii'
  | 'pll-iii'
  | 'pll-iv'
  | 'pll-v'
  | 'tx-i'
  | 'tx-ii'
  | 'delay-i'
  | 'delay-ii'
  | 'rx'
  | 'rx-gain-i'
  | 'rx-gain-ii'
  | 'rx-gain-iii'
  | 'rx-gain-iv'
  | 'rssi-threshold'
  | 'adc-control'
  | 'code-i'
  | 'code-ii'
  | 'code-iii'
  | 'if-calibration-i'
  | 'if-calibration-ii'
  | 'vco-current-calibration'
  | 'vco-band-calibration-i'
  | 'vco-band-calibration-ii'
  | 'battery-detect'
  | 'tx-test'
  | 'rx-dem-test-i'
  | 'rx-dem-test-ii'
  | 'charge-pump'
  | 'crystal-test'
  | 'pll-test'
  | 'vco-test-i'
  | 'vco-test-ii'
  | 'ifat'
  | 'rscale'
  | 'filter-test'
  | 'unknown'
export type Field =
  | 'fec-error'
  | 'crc-error'
  | 'chip-enabled'
  | 'crystal-enabled'
  | 'pll-enabled'
  | 'transmit-state'
  | 'trx-enabled'
  | 'direct-data-sdio'
  | 'auto-rssi'
  | 'auto-if-offset'
  | 'fifo-enabled'
  | 'adc-measurement'
  | 'vco-current-calibration'
  | 'vco-band-calibration'
  | 'if-filter-calibration'
  | 'tx-average-code'
  | 'tx-data-invert'
  | 'tx-manual-enabled'
  | 'gaussian-filter'
  | 'deviation-scale-code'
  | 'whitening-enabled'
  | 'fec-enabled'
  | 'crc-enabled'
  | 'id-length-code'
  | 'preamble-length-code'
  | 'dc-estimation-code'
  | 'id-error-tolerance'
  | 'preamble-detection-code'
  | 'whitening-seed'
  | 'tx-current-code'
  | 'pa-current-code'
  | 'tx-buffer-code'
  | 'reset-request'
  | 'carrier-detected'
  | 'carrier-filter-enabled'
  | 'mode'
  | 'mode-control'
  | 'calibration'
  | 'fifo-data'
  | 'id-data'
  | 'data-rate'
  | 'channel'
  | 'tx-control'
  | 'fifo-length'
  | 'fifo-mode'
  | 'fifo-pointer'
  | 'reserved-pdv-code'
  | 'frequency-deviation-code'
export type End = 'complete' | 'incomplete' | 'aborted' | 'unsupported'
export type Event =
  | { kind: 'begin'; tx: u64 }
  | { kind: 'command'; tx: u64; opcode: u8; address: u16; read: bool; status: u8 | undefined }
  | { kind: 'data'; tx: u64; offset: u32; address: u16; value: u8; read: bool }
  | { kind: 'operation'; tx: u64; operation: Operation }
  | { kind: 'field'; tx: u64; address: u16; field: Field; value: u32 }
  | { kind: 'register'; tx: u64; address: u16; name: Register; read: bool; value: u8 }
  | { kind: 'end'; tx: u64; reason: End }
export const events = eventSchema<Event>('a7105.events', 1).roles({
  begin: 'framing',
  command: 'command',
  data: 'data',
  operation: 'command',
  field: 'data',
  register: 'data',
  end: 'framing',
})
export interface Config {}
interface Layout {
  address: u16
  read: bool
  step: u8
  count: u8 | undefined
  dummy: u8
  valid: bool
}
class Device extends BusDecoder<Config, SpiWord, Event, End> {
  open: u64 | undefined = undefined
  override async recover(reason: End, where: Provenance): Promise<void> {
    if (this.open !== undefined) {
      const tx = this.open
      this.open = undefined
      await this.out.emit({ kind: 'end', tx, reason }, where)
    }
  }
  name(address: u16): Register {
    if (address === 0) return 'mode'
    if (address === 1) return 'mode-control'
    if (address === 2) return 'calibration-control'
    if (address === 3) return 'fifo-i'
    if (address === 4) return 'fifo-ii'
    if (address === 5) return 'fifo-data'
    if (address === 6) return 'id-data'
    if (address === 7) return 'rc-osc-i'
    if (address === 8) return 'rc-osc-ii'
    if (address === 9) return 'rc-osc-iii'
    if (address === 10) return 'cko-control'
    if (address === 11) return 'gio1-control'
    if (address === 12) return 'gio2-control'
    if (address === 13) return 'clock'
    if (address === 14) return 'data-rate'
    if (address === 15) return 'pll-i'
    if (address === 16) return 'pll-ii'
    if (address === 17) return 'pll-iii'
    if (address === 18) return 'pll-iv'
    if (address === 19) return 'pll-v'
    if (address === 20) return 'tx-i'
    if (address === 21) return 'tx-ii'
    if (address === 22) return 'delay-i'
    if (address === 23) return 'delay-ii'
    if (address === 24) return 'rx'
    if (address === 25) return 'rx-gain-i'
    if (address === 26) return 'rx-gain-ii'
    if (address === 27) return 'rx-gain-iii'
    if (address === 28) return 'rx-gain-iv'
    if (address === 29) return 'rssi-threshold'
    if (address === 30) return 'adc-control'
    if (address === 31) return 'code-i'
    if (address === 32) return 'code-ii'
    if (address === 33) return 'code-iii'
    if (address === 34) return 'if-calibration-i'
    if (address === 35) return 'if-calibration-ii'
    if (address === 36) return 'vco-current-calibration'
    if (address === 37) return 'vco-band-calibration-i'
    if (address === 38) return 'vco-band-calibration-ii'
    if (address === 39) return 'battery-detect'
    if (address === 40) return 'tx-test'
    if (address === 41) return 'rx-dem-test-i'
    if (address === 42) return 'rx-dem-test-ii'
    if (address === 43) return 'charge-pump'
    if (address === 44) return 'crystal-test'
    if (address === 45) return 'pll-test'
    if (address === 46) return 'vco-test-i'
    if (address === 47) return 'vco-test-ii'
    if (address === 48) return 'ifat'
    if (address === 49) return 'rscale'
    if (address === 50) return 'filter-test'
    return 'unknown'
  }
  layout(opcode: u8): Layout {
    if ((opcode & 128) !== 0)
      return { address: opcode & 0xf0, read: false, step: 0, count: 0, dummy: 0, valid: true }
    const address: u16 = opcode & 63
    return {
      address,
      read: (opcode & 64) !== 0,
      step: 0,
      count: address === 5 || address === 6 ? undefined : 1,
      dummy: 0,
      valid: address <= 0x32,
    }
  }
  operation(opcode: u8, read: bool): Operation {
    const command: u8 = opcode & 0xf0
    if (command === 0x80) return 'sleep'
    if (command === 0x90) return 'idle'
    if (command === 0xa0) return 'standby'
    if (command === 0xb0) return 'pll'
    if (command === 0xc0) return 'receive'
    if (command === 0xd0) return 'transmit'
    if (command === 0xe0) return 'reset-tx-fifo'
    if (command === 0xf0) return 'reset-rx-fifo'
    return read ? 'read-register' : 'write-register'
  }
  async interpret(tx: u64, address: u16, value: u8, read: bool, where: Provenance): Promise<void> {
    await this.out.emit(
      { kind: 'register', tx, address, name: this.name(address), read, value },
      where,
    )
    if (address === 0 && read) {
      await this.out.emit(
        { kind: 'field', tx, address, field: 'fec-error', value: (value >> 6) & 1 },
        where,
      )
      await this.out.emit(
        { kind: 'field', tx, address, field: 'crc-error', value: (value >> 5) & 1 },
        where,
      )
      await this.out.emit(
        { kind: 'field', tx, address, field: 'chip-enabled', value: (value >> 4) & 1 },
        where,
      )
      await this.out.emit(
        { kind: 'field', tx, address, field: 'crystal-enabled', value: (value >> 3) & 1 },
        where,
      )
      await this.out.emit(
        { kind: 'field', tx, address, field: 'pll-enabled', value: (value >> 2) & 1 },
        where,
      )
      await this.out.emit(
        { kind: 'field', tx, address, field: 'transmit-state', value: (value >> 1) & 1 },
        where,
      )
      await this.out.emit(
        { kind: 'field', tx, address, field: 'trx-enabled', value: (value >> 0) & 1 },
        where,
      )
    }
    if (address === 1) {
      await this.out.emit(
        { kind: 'field', tx, address, field: 'direct-data-sdio', value: (value >> 7) & 1 },
        where,
      )
      await this.out.emit(
        { kind: 'field', tx, address, field: 'auto-rssi', value: (value >> 6) & 1 },
        where,
      )
      await this.out.emit(
        { kind: 'field', tx, address, field: 'auto-if-offset', value: (value >> 5) & 1 },
        where,
      )
      await this.out.emit(
        { kind: 'field', tx, address, field: 'fifo-enabled', value: (value >> 1) & 1 },
        where,
      )
      await this.out.emit(
        { kind: 'field', tx, address, field: 'adc-measurement', value: (value >> 0) & 1 },
        where,
      )
    }
    if (address === 2) {
      await this.out.emit(
        { kind: 'field', tx, address, field: 'vco-current-calibration', value: (value >> 2) & 1 },
        where,
      )
      await this.out.emit(
        { kind: 'field', tx, address, field: 'vco-band-calibration', value: (value >> 1) & 1 },
        where,
      )
      await this.out.emit(
        { kind: 'field', tx, address, field: 'if-filter-calibration', value: (value >> 0) & 1 },
        where,
      )
    }
    if (address === 20) {
      await this.out.emit(
        { kind: 'field', tx, address, field: 'tx-average-code', value: (value >> 6) & 3 },
        where,
      )
      await this.out.emit(
        { kind: 'field', tx, address, field: 'tx-data-invert', value: (value >> 5) & 1 },
        where,
      )
      await this.out.emit(
        { kind: 'field', tx, address, field: 'tx-manual-enabled', value: (value >> 4) & 1 },
        where,
      )
      await this.out.emit(
        { kind: 'field', tx, address, field: 'gaussian-filter', value: (value >> 3) & 1 },
        where,
      )
      await this.out.emit(
        { kind: 'field', tx, address, field: 'deviation-scale-code', value: (value >> 0) & 7 },
        where,
      )
    }
    if (address === 31) {
      await this.out.emit(
        { kind: 'field', tx, address, field: 'whitening-enabled', value: (value >> 5) & 1 },
        where,
      )
      await this.out.emit(
        { kind: 'field', tx, address, field: 'fec-enabled', value: (value >> 4) & 1 },
        where,
      )
      await this.out.emit(
        { kind: 'field', tx, address, field: 'crc-enabled', value: (value >> 3) & 1 },
        where,
      )
      await this.out.emit(
        { kind: 'field', tx, address, field: 'id-length-code', value: (value >> 2) & 1 },
        where,
      )
      await this.out.emit(
        { kind: 'field', tx, address, field: 'preamble-length-code', value: (value >> 0) & 3 },
        where,
      )
    }
    if (address === 32) {
      await this.out.emit(
        { kind: 'field', tx, address, field: 'dc-estimation-code', value: (value >> 4) & 7 },
        where,
      )
      await this.out.emit(
        { kind: 'field', tx, address, field: 'id-error-tolerance', value: (value >> 2) & 3 },
        where,
      )
      await this.out.emit(
        { kind: 'field', tx, address, field: 'preamble-detection-code', value: (value >> 0) & 3 },
        where,
      )
    }
    if (address === 33) {
      await this.out.emit(
        { kind: 'field', tx, address, field: 'whitening-seed', value: (value >> 0) & 127 },
        where,
      )
    }
    if (address === 40) {
      await this.out.emit(
        { kind: 'field', tx, address, field: 'tx-current-code', value: (value >> 5) & 1 },
        where,
      )
      await this.out.emit(
        { kind: 'field', tx, address, field: 'pa-current-code', value: (value >> 3) & 3 },
        where,
      )
      await this.out.emit(
        { kind: 'field', tx, address, field: 'tx-buffer-code', value: (value >> 0) & 7 },
        where,
      )
    }
    if (address === 0 && !read && value === 0)
      await this.out.emit({ kind: 'field', tx, address, field: 'reset-request', value: 1 }, where)
    if (address === 1)
      await this.out.emit(
        {
          kind: 'field',
          tx,
          address,
          field: read ? 'carrier-detected' : 'carrier-filter-enabled',
          value: (value >> 4) & 1,
        },
        where,
      )
    if (address === 0)
      await this.out.emit({ kind: 'field', tx, address, field: 'mode', value }, where)
    if (address === 1)
      await this.out.emit({ kind: 'field', tx, address, field: 'mode-control', value }, where)
    if (address === 2)
      await this.out.emit({ kind: 'field', tx, address, field: 'calibration', value }, where)
    if (address === 5)
      await this.out.emit({ kind: 'field', tx, address, field: 'fifo-data', value }, where)
    if (address === 6)
      await this.out.emit({ kind: 'field', tx, address, field: 'id-data', value }, where)
    if (address === 14)
      await this.out.emit({ kind: 'field', tx, address, field: 'data-rate', value }, where)
    if (address === 15)
      await this.out.emit({ kind: 'field', tx, address, field: 'channel', value }, where)
    if (address === 20)
      await this.out.emit({ kind: 'field', tx, address, field: 'tx-control', value }, where)
    if (address === 3) {
      await this.out.emit(
        { kind: 'field', tx, address, field: 'fifo-length', value: value + 1 },
        where,
      )
    }
    if (address === 4) {
      await this.out.emit(
        { kind: 'field', tx, address, field: 'fifo-mode', value: value >> 6 },
        where,
      )
      await this.out.emit(
        { kind: 'field', tx, address, field: 'fifo-pointer', value: value & 63 },
        where,
      )
    }
    if (address === 21) {
      await this.out.emit(
        { kind: 'field', tx, address, field: 'reserved-pdv-code', value: (value >> 5) & 3 },
        where,
      )
      await this.out.emit(
        { kind: 'field', tx, address, field: 'frequency-deviation-code', value: value & 31 },
        where,
      )
    }
  }
  override async transaction(tx: Transaction<SpiWord>): Promise<void> {
    this.open = tx.id
    await this.out.emit({ kind: 'begin', tx: tx.id })
    let offset: u32 = 0
    for (;;) {
      const first = await tx.itemOrEnd()
      if (first === undefined) break
      const opcode = first.mosi!
      const layout = this.layout(opcode)
      const commandAt = this.consumed()
      await this.out.emit(
        {
          kind: 'command',
          tx: tx.id,
          opcode,
          address: layout.address,
          read: layout.read,
          status: first.miso,
        },
        commandAt,
      )
      await this.out.emit(
        { kind: 'operation', tx: tx.id, operation: this.operation(opcode, layout.read) },
        commandAt,
      )
      if (!layout.valid) {
        await tx.drain()
        this.open = undefined
        await this.out.emit({ kind: 'end', tx: tx.id, reason: 'unsupported' })
        return
      }
      for (let i = 0; i < layout.dummy; i++) await tx.item()
      let address = layout.address
      let count: u32 = 0
      for (;;) {
        if (layout.count !== undefined && count >= layout.count) break
        const word = layout.count === undefined ? await tx.itemOrEnd() : await tx.item()
        if (word === undefined) {
          this.open = undefined
          await this.out.emit({ kind: 'end', tx: tx.id, reason: 'complete' })
          return
        }
        const value = layout.read ? word.miso! : word.mosi!
        const where = this.consumed()
        await this.out.emit(
          { kind: 'data', tx: tx.id, offset, address, value, read: layout.read },
          where,
        )
        await this.interpret(tx.id, address, value, layout.read, where)
        count += 1
        offset += 1
        address += layout.step
      }
    }
    this.open = undefined
    await this.out.emit({ kind: 'end', tx: tx.id, reason: 'complete' })
  }
}
export const device = defineBusDecoder({
  name: 'a7105',
  category: 'device',
  config: { defaults: {} },
  input: spiEvents,
  requires: () => ({ wordBits: 8, bitOrder: 'msb-first', cs: true, mosi: true, miso: true }),
  output: events,
  props: () => ({}),
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
      name: 'A7105',
      summary: 'A7105 radio strobes, register configuration and FIFO transfers',
      fields: {},
      events: {
        begin: 'Begin',
        command: 'Command',
        data: 'Data',
        operation: 'Operation',
        field: 'Field',
        end: 'End',
        register: 'Register',
      },
      eventFields: {
        tx: 'Transaction',
        opcode: 'Opcode',
        address: 'Address',
        read: 'Read',
        status: 'Status',
        offset: 'Offset',
        value: 'Value',
        operation: 'Operation',
        field: 'Field',
        reason: 'Reason',
        name: 'Register name',
      },
      values: {},
    },
    'zh-CN': {
      name: 'A7105',
      summary: '解析A7105射频选通命令、寄存器配置及FIFO传输',
      fields: {},
      events: {
        begin: '开始',
        command: '命令',
        data: '数据通道',
        operation: '操作',
        field: '字段',
        end: '结束',
        register: '寄存器',
      },
      eventFields: {
        tx: '事务号',
        opcode: '操作码',
        address: '地址',
        read: '读取',
        status: '状态',
        offset: '偏移',
        value: '数值',
        operation: '操作',
        field: '字段',
        reason: '结束原因',
        name: '寄存器名称',
      },
      values: {},
    },
  },
  decoder: Device,
})
