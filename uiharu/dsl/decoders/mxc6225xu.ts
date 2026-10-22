// SPDX-License-Identifier: Apache-2.0

/** MXC6225XU. Implemented from MEMSIC MXC6225XU v1.0 register facts; cross-checked with attributed sigrok reference metadata (see protocol page). Original implementation; reference facts and their licenses are documented.
 * Register pointers persist across STOP, but unknown pointers never become fabricated zeroes.
 * Bytes stream immediately; at most one unfinished two-byte semantic value is retained.
 */
import type { ItemOf, Provenance, Tick, Transaction, bool, u16, u32, u64, u8 } from 'uiharu'
import { BusDecoder, defineBusDecoder, eventSchema } from 'uiharu'
import { i2cEvents } from '../contracts/i2c'

type BusItem = ItemOf<typeof i2cEvents>
export type OverlayEnd = 'complete' | 'incomplete' | 'aborted' | 'nack'
export type Field =
  | 'x-raw'
  | 'y-raw'
  | 'interrupt'
  | 'shake'
  | 'tilt'
  | 'instant-orientation'
  | 'orientation'
  | 'power-down'
  | 'shake-mode'
  | 'shake-threshold-millig'
  | 'shake-count'
  | 'orientation-count'
  | 'unknown'
export type DeviceEvent =
  | { kind: 'begin'; op: u64; tx: u64; read: bool; ack: bool }
  | { kind: 'pointer'; op: u64; register: u8; ack: bool }
  | { kind: 'byte'; op: u64; register: u8 | undefined; value: u8; read: bool; ack: bool }
  | { kind: 'field'; op: u64; register: u8; field: Field; value: u32; negative: bool; valid: bool }
  | { kind: 'end'; op: u64; reason: OverlayEnd }
export const deviceEvents = eventSchema<DeviceEvent>('mxc6225xu.events', 1).roles({
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
    if (register === 0 || register === 1) {
      const negative = (byte & 0x80) !== 0
      await this.field(
        op,
        register,
        register === 0 ? 'x-raw' : 'y-raw',
        negative ? 256 - byte : byte,
        negative,
        true,
        where,
      )
      return
    }
    if (register === 2) {
      await this.field(op, register, 'interrupt', (byte >> 7) & 1, false, true, where)
      await this.field(
        op,
        register,
        'shake',
        (byte >> 5) & 3,
        false,
        ((byte >> 5) & 3) !== 3,
        where,
      )
      await this.field(op, register, 'tilt', (byte >> 4) & 1, false, true, where)
      await this.field(
        op,
        register,
        'instant-orientation',
        (byte >> 2) & 3,
        false,
        (byte & 0x10) === 0,
        where,
      )
      await this.field(op, register, 'orientation', byte & 3, false, true, where)
      return
    }
    if (register === 3) {
      await this.field(op, register, 'power-down', (byte >> 7) & 1, false, true, where)
      await this.field(op, register, 'shake-mode', (byte >> 6) & 1, false, true, where)
      await this.field(
        op,
        register,
        'shake-threshold-millig',
        (((byte >> 4) & 3) + 1) * 500,
        false,
        true,
        where,
      )
      await this.field(op, register, 'shake-count', 16 << ((byte >> 2) & 3), false, true, where)
      await this.field(op, register, 'orientation-count', 16 << (byte & 3), false, true, where)
      return
    }
    await this.field(op, register, 'unknown', byte, false, true, where)
  }
  advance(register: u8, _ack: bool): void {
    this.pointer = register === 255 ? 0 : register + 1
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
  name: 'mxc6225xu',
  category: 'device',
  config: { defaults: { address: 21 }, validate },
  input: i2cEvents,
  requires: () => ({}),
  output: deviceEvents,
  props: (config) => ({ address: config.address }),
  locales: {
    'en-US': {
      name: 'MXC6225XU',
      summary: 'Signed axes, orientation, shake and detection settings',
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
      name: 'MXC6225XU',
      summary: '有符号轴数据、方向、摇动和检测设置',
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
