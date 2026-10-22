// SPDX-License-Identifier: Apache-2.0
/* eslint-disable @typescript-eslint/no-empty-object-type -- DSL integer widths and empty configuration records carry IR meaning. */
/** Nordic nRF905 original product specification chapter SPI: configuration bytes, TX/RX payload/address and channel-config command; over-the-air state is not inferred.
 * Public manufacturer specification: https://cdn.sparkfun.com/assets/5/8/c/f/8/nRF905_rev1_1.pdf .
 * Each transfer is bounded state and streaming data. Partial CS frames are skipped;
 * missing required data closes incomplete/aborted rather than inventing bytes. */
import type { bool, Provenance, Transaction, u8, u16, u32, u64 } from 'uiharu'
import { BusDecoder, defineBusDecoder, eventSchema } from 'uiharu'
import { spiEvents, type SpiWord } from './spi'
export type Operation =
  | 'write-config'
  | 'read-config'
  | 'write-tx-payload'
  | 'read-tx-payload'
  | 'write-tx-address'
  | 'read-tx-address'
  | 'read-rx-payload'
  | 'channel-config'
  | 'read-register'
  | 'write-register'
export type Field =
  | 'channel-low'
  | 'rx-address-0'
  | 'rx-address-1'
  | 'rx-address-2'
  | 'rx-address-3'
  | 'tx-payload'
  | 'tx-address'
  | 'rx-payload'
  | 'channel-high'
  | 'high-frequency'
  | 'power-code'
  | 'reduced-rx-power'
  | 'auto-retransmit'
  | 'rx-address-bytes'
  | 'tx-address-bytes'
  | 'rx-payload-bytes'
  | 'tx-payload-bytes'
  | 'crc-16'
  | 'crc-enabled'
  | 'crystal-code'
  | 'clock-output-enabled'
  | 'clock-output-code'
export type End = 'complete' | 'incomplete' | 'aborted' | 'unsupported'
export type Event =
  | { kind: 'begin'; tx: u64 }
  | { kind: 'command'; tx: u64; opcode: u8; address: u16; read: bool; status: u8 | undefined }
  | { kind: 'data'; tx: u64; offset: u32; address: u16; value: u8; read: bool }
  | { kind: 'operation'; tx: u64; operation: Operation }
  | { kind: 'field'; tx: u64; address: u16; field: Field; value: u32 }
  | { kind: 'end'; tx: u64; reason: End }
