// SPDX-License-Identifier: Apache-2.0

/** RTC-8564. Implemented from Epson RTC-8564JE/NB ETM11E-05 §§12.2 and 12.3. No third-party decoder source is used.
 * Register pointers persist across STOP, but unknown pointers never become fabricated zeroes.
 * Bytes stream immediately; at most one unfinished two-byte semantic value is retained.
 */
import type { ItemOf, Provenance, Tick, Transaction, bool, u16, u32, u64, u8 } from 'uiharu'
import { BusDecoder, defineBusDecoder, eventSchema } from 'uiharu'
import { i2cEvents } from '../contracts/i2c'

type BusItem = ItemOf<typeof i2cEvents>
export type OverlayEnd = 'complete' | 'incomplete' | 'aborted' | 'nack'
export type Field =
  | 'second'
  | 'minute'
  | 'hour'
  | 'day'
  | 'weekday'
  | 'month'
  | 'year'
  | 'control'
  | 'alarm-minute'
  | 'alarm-hour'
  | 'alarm-day'
  | 'alarm-weekday'
  | 'alarm-disabled'
  | 'clkout'
  | 'timer-control'
  | 'timer'
  | 'voltage-low'
  | 'century'
export type DeviceEvent =
  | { kind: 'begin'; op: u64; tx: u64; read: bool; ack: bool }
  | { kind: 'pointer'; op: u64; register: u8; ack: bool }
  | { kind: 'byte'; op: u64; register: u8 | undefined; value: u8; read: bool; ack: bool }
  | { kind: 'field'; op: u64; register: u8; field: Field; value: u32; negative: bool; valid: bool }
  | { kind: 'end'; op: u64; reason: OverlayEnd }
export const deviceEvents = eventSchema<DeviceEvent>('rtc8564.events', 1).roles({
  begin: { role: 'address', errorWhen: { ack: false } },
  pointer: 'address',
  byte: 'data',
  field: { role: 'data', errorWhen: { valid: false } },
  end: 'framing',
})
export interface DeviceConfig {
  address: u16
}
function validate(config: DeviceConfig): string | undefined {
  if (config.address > 127) return 'decode.config.address'
  return undefined
}
function bcd(value: u8): u32 {
  return (value >> 4) * 10 + (value & 15)
}
function bcdValid(value: u8, low: u8, high: u8): bool {
  return (value & 15) < 10 && value >> 4 < 10 && bcd(value) >= low && bcd(value) <= high
}

class Device extends BusDecoder<DeviceConfig, BusItem, DeviceEvent, OverlayEnd> {
  pointer: u8 | undefined = undefined
  nextOp: u64 = 1
  open = false
  currentOp: u64 = 0
  half: u8 | undefined = undefined
  halfAt: Provenance | undefined = undefined

