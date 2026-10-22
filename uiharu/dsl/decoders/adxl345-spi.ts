// SPDX-License-Identifier: Apache-2.0
/* eslint-disable @typescript-eslint/no-empty-object-type, @typescript-eslint/no-unnecessary-type-assertion -- DSL integer widths and empty configuration records carry IR meaning. */
/** ADXL345 four-wire SPI address, read/write and multibyte register transfers; raw axis bytes retain their device addresses, signed X/Y/Z raw samples and named control fields; no scale/orientation assumed.
 * Public manufacturer specification: https://www.analog.com/media/en/technical-documentation/data-sheets/ADXL345.pdf .
 * Each transfer is bounded state and streaming data. Partial CS frames are skipped;
 * missing required data closes incomplete/aborted rather than inventing bytes. */
import type { bool, Provenance, Tick, Transaction, u8, u16, u32, u64 } from 'uiharu'
import { BusDecoder, defineBusDecoder, eventSchema, join } from 'uiharu'
import { spiEvents, type SpiWord } from './spi'
export type Field =
  | 'rate-millihz'
  | 'tap-threshold-microg'
  | 'offset-x-microg'
  | 'offset-y-microg'
  | 'offset-z-microg'
  | 'tap-duration-micros'
  | 'tap-latency-micros'
  | 'tap-window-micros'
  | 'activity-threshold-microg'
  | 'inactivity-threshold-microg'
  | 'inactivity-time-seconds'
  | 'freefall-threshold-microg'
  | 'freefall-time-millis'
  | 'tap-suppress'
  | 'activity-ac'
  | 'activity-x'
  | 'activity-y'
  | 'activity-z'
  | 'inactivity-ac'
  | 'inactivity-x'
  | 'inactivity-y'
  | 'inactivity-z'
  | 'tap-x'
  | 'tap-y'
  | 'tap-z'
  | 'activity-status-x'
  | 'activity-status-y'
  | 'activity-status-z'
  | 'asleep'
  | 'tap-status-x'
  | 'tap-status-y'
  | 'tap-status-z'
  | 'wakeup-code'
  | 'interrupt-invert'
  | 'fifo-triggered'
  | 'x-microg'
  | 'y-microg'
  | 'z-microg'
  | 'x-raw'
  | 'y-raw'
  | 'z-raw'
  | 'device-id'
  | 'rate'
  | 'low-power'
  | 'measure'
  | 'sleep'
  | 'auto-sleep'
  | 'link'
  | 'interrupt-enable'
  | 'interrupt-map'
  | 'interrupt-source'
  | 'range'
  | 'full-resolution'
  | 'justify'
  | 'three-wire'
  | 'self-test'
  | 'fifo-mode'
  | 'fifo-trigger'
  | 'fifo-samples'
  | 'fifo-entries'
export type End = 'complete' | 'incomplete' | 'aborted' | 'unsupported'
export type Event =
  | { kind: 'begin'; tx: u64 }
  | { kind: 'command'; tx: u64; opcode: u8; address: u16; read: bool; status: u8 | undefined }
  | { kind: 'data'; tx: u64; offset: u32; address: u16; value: u8; read: bool }
  | { kind: 'field'; tx: u64; address: u16; field: Field; value: u32; negative: bool }
  | { kind: 'end'; tx: u64; reason: End }
