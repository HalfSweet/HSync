// SPDX-License-Identifier: Apache-2.0

/** Wii Nunchuk. Implemented from WiiBrew Wiimote/Extension Controllers/Nunchuck original reverse-engineering report (see protocol page). Original implementation; reference facts and their licenses are documented.
 * Register pointers persist across STOP, but unknown pointers never become fabricated zeroes.
 * Bytes stream immediately; at most one unfinished two-byte semantic value is retained.
 */
import type { ItemOf, Provenance, Tick, Transaction, bool, u16, u32, u64, u8 } from 'uiharu'
import { BusDecoder, defineBusDecoder, eventSchema, join } from 'uiharu'
import { i2cEvents } from '../contracts/i2c'

type BusItem = ItemOf<typeof i2cEvents>
export type OverlayEnd = 'complete' | 'incomplete' | 'aborted' | 'nack'
export type Field =
  | 'configuration'
  | 'stick-x'
  | 'stick-y'
  | 'accel-x'
  | 'accel-y'
  | 'accel-z'
  | 'z-button-pressed'
  | 'c-button-pressed'
  | 'register'
export type DeviceEvent =
  | { kind: 'begin'; op: u64; tx: u64; read: bool; ack: bool }
  | { kind: 'pointer'; op: u64; register: u8; ack: bool }
  | { kind: 'byte'; op: u64; register: u8 | undefined; value: u8; read: bool; ack: bool }
  | { kind: 'field'; op: u64; register: u8; field: Field; value: u32; negative: bool; valid: bool }
  | { kind: 'end'; op: u64; reason: OverlayEnd }
export const deviceEvents = eventSchema<DeviceEvent>('nunchuk.events', 1).roles({
  begin: { role: 'address', errorWhen: { ack: false } },
  pointer: 'address',
  byte: 'data',
  field: { role: 'data', errorWhen: { valid: false } },
  end: 'framing',
})
export interface DeviceConfig {
  address: u16
  encoding: Encoding
}
function validate(config: DeviceConfig): string | undefined {
  if (config.address > 127) return 'decode.config.address'
  return undefined
}
export type Encoding = 'plain' | 'legacy-xor'
class Device extends BusDecoder<DeviceConfig, BusItem, DeviceEvent, OverlayEnd> {
  pointer: u8 | undefined = undefined
  nextOp: u64 = 1
  open = false
  currentOp: u64 = 0
  half: u8 | undefined = undefined
  halfAt: Provenance | undefined = undefined
  plain = this.config.encoding === 'plain'
  plainArmed = false
  axesSeen: u8 = 0
  ax: u8 = 0
  ay: u8 = 0
  az: u8 = 0
  override hold(): Tick | undefined {
    return this.halfAt?.span.start
  }
  override onGap(): void {
    this.pointer = undefined
    this.half = undefined
    this.halfAt = undefined
    this.axesSeen = 0
    this.plainArmed = false
    this.plain = this.config.encoding === 'plain'
  }
  async close(reason: OverlayEnd, where: Provenance): Promise<void> {
    let ending = reason
    if (this.open) {
      const op = this.currentOp
      if (ending === 'complete' && this.half !== undefined) ending = 'incomplete'
      this.half = undefined
      this.halfAt = undefined
      this.axesSeen = 0
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
  async interpret(op: u64, register: u8, byte: u8, read: bool, where: Provenance): Promise<void> {
    if (!read) {
      await this.field(op, register, 'configuration', byte, false, true, where)
      if (register === 0xf0 && byte === 0x55) this.plainArmed = true
      if (register === 0xfb && byte === 0 && this.plainArmed) this.plain = true
      if (register === 0x40 && byte === 0) this.plain = this.config.encoding === 'plain'
      return
    }
    let value: u8 = byte
    if (!this.plain) value = ((byte ^ 0x17) + 0x17) & 255
    switch (register) {
      case 0:
        this.axesSeen = 0
        await this.field(op, register, 'stick-x', value, false, true, where)
        break
      case 1:
        await this.field(op, register, 'stick-y', value, false, true, where)
        break
      case 2:
        this.ax = value
        this.axesSeen = 1
        this.halfAt = where
        break
      case 3:
        if (this.axesSeen === 1) {
          this.ay = value
          this.axesSeen = 2
        }
        break
      case 4:
        if (this.axesSeen === 2) {
          this.az = value
          this.axesSeen = 3
        }
        break
      case 5:
        if (this.axesSeen === 3) {
          const at = join(this.halfAt!, where)
          await this.field(
            op,
            register,
            'accel-x',
            (this.ax << 2) | ((value >> 2) & 3),
            false,
            true,
            at,
          )
          await this.field(
            op,
            register,
            'accel-y',
            (this.ay << 2) | ((value >> 4) & 3),
            false,
            true,
            at,
          )
          await this.field(
            op,
            register,
            'accel-z',
            (this.az << 2) | ((value >> 6) & 3),
            false,
            true,
            at,
          )
        }
        this.halfAt = undefined
        this.axesSeen = 0
        await this.field(
          op,
          register,
          'z-button-pressed',
          (value & 1) === 0 ? 1 : 0,
          false,
          true,
          where,
        )
        await this.field(
          op,
          register,
          'c-button-pressed',
          (value & 2) === 0 ? 1 : 0,
          false,
          true,
          where,
        )
        break
      default:
        await this.field(op, register, 'register', value, false, true, where)
        break
    }
  }
  advance(register: u8, _ack: bool): void {
    this.pointer = register === 255 ? 0 : register + 1
  }
  override async transaction(tx: Transaction<BusItem>): Promise<void> {
    this.axesSeen = 0
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
  name: 'nunchuk',
  category: 'device',
  config: { defaults: { address: 82, encoding: 'plain' }, validate },
  input: i2cEvents,
  requires: () => ({}),
  output: deviceEvents,
  props: (config) => ({ address: config.address }),
  locales: {
    'en-US': {
      name: 'Wii Nunchuk',
      summary: 'Stick, ten-bit acceleration and buttons from extension reports',
      fields: { address: 'I²C address', encoding: 'Data encoding' },
      values: { encoding: { plain: 'Unencrypted', 'legacy-xor': 'Legacy XOR' } },
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
      name: 'Wii Nunchuk',
      summary: '从扩展报告解码摇杆、十位加速度和按键',
      fields: { address: 'I²C 地址', encoding: '数据编码' },
      values: { encoding: { plain: '未加密', 'legacy-xor': '旧式异或' } },
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