  override hold(): Tick | undefined {
    return this.halfAt?.span.start
  }
  override onGap(): void {
    this.pointer = undefined
    this.half = undefined
    this.halfAt = undefined
  }
  async close(reason: OverlayEnd, where: Provenance): Promise<void> {
    let ending = reason
    if (this.open) {
      const op = this.currentOp
      if (ending === 'complete' && this.half !== undefined) ending = 'incomplete'
      this.half = undefined
      this.halfAt = undefined
      this.open = false
      await this.out.emit({ kind: 'end', op, reason: ending }, where)
    }
  }
  override async recover(reason: OverlayEnd, where: Provenance): Promise<void> {
    this.pointer = undefined
    await this.close(reason, where)
  }
  async field(
    op: u64,
    register: u8,
    field: Field,
    value: u32,
    negative: bool,
    valid: bool,
    where: Provenance,
  ): Promise<void> {
    await this.out.emit({ kind: 'field', op, register, field, value, negative, valid }, where)
  }
  async interpret(op: u64, register: u8, byte: u8, _read: bool, where: Provenance): Promise<void> {
    let field: Field = 'second'
    let masked: u8 = byte
    let low: u8 = 0
    let high: u8 = 99
    switch (register) {
      case 2:
        masked = byte & 0x7f
        high = 59
        break
      case 3:
        field = 'minute'
        masked = byte & 0x7f
        high = 59
        break
      case 4:
        field = 'hour'
        masked = byte & 0x3f
        high = 23
        break
      case 5:
        field = 'day'
        masked = byte & 0x3f
        low = 1
        high = 31
        break
      case 6:
        await this.field(op, register, 'weekday', byte & 7, false, (byte & 7) <= 6, where)
        return
      case 7:
        field = 'month'
        masked = byte & 0x1f
        low = 1
        high = 12
        break
      case 8:
        field = 'year'
        break
      case 9:
        field = 'alarm-minute'
        masked = byte & 0x7f
        high = 59
        break
      case 10:
        field = 'alarm-hour'
        masked = byte & 0x3f
        high = 23
        break
      case 11:
        field = 'alarm-day'
        masked = byte & 0x3f
        low = 1
        high = 31
        break
      case 12:
        await this.field(
          op,
          register,
          'alarm-weekday',
          byte & 7,
          false,
          (byte & 7) <= 6 || (byte & 0x80) !== 0,
          where,
        )
        await this.field(op, register, 'alarm-disabled', (byte >> 7) & 1, false, true, where)
        return
      case 13:
        await this.field(op, register, 'clkout', byte, false, true, where)
        return
      case 14:
        await this.field(op, register, 'timer-control', byte, false, true, where)
        return
      case 15:
        await this.field(op, register, 'timer', byte, false, true, where)
        return
      default:
        await this.field(op, register, 'control', byte, false, register < 2, where)
        return
    }
    await this.field(
      op,
      register,
      field,
      bcd(masked),
      false,
      (register >= 9 && (byte & 0x80) !== 0) || bcdValid(masked, low, high),
      where,
    )
    if (register === 2)
      await this.field(op, register, 'voltage-low', (byte >> 7) & 1, false, true, where)
    if (register === 7)
      await this.field(op, register, 'century', (byte >> 7) & 1, false, true, where)
    if (register >= 9 && register <= 12)
      await this.field(op, register, 'alarm-disabled', (byte >> 7) & 1, false, true, where)
  }
  advance(register: u8, _ack: bool): void {
    this.pointer = (register + 1) & 15
  }
  override async transaction(tx: Transaction<BusItem>): Promise<void> {
    let active = false
    let read = false
    let first = false
    for (;;) {
      const item = await tx.itemOrEnd()
      if (item === undefined) {
        await this.close('complete', this.consumed())
        return
      }
      const where = this.consumed()
      switch (item.kind) {
        case 'address': {
          await this.close('complete', where)
          active = !item.tenBit && item.address === this.config.address && item.ack
          read = item.read
          first = !read
          if (!item.tenBit && item.address === this.config.address) {
            const op = this.nextOp
            this.nextOp += 1
            this.currentOp = op
            this.open = true
            await this.out.emit({ kind: 'begin', op, tx: tx.id, read, ack: item.ack }, where)
          }
          continue
        }
        case 'restart':
          await this.close('complete', where)
          active = false
          if (item.droppedBits !== 0) this.pointer = undefined
          continue
        case 'data': {
          if (!active || !this.open) continue
          const op = this.currentOp
          if (first) {
            first = false
            this.pointer = item.ack ? item.byte : undefined
            await this.out.emit({ kind: 'pointer', op, register: item.byte, ack: item.ack }, where)
          } else {
            const register = this.pointer
            await this.out.emit(
              { kind: 'byte', op, register, value: item.byte, read, ack: item.ack },
              where,
            )
            if (register !== undefined && (read || item.ack)) {
              await this.interpret(op, register, item.byte, read, where)
              this.advance(register, item.ack)
            }
          }
          if (!item.ack) {
            if (!read) this.pointer = undefined
            await this.close(read ? 'complete' : 'nack', where)
            active = false
          }
          continue
        }
      }
    }
  }
}
export const device = defineBusDecoder({
  name: 'rtc8564',
  category: 'device',
  config: { defaults: { address: 81 }, validate },
  input: i2cEvents,
  requires: () => ({}),
  output: deviceEvents,
  props: (config) => ({ address: config.address }),
  locales: {
    'en-US': {
      name: 'RTC-8564',
      summary: 'Clock, calendar, alarms and timer registers',
      fields: { address: 'I²C address' },
      events: {
        begin: 'Address',
        pointer: 'Register pointer',
        byte: 'Register byte',
        field: 'Decoded field',
        end: 'End',
      },
      eventFields: {
        op: 'Operation',
        tx: 'I²C transaction',
        read: 'Read',
        ack: 'Acknowledged',
        register: 'Register',
        value: 'Value',
        field: 'Field',
        negative: 'Negative',
        valid: 'Valid',
        reason: 'Reason',
      },
    },
    'zh-CN': {
      name: 'RTC-8564',
      summary: '时钟、日历、闹钟和定时器寄存器',
      fields: { address: 'I²C 地址' },
      events: {
        begin: '地址',
        pointer: '寄存器指针',
        byte: '寄存器字节',
        field: '解码字段',
        end: '结束',
      },
      eventFields: {
        op: '操作号',
        tx: 'I²C 事务',
        read: '读取',
        ack: '应答',
        register: '寄存器',
        value: '值',
        field: '字段',
        negative: '负数',
        valid: '有效',
        reason: '原因',
      },
    },
  },
  partial: 'drain',
  recovery: {
    incomplete: 'incomplete',
    truncated: 'aborted',
    missing: 'aborted',
    gap: 'aborted',
    end: 'aborted',
  },
  decoder: Device,
})