export const events = eventSchema<Event>('adxl345-spi.events', 1).roles({
  begin: 'framing',
  command: 'command',
  data: 'data',
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
  half: u8 | undefined = undefined
  halfAddress: u16 = 0
  format: u8 | undefined = undefined
  halfAt: Provenance | undefined = undefined
  override hold(): Tick | undefined {
    return this.halfAt?.span.start
  }
  override onGap(): void {
    this.format = undefined
    this.half = undefined
    this.halfAt = undefined
  }
  async field(
    tx: u64,
    address: u16,
    field: Field,
    value: u32,
    negative: bool,
    where: Provenance,
  ): Promise<void> {
    await this.out.emit({ kind: 'field', tx, address, field, value, negative }, where)
  }
  dataRate(code: u8): u32 {
    if (code === 0) return 100
    if (code === 1) return 200
    if (code === 2) return 390
    if (code === 3) return 780
    if (code === 4) return 1560
    if (code === 5) return 3130
    if (code === 6) return 6250
    if (code === 7) return 12500
    if (code === 8) return 25000
    if (code === 9) return 50000
    if (code === 10) return 100000
    if (code === 11) return 200000
    if (code === 12) return 400000
    if (code === 13) return 800000
    if (code === 14) return 1600000
    return 3200000
  }
  async interpret(tx: u64, address: u16, value: u8, read: bool, where: Provenance): Promise<void> {
    if (read && address >= 0x32 && address <= 0x37) {
      if ((address & 1) === 0) {
        this.half = value
        this.halfAddress = address
        this.halfAt = where
        return
      }
      if (
        this.half !== undefined &&
        this.halfAt !== undefined &&
        this.halfAddress + 1 === address
      ) {
        const raw: u16 = (value << 8) | this.half
        const negative = (raw & 0x8000) !== 0
        const magnitude: u32 = negative ? 65536 - raw : raw
        const span = join(this.halfAt, where)
        this.half = undefined
        this.halfAt = undefined
        let field: Field = 'x-raw'
        if (address === 0x35) field = 'y-raw'
        else if (address === 0x37) field = 'z-raw'
        await this.field(tx, address - 1, field, magnitude, negative, span)
        const format = this.format
        if (format !== undefined) {
          const range: u8 = format & 3
          const full = (format & 8) !== 0
          let bits: u8 = 10
          if (full) bits += range
          let units = magnitude
          if ((format & 4) !== 0) units = units >> (16 - bits)
          let sensitivity: u32 = 256
          if (!full) sensitivity = sensitivity >> range
          const microg: u32 = (((units as u64) * 1000000) / (sensitivity as u64)) as u32
          let axis: Field = 'x-microg'
          if (address === 0x35) axis = 'y-microg'
          if (address === 0x37) axis = 'z-microg'
          await this.field(tx, address - 1, axis, microg, negative, span)
        }
      }
      return
    }
    if (address === 29)
      await this.field(tx, address, 'tap-threshold-microg', (value as u32) * 62500, false, where)
    if (address === 33)
      await this.field(tx, address, 'tap-duration-micros', (value as u32) * 625, false, where)
    if (address === 34)
      await this.field(tx, address, 'tap-latency-micros', (value as u32) * 1250, false, where)
    if (address === 35)
      await this.field(tx, address, 'tap-window-micros', (value as u32) * 1250, false, where)
    if (address === 36)
      await this.field(
        tx,
        address,
        'activity-threshold-microg',
        (value as u32) * 62500,
        false,
        where,
      )
    if (address === 37)
      await this.field(
        tx,
        address,
        'inactivity-threshold-microg',
        (value as u32) * 62500,
        false,
        where,
      )
    if (address === 38)
      await this.field(tx, address, 'inactivity-time-seconds', (value as u32) * 1, false, where)
    if (address === 40)
      await this.field(
        tx,
        address,
        'freefall-threshold-microg',
        (value as u32) * 62500,
        false,
        where,
      )
    if (address === 41)
      await this.field(tx, address, 'freefall-time-millis', (value as u32) * 5, false, where)
    if (address === 30) {
      const negative = value >= 128
      const magnitude: u32 = negative ? 256 - value : value
      await this.field(tx, address, 'offset-x-microg', magnitude * 15600, negative, where)
    }
    if (address === 31) {
      const negative = value >= 128
      const magnitude: u32 = negative ? 256 - value : value
      await this.field(tx, address, 'offset-y-microg', magnitude * 15600, negative, where)
    }
    if (address === 32) {
      const negative = value >= 128
      const magnitude: u32 = negative ? 256 - value : value
      await this.field(tx, address, 'offset-z-microg', magnitude * 15600, negative, where)
    }
    if (address === 39) {
      await this.field(tx, address, 'activity-ac', (value >> 7) & 1, false, where)
      await this.field(tx, address, 'activity-x', (value >> 6) & 1, false, where)
      await this.field(tx, address, 'activity-y', (value >> 5) & 1, false, where)
      await this.field(tx, address, 'activity-z', (value >> 4) & 1, false, where)
      await this.field(tx, address, 'inactivity-ac', (value >> 3) & 1, false, where)
      await this.field(tx, address, 'inactivity-x', (value >> 2) & 1, false, where)
      await this.field(tx, address, 'inactivity-y', (value >> 1) & 1, false, where)
      await this.field(tx, address, 'inactivity-z', (value >> 0) & 1, false, where)
    }
    if (address === 42) {
      await this.field(tx, address, 'tap-suppress', (value >> 3) & 1, false, where)
      await this.field(tx, address, 'tap-x', (value >> 2) & 1, false, where)
      await this.field(tx, address, 'tap-y', (value >> 1) & 1, false, where)
      await this.field(tx, address, 'tap-z', (value >> 0) & 1, false, where)
    }
    if (address === 43) {
      await this.field(tx, address, 'activity-status-x', (value >> 6) & 1, false, where)
      await this.field(tx, address, 'activity-status-y', (value >> 5) & 1, false, where)
      await this.field(tx, address, 'activity-status-z', (value >> 4) & 1, false, where)
      await this.field(tx, address, 'asleep', (value >> 3) & 1, false, where)
      await this.field(tx, address, 'tap-status-x', (value >> 2) & 1, false, where)
      await this.field(tx, address, 'tap-status-y', (value >> 1) & 1, false, where)
      await this.field(tx, address, 'tap-status-z', (value >> 0) & 1, false, where)
    }
    if (address === 0) await this.field(tx, address, 'device-id', value, false, where)
    else if (address === 0x2c) {
      await this.field(tx, address, 'rate', value & 15, false, where)
      await this.field(tx, address, 'rate-millihz', this.dataRate(value & 15), false, where)
      await this.field(tx, address, 'low-power', (value >> 4) & 1, false, where)
    } else if (address === 0x2d) {
      await this.field(tx, address, 'wakeup-code', value & 3, false, where)
      await this.field(tx, address, 'measure', (value >> 3) & 1, false, where)
      await this.field(tx, address, 'sleep', (value >> 2) & 1, false, where)
      await this.field(tx, address, 'auto-sleep', (value >> 4) & 1, false, where)
      await this.field(tx, address, 'link', (value >> 5) & 1, false, where)
    } else if (address === 0x2e)
      await this.field(tx, address, 'interrupt-enable', value, false, where)
    else if (address === 0x2f) await this.field(tx, address, 'interrupt-map', value, false, where)
    else if (address === 0x30)
      await this.field(tx, address, 'interrupt-source', value, false, where)
    else if (address === 0x31) {
      this.format = value
      await this.field(tx, address, 'interrupt-invert', (value >> 5) & 1, false, where)
      await this.field(tx, address, 'range', value & 3, false, where)
      await this.field(tx, address, 'full-resolution', (value >> 3) & 1, false, where)
      await this.field(tx, address, 'justify', (value >> 2) & 1, false, where)
      await this.field(tx, address, 'three-wire', (value >> 6) & 1, false, where)
      await this.field(tx, address, 'self-test', (value >> 7) & 1, false, where)
    } else if (address === 0x38) {
      await this.field(tx, address, 'fifo-mode', value >> 6, false, where)
      await this.field(tx, address, 'fifo-trigger', (value >> 5) & 1, false, where)
      await this.field(tx, address, 'fifo-samples', value & 31, false, where)
    } else if (address === 0x39) {
      await this.field(tx, address, 'fifo-triggered', (value >> 7) & 1, false, where)
      await this.field(tx, address, 'fifo-entries', value & 63, false, where)
    }
  }
  override async recover(reason: End, where: Provenance): Promise<void> {
    this.format = undefined
    if (this.open !== undefined) {
      this.half = undefined
      this.halfAt = undefined
      const tx = this.open
      this.open = undefined
      await this.out.emit({ kind: 'end', tx, reason }, where)
    }
  }
  layout(opcode: u8): Layout {
    return {
      address: opcode & 63,
      read: (opcode & 128) !== 0,
      step: (opcode & 64) !== 0 ? 1 : 0,
      count: (opcode & 64) !== 0 ? undefined : 1,
      dummy: 0,
      valid: true,
    }
  }
  override async transaction(tx: Transaction<SpiWord>): Promise<void> {
    this.open = tx.id
    await this.out.emit({ kind: 'begin', tx: tx.id })
    let offset: u32 = 0
    for (;;) {
      const first = await tx.itemOrEnd()
      if (first === undefined) break
      this.half = undefined
      this.halfAt = undefined
      const opcode = first.mosi!
      const layout = this.layout(opcode)
      await this.out.emit({
        kind: 'command',
        tx: tx.id,
        opcode,
        address: layout.address,
        read: layout.read,
        status: first.miso,
      })
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
          this.half = undefined
          this.halfAt = undefined
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
    this.half = undefined
    this.halfAt = undefined
    this.open = undefined
    await this.out.emit({ kind: 'end', tx: tx.id, reason: 'complete' })
  }
}
export const device = defineBusDecoder({
  name: 'adxl345-spi',
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
      name: 'ADXL345-SPI',
      summary: 'ADXL345 signed raw XYZ samples, power/range and FIFO controls',
      fields: {},
      events: { begin: 'Begin', command: 'Command', data: 'Data', field: 'Field', end: 'End' },
      eventFields: {
        tx: 'Transaction',
        opcode: 'Opcode',
        address: 'Address',
        read: 'Read',
        status: 'Status',
        offset: 'Offset',
        value: 'Value',
        field: 'Field',
        negative: 'Negative',
        reason: 'Reason',
      },
      values: {},
    },
    'zh-CN': {
      name: 'ADXL345-SPI',
      summary: '解析ADXL345有符号XYZ原始采样、电源、量程及FIFO控制',
      fields: {},
      events: { begin: '开始', command: '命令', data: '数据通道', field: '字段', end: '结束' },
      eventFields: {
        tx: '事务号',
        opcode: '操作码',
        address: '地址',
        read: '读取',
        status: '状态',
        offset: '偏移',
        value: '数值',
        field: '字段',
        negative: '负数',
        reason: '结束原因',
      },
      values: {},
    },
  },
  decoder: Device,
})
