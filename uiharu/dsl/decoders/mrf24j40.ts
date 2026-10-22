// SPDX-License-Identifier: Apache-2.0
/* eslint-disable @typescript-eslint/no-empty-object-type -- DSL integer widths and empty configuration records carry IR meaning. */
/** MRF24J40 short and long SPI register accesses, raw FIFO values; no PHY/frame/security interpretation.
 * Public manufacturer specification: https://ww1.microchip.com/downloads/en/devicedoc/ds-39776b.pdf .
 * Each transfer is bounded state and streaming data. Partial CS frames are skipped;
 * missing required data closes incomplete/aborted rather than inventing bytes. */
import type { bool, Provenance, Range, Tick, Transaction, u8, u16, u32, u64 } from 'uiharu'
import { BusDecoder, defineBusDecoder, eventSchema, at } from 'uiharu'
import { spiEvents, type SpiWord } from './spi'
export type Operation =
  | 'read-long-register'
  | 'write-long-register'
  | 'read-short-register'
  | 'write-short-register'
  | 'read-register'
  | 'write-register'
export type Field =
  | 'tx-pending'
  | 'wake-control'
  | 'pa-control-1'
  | 'pa-control-2'
  | 'pa-control-3'
  | 'fifo-enable'
  | 'tx-normal-control'
  | 'tx-gts1-control'
  | 'tx-gts2-control'
  | 'tx-status'
  | 'interrupt-status'
  | 'interrupt-control'
  | 'rx-frame-length'
  | 'tx-failed'
  | 'cca-failed'
  | 'retry-count'
  | 'tx-trigger'
  | 'security-enabled'
  | 'ack-request'
  | 'tx-normal-interrupt'
  | 'rx-interrupt'
  | 'security-interrupt'
export type End = 'complete' | 'incomplete' | 'aborted' | 'unsupported'
export type Event =
  | { kind: 'begin'; tx: u64 }
  | { kind: 'command'; tx: u64; opcode: u8; address: u16; read: bool; status: u8 | undefined }
  | { kind: 'data'; tx: u64; offset: u32; address: u16; value: u8; read: bool }
  | { kind: 'operation'; tx: u64; operation: Operation }
  | { kind: 'field'; tx: u64; address: u16; field: Field; value: u32 }
  | { kind: 'frame-begin'; frame: u64; receive: bool; length: u8 }
  | { kind: 'frame-byte'; frame: u64; receive: bool; offset: u8; value: u8 }
  | { kind: 'frame-end'; frame: u64; receive: bool; reason: End }
  | { kind: 'end'; tx: u64; reason: End }
