// SPDX-License-Identifier: Apache-2.0

/** MLX90614. Implemented from Melexis MLX90614 Rev20 §§4.1.4.3.6–4.1.4.3.8. No third-party decoder source is used.
 * Register pointers persist across STOP, but unknown pointers never become fabricated zeroes.
 * Bytes stream immediately; at most one unfinished two-byte semantic value is retained.
 */
import type { ItemOf, Provenance, Tick, Transaction, bool, u16, u32, u64, u8 } from 'uiharu'
import { BusDecoder, defineBusDecoder, eventSchema, join } from 'uiharu'
import { i2cEvents } from '../contracts/i2c'

type BusItem = ItemOf<typeof i2cEvents>
export type OverlayEnd = 'complete' | 'incomplete' | 'aborted' | 'nack'
export type Field =
  | 'word'
  | 'ambient-centicelsius'
  | 'object-1-centicelsius'
  | 'object-2-centicelsius'
  | 'expected-pec'
  | 'pec'
export type DeviceEvent =
  | { kind: 'begin'; op: u64; tx: u64; read: bool; ack: bool }
  | { kind: 'pointer'; op: u64; register: u8; ack: bool }
  | { kind: 'byte'; op: u64; register: u8 | undefined; value: u8; read: bool; ack: bool }
  | { kind: 'field'; op: u64; register: u8; field: Field; value: u32; negative: bool; valid: bool }
  | { kind: 'end'; op: u64; reason: OverlayEnd }
export const deviceEvents = eventSchema<DeviceEvent>('mlx90614.events', 1).roles({
  begin: { role: 'address', errorWhen: { ack: false } },
  pointer: 'address',
  byte: 'data',
  field: { role: 'data', errorWhen: { valid: false } },
  end: 'framing',
})
export interface DeviceConfig {
  address: u8
}
function validate(config: DeviceConfig): string | undefined {
  if (config.address > 127) return 'decode.config.address'
  return undefined
}
function crc8(crc: u8, byte: u8): u8 {
  let value: u8 = crc ^ byte
  for (let i: u8 = 0; i < 8; i += 1) {
    value = (value & 0x80) !== 0 ? ((value << 1) ^ 7) & 255 : (value << 1) & 255
  }
  return value
}
class Device extends BusDecoder<DeviceConfig, BusItem, DeviceEvent, OverlayEnd> {
  pointer: u8 | undefined = undefined
  nextOp: u64 = 1
  open = false
  currentOp: u64 = 0
  half: u8 | undefined = undefined
  halfAt: Provenance | undefined = undefined
  crc: u8 = 0
  stage: u8 = 0
  commandArmed = false
  override hold(): Tick | undefined {
    return this.halfAt?.span.start
  }
  override onGap(): void {
    this.pointer = undefined
    this.half = undefined
    this.halfAt = undefined
    this.commandArmed = false
    this.stage = 0
  }
  async close(reason: OverlayEnd, where: Provenance): Promise<void> {
    let ending = reason
    if (this.open) {
      const op = this.currentOp
      if (ending === 'complete' && (this.half !== undefined || this.stage === 2))
        ending = 'incomplete'
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
  async interpret(op: u64, register: u8, byte: u8, read: bool, where: Provenance): Promise<void> {
    if (!this.commandArmed) return
    if (this.stage === 0) {
      this.half = byte
      this.halfAt = where
      this.crc = crc8(this.crc, byte)
      this.stage = 1
      return
    }
    if (this.stage === 1) {
      const raw: u16 = (byte << 8) | this.half!
      const at = join(this.halfAt!, where)
      this.half = undefined
      this.halfAt = undefined
      this.crc = crc8(this.crc, byte)
      this.stage = 2
      await this.field(op, register, 'word', raw, false, true, at)
      if (read && register >= 6 && register <= 8) {
        const centiKelvin: u32 = (raw & 0x7fff) * 2
        const negative = centiKelvin < 27315
        const magnitude = negative ? 27315 - centiKelvin : centiKelvin - 27315
        let field: Field = 'ambient-centicelsius'
        if (register === 7) field = 'object-1-centicelsius'
        if (register === 8) field = 'object-2-centicelsius'
        await this.field(op, register, field, magnitude, negative, (raw & 0x8000) === 0, at)
      }
      return
    }
    if (this.stage === 2) {
      await this.field(op, register, 'expected-pec', this.crc, false, true, where)
      await this.field(op, register, 'pec', byte, false, byte === this.crc, where)
      this.stage = 3
      this.commandArmed = false
    }
  }
  advance(_register: u8, _ack: bool): void {}
  override async transaction(tx: Transaction<BusItem>): Promise<void> {
    this.commandArmed = false
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
          if (active) {
            if (read) {
              this.crc = crc8(this.crc, this.config.address * 2 + 1)
              this.stage = 0
            } else {
              this.commandArmed = false
              this.crc = crc8(0, this.config.address * 2)
              this.stage = 0
            }
          } else {
            this.commandArmed = false
          }
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
            this.crc = crc8(this.crc, item.byte)
            this.commandArmed = item.ack
            this.stage = 0
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
  name: 'mlx90614',
  category: 'device',
  config: { defaults: { address: 90 }, validate },
  input: i2cEvents,
  requires: () => ({}),
  output: deviceEvents,
  props: (config) => ({ address: config.address }),
  locales: {
    'en-US': {
      name: 'MLX90614',
      summary: 'SMBus words, infrared temperatures and packet error checks',
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
      name: 'MLX90614',
      summary: 'SMBus 字、红外温度和数据包错误校验',
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
