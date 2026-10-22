// SPDX-License-Identifier: Apache-2.0
/** Avago ADNS-5020-EN register read/write and raw motion burst bytes; connect shared SDIO as SPI MOSI with sharedData enabled; no motion integration.
 * Public manufacturer specification: https://datasheet.octopart.com/ADNS-5020-EN-Avago-datasheet-10313908.pdf .
 * Each transfer is bounded state and streaming data. Partial CS frames are skipped;
 * missing required data closes incomplete/aborted rather than inventing bytes. */
import type { bool, Provenance, Transaction, u8, u16, u32, u64 } from 'uiharu'
import { BusDecoder, defineBusDecoder, eventSchema } from 'uiharu'
import { spiEvents, type SpiWord } from './spi'
export type Operation = 'motion-burst' | 'read-register' | 'write-register'
export type Field =
  | 'product-id'
  | 'revision-id'
  | 'delta-x-raw'
  | 'delta-y-raw'
  | 'surface-quality'
  | 'pixel-sum'
  | 'maximum-pixel'
  | 'shutter-upper'
  | 'shutter-lower'
  | 'chip-reset'
  | 'motion-burst-byte'
  | 'motion'
  | 'overflow'
  | 'resolution-code'
export type End = 'complete' | 'incomplete' | 'aborted' | 'unsupported'
export type Event =
  | { kind: 'begin'; tx: u64 }
  | { kind: 'command'; tx: u64; opcode: u8; address: u16; read: bool; status: u8 | undefined }
  | { kind: 'data'; tx: u64; offset: u32; address: u16; value: u8; read: bool }
  | { kind: 'operation'; tx: u64; operation: Operation }
  | { kind: 'field'; tx: u64; address: u16; field: Field; value: u32 }
  | { kind: 'end'; tx: u64; reason: End }
export const events = eventSchema<Event>('adns5020.events', 1).roles({
  begin: 'framing',
  command: 'command',
  data: 'data',
  operation: 'command',
  field: 'data',
  end: 'framing',
})
export interface Config {
  sharedData: bool
}
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
    return {
      address: opcode & 127,
      read: (opcode & 128) === 0,
      step: 0,
      count: opcode === 0x63 ? undefined : 1,
      dummy: 0,
      valid: true,
    }
  }
  operation(opcode: u8, read: bool): Operation {
    if (opcode === 0x63) return 'motion-burst'
    return read ? 'read-register' : 'write-register'
  }
  async interpret(tx: u64, address: u16, value: u8, where: Provenance): Promise<void> {
    if (address === 0)
      await this.out.emit({ kind: 'field', tx, address, field: 'product-id', value }, where)
    if (address === 1)
      await this.out.emit({ kind: 'field', tx, address, field: 'revision-id', value }, where)
    if (address === 3)
      await this.out.emit({ kind: 'field', tx, address, field: 'delta-x-raw', value }, where)
    if (address === 4)
      await this.out.emit({ kind: 'field', tx, address, field: 'delta-y-raw', value }, where)
    if (address === 5)
      await this.out.emit({ kind: 'field', tx, address, field: 'surface-quality', value }, where)
    if (address === 6)
      await this.out.emit({ kind: 'field', tx, address, field: 'pixel-sum', value }, where)
    if (address === 7)
      await this.out.emit({ kind: 'field', tx, address, field: 'maximum-pixel', value }, where)
    if (address === 8)
      await this.out.emit({ kind: 'field', tx, address, field: 'shutter-upper', value }, where)
    if (address === 9)
      await this.out.emit({ kind: 'field', tx, address, field: 'shutter-lower', value }, where)
    if (address === 58)
      await this.out.emit({ kind: 'field', tx, address, field: 'chip-reset', value }, where)
    if (address === 99)
      await this.out.emit({ kind: 'field', tx, address, field: 'motion-burst-byte', value }, where)
    if (address === 2) {
      await this.out.emit({ kind: 'field', tx, address, field: 'motion', value: value >> 7 }, where)
      await this.out.emit(
        { kind: 'field', tx, address, field: 'overflow', value: (value >> 4) & 1 },
        where,
      )
      await this.out.emit(
        { kind: 'field', tx, address, field: 'resolution-code', value: value & 1 },
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
        const value = layout.read ? (this.config.sharedData ? word.mosi! : word.miso!) : word.mosi!
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
  name: 'adns5020',
  category: 'device',
  config: { defaults: { sharedData: true } },
  input: spiEvents,
  requires: () => ({ wordBits: 8, bitOrder: 'msb-first', cs: true, mosi: true }),
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
      name: 'ADNS5020',
      summary: 'ADNS5020 SDIO registers, motion flags and raw movement bytes',
      fields: { sharedData: 'Shared SDIO data line' },
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
      name: 'ADNS5020',
      summary: '解析ADNS5020共享数据线寄存器、运动标志及原始位移字节',
      fields: { sharedData: '共享SDIO数据线' },
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