export const events = eventSchema<Event>('mrf24j40.events', 1).roles({
  begin: 'framing',
  command: 'command',
  data: 'data',
  operation: 'command',
  field: 'data',
  'frame-begin': 'framing',
  'frame-byte': 'data',
  'frame-end': 'framing',
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
  nextFrame: u64 = 1
  txFrame: u64 | undefined = undefined
  rxFrame: u64 | undefined = undefined
  txLeft: u8 = 0
  rxLeft: u8 = 0
  txOffset: u8 = 0
  rxOffset: u8 = 0
  override async onGap(range: Range): Promise<void> {
    await this.cutFrames(at(range.start))
  }
  override async onEnd(tick: Tick): Promise<void> {
    await this.cutFrames(at(tick))
  }
  async cutFrames(where: Provenance): Promise<void> {
    if (this.txFrame !== undefined) {
      await this.out.emit(
        { kind: 'frame-end', frame: this.txFrame, receive: false, reason: 'aborted' },
        where,
      )
      this.txFrame = undefined
    }
    if (this.rxFrame !== undefined) {
      await this.out.emit(
        { kind: 'frame-end', frame: this.rxFrame, receive: true, reason: 'aborted' },
        where,
      )
      this.rxFrame = undefined
    }
  }
  async frameData(address: u16, value: u8, read: bool, where: Provenance): Promise<void> {
    if (!read && address === 1) {
      if (this.txFrame !== undefined)
        await this.out.emit(
          { kind: 'frame-end', frame: this.txFrame, receive: false, reason: 'incomplete' },
          where,
        )
      this.txFrame = undefined
      if (value > 0 && value <= 127) {
        const frame = this.nextFrame
        this.nextFrame += 1
        this.txFrame = frame
        this.txLeft = value
        this.txOffset = 0
        await this.out.emit({ kind: 'frame-begin', frame, receive: false, length: value }, where)
      }
      return
    }
    if (read && address === 0x300) {
      if (this.rxFrame !== undefined)
        await this.out.emit(
          { kind: 'frame-end', frame: this.rxFrame, receive: true, reason: 'incomplete' },
          where,
        )
      this.rxFrame = undefined
      if (value > 0 && value <= 127) {
        const frame = this.nextFrame
        this.nextFrame += 1
        this.rxFrame = frame
        this.rxLeft = value
        this.rxOffset = 0
        await this.out.emit({ kind: 'frame-begin', frame, receive: true, length: value }, where)
      }
      return
    }
    if (!read && this.txFrame !== undefined && address === 2 + this.txOffset) {
      const frame = this.txFrame
      await this.out.emit(
        { kind: 'frame-byte', frame, receive: false, offset: this.txOffset, value },
        where,
      )
      this.txLeft -= 1
      this.txOffset += 1
      if (this.txLeft === 0) {
        this.txFrame = undefined
        await this.out.emit({ kind: 'frame-end', frame, receive: false, reason: 'complete' }, where)
      }
    }
    if (read && this.rxFrame !== undefined && address === 0x301 + this.rxOffset) {
      const frame = this.rxFrame
      await this.out.emit(
        { kind: 'frame-byte', frame, receive: true, offset: this.rxOffset, value },
        where,
      )
      this.rxLeft -= 1
      this.rxOffset += 1
      if (this.rxLeft === 0) {
        this.rxFrame = undefined
        await this.out.emit({ kind: 'frame-end', frame, receive: true, reason: 'complete' }, where)
      }
    }
  }

  override async recover(reason: End, where: Provenance): Promise<void> {
    if (this.open !== undefined) {
      const tx = this.open
      this.open = undefined
      await this.out.emit({ kind: 'end', tx, reason }, where)
    }
  }
  async layout(tx: Transaction<SpiWord>, opcode: u8): Promise<Layout> {
    if ((opcode & 128) !== 0) {
      const low = (await tx.item()).mosi!
      return {
        address: ((opcode & 127) << 3) | (low >> 5),
        read: (low & 16) === 0,
        step: 0,
        count: 1,
        dummy: 0,
        valid: true,
      }
    }
    return {
      address: (opcode >> 1) & 63,
      read: (opcode & 1) === 0,
      step: 0,
      count: 1,
      dummy: 0,
      valid: true,
    }
  }
  operation(opcode: u8, read: bool): Operation {
    if ((opcode & 128) !== 0) return read ? 'read-long-register' : 'write-long-register'
    if ((opcode & 128) === 0) return read ? 'read-short-register' : 'write-short-register'
    return read ? 'read-register' : 'write-register'
  }
  async interpret(tx: u64, address: u16, value: u8, read: bool, where: Provenance): Promise<void> {
    await this.frameData(address, value, read, where)
    if (address === 16)
      await this.out.emit({ kind: 'field', tx, address, field: 'tx-pending', value }, where)
    if (address === 17)
      await this.out.emit({ kind: 'field', tx, address, field: 'wake-control', value }, where)
    if (address === 18)
      await this.out.emit({ kind: 'field', tx, address, field: 'pa-control-1', value }, where)
    if (address === 19)
      await this.out.emit({ kind: 'field', tx, address, field: 'pa-control-2', value }, where)
    if (address === 20)
      await this.out.emit({ kind: 'field', tx, address, field: 'pa-control-3', value }, where)
    if (address === 21)
      await this.out.emit({ kind: 'field', tx, address, field: 'fifo-enable', value }, where)
    if (address === 27)
      await this.out.emit({ kind: 'field', tx, address, field: 'tx-normal-control', value }, where)
    if (address === 28)
      await this.out.emit({ kind: 'field', tx, address, field: 'tx-gts1-control', value }, where)
    if (address === 29)
      await this.out.emit({ kind: 'field', tx, address, field: 'tx-gts2-control', value }, where)
    if (address === 36)
      await this.out.emit({ kind: 'field', tx, address, field: 'tx-status', value }, where)
    if (address === 49)
      await this.out.emit({ kind: 'field', tx, address, field: 'interrupt-status', value }, where)
    if (address === 50)
      await this.out.emit({ kind: 'field', tx, address, field: 'interrupt-control', value }, where)
    if (address === 768)
      await this.out.emit({ kind: 'field', tx, address, field: 'rx-frame-length', value }, where)
    if (address === 36) {
      await this.out.emit(
        { kind: 'field', tx, address, field: 'tx-failed', value: value & 1 },
        where,
      )
      await this.out.emit(
        { kind: 'field', tx, address, field: 'cca-failed', value: (value >> 5) & 1 },
        where,
      )
      await this.out.emit(
        { kind: 'field', tx, address, field: 'retry-count', value: value >> 6 },
        where,
      )
    }
    if (address === 27) {
      await this.out.emit(
        { kind: 'field', tx, address, field: 'tx-trigger', value: value & 1 },
        where,
      )
      await this.out.emit(
        { kind: 'field', tx, address, field: 'security-enabled', value: (value >> 1) & 1 },
        where,
      )
      await this.out.emit(
        { kind: 'field', tx, address, field: 'ack-request', value: (value >> 2) & 1 },
        where,
      )
    }
    if (address === 49) {
      await this.out.emit(
        { kind: 'field', tx, address, field: 'tx-normal-interrupt', value: value & 1 },
        where,
      )
      await this.out.emit(
        { kind: 'field', tx, address, field: 'rx-interrupt', value: (value >> 3) & 1 },
        where,
      )
      await this.out.emit(
        { kind: 'field', tx, address, field: 'security-interrupt', value: (value >> 4) & 1 },
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
      const layout = await this.layout(tx, opcode)
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
  name: 'mrf24j40',
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
      name: 'MRF24J40',
      summary: 'MRF24J40 register/FIFO frames, transmit retries and failure flags',
      fields: {},
      events: {
        begin: 'Begin',
        command: 'Command',
        data: 'Data',
        operation: 'Operation',
        field: 'Field',
        'frame-begin': 'Frame begin',
        'frame-byte': 'Frame byte',
        'frame-end': 'Frame end',
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
        frame: 'Frame',
        receive: 'Receive',
        length: 'Length',
        reason: 'Reason',
      },
      values: {},
    },
    'zh-CN': {
      name: 'MRF24J40',
      summary: '解析MRF24J40寄存器、FIFO帧、发送重试及失败标志',
      fields: {},
      events: {
        begin: '开始',
        command: '命令',
        data: '数据通道',
        operation: '操作',
        field: '字段',
        'frame-begin': '帧开始',
        'frame-byte': '帧字节',
        'frame-end': '帧结束',
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
        frame: '帧编号',
        receive: '接收',
        length: '长度',
        reason: '结束原因',
      },
      values: {},
    },
  },
  decoder: Device,
})