export const events = eventSchema<Event>('nrf905.events', 1).roles({
  begin: 'framing',
  command: 'command',
  data: 'data',
  operation: 'command',
  field: 'data',
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
  layout(opcode: u8): Layout {
    if (opcode < 0x20)
      return {
        address: opcode & 15,
        read: (opcode & 16) !== 0,
        step: 1,
        count: undefined,
        dummy: 0,
        valid: true,
      }
    if (opcode >= 0x80)
      return { address: opcode & 15, read: false, step: 0, count: 1, dummy: 0, valid: true }
    return {
      address: opcode,
      read: opcode === 0x21 || opcode === 0x23 || opcode === 0x24,
      step: 0,
      count: undefined,
      dummy: 0,
      valid: opcode >= 0x20 && opcode <= 0x24,
    }
  }
  operation(opcode: u8, read: bool): Operation {
    if (opcode < 0x10) return 'write-config'
    if (opcode >= 0x10 && opcode < 0x20) return 'read-config'
    if (opcode === 0x20) return 'write-tx-payload'
    if (opcode === 0x21) return 'read-tx-payload'
    if (opcode === 0x22) return 'write-tx-address'
    if (opcode === 0x23) return 'read-tx-address'
    if (opcode === 0x24) return 'read-rx-payload'
    if (opcode >= 0x80) return 'channel-config'
    return read ? 'read-register' : 'write-register'
  }
  async interpret(tx: u64, address: u16, value: u8, where: Provenance): Promise<void> {
    if (address === 0)
      await this.out.emit({ kind: 'field', tx, address, field: 'channel-low', value }, where)
    if (address === 5)
      await this.out.emit({ kind: 'field', tx, address, field: 'rx-address-0', value }, where)
    if (address === 6)
      await this.out.emit({ kind: 'field', tx, address, field: 'rx-address-1', value }, where)
    if (address === 7)
      await this.out.emit({ kind: 'field', tx, address, field: 'rx-address-2', value }, where)
    if (address === 8)
      await this.out.emit({ kind: 'field', tx, address, field: 'rx-address-3', value }, where)
    if (address === 32)
      await this.out.emit({ kind: 'field', tx, address, field: 'tx-payload', value }, where)
    if (address === 33)
      await this.out.emit({ kind: 'field', tx, address, field: 'tx-payload', value }, where)
    if (address === 34)
      await this.out.emit({ kind: 'field', tx, address, field: 'tx-address', value }, where)
    if (address === 35)
      await this.out.emit({ kind: 'field', tx, address, field: 'tx-address', value }, where)
    if (address === 36)
      await this.out.emit({ kind: 'field', tx, address, field: 'rx-payload', value }, where)
    if (address === 1) {
      await this.out.emit(
        { kind: 'field', tx, address, field: 'channel-high', value: value & 1 },
        where,
      )
      await this.out.emit(
        { kind: 'field', tx, address, field: 'high-frequency', value: (value >> 1) & 1 },
        where,
      )
      await this.out.emit(
        { kind: 'field', tx, address, field: 'power-code', value: (value >> 2) & 3 },
        where,
      )
      await this.out.emit(
        { kind: 'field', tx, address, field: 'reduced-rx-power', value: (value >> 4) & 1 },
        where,
      )
      await this.out.emit(
        { kind: 'field', tx, address, field: 'auto-retransmit', value: (value >> 5) & 1 },
        where,
      )
    }
    if (address === 2) {
      await this.out.emit(
        { kind: 'field', tx, address, field: 'rx-address-bytes', value: value & 7 },
        where,
      )
      await this.out.emit(
        { kind: 'field', tx, address, field: 'tx-address-bytes', value: (value >> 4) & 7 },
        where,
      )
    }
    if (address === 3) {
      await this.out.emit(
        { kind: 'field', tx, address, field: 'rx-payload-bytes', value: value & 63 },
        where,
      )
    }
    if (address === 4) {
      await this.out.emit(
        { kind: 'field', tx, address, field: 'tx-payload-bytes', value: value & 63 },
        where,
      )
    }
    if (address === 9) {
      await this.out.emit({ kind: 'field', tx, address, field: 'crc-16', value: value >> 7 }, where)
      await this.out.emit(
        { kind: 'field', tx, address, field: 'crc-enabled', value: (value >> 6) & 1 },
        where,
      )
      await this.out.emit(
        { kind: 'field', tx, address, field: 'crystal-code', value: (value >> 3) & 7 },
        where,
      )
      await this.out.emit(
        { kind: 'field', tx, address, field: 'clock-output-enabled', value: (value >> 2) & 1 },
        where,
      )
      await this.out.emit(
        { kind: 'field', tx, address, field: 'clock-output-code', value: value & 3 },
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
        await this.interpret(tx.id, address, value, where)
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
  name: 'nrf905',
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
      name: 'NRF905',
      summary: 'nRF905 payload, address and radio configuration fields',
      fields: {},
      events: {
        begin: 'Begin',
        command: 'Command',
        data: 'Data',
        operation: 'Operation',
        field: 'Field',
        end: 'End',
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
      },
      values: {},
    },
    'zh-CN': {
      name: 'NRF905',
      summary: '解析nRF905载荷、地址及射频配置字段',
      fields: {},
      events: {
        begin: '开始',
        command: '命令',
        data: '数据通道',
        operation: '操作',
        field: '字段',
        end: '结束',
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
      },
      values: {},
    },
  },
  decoder: Device,
})
