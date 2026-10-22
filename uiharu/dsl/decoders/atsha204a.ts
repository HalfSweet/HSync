// SPDX-License-Identifier: Apache-2.0

/** ATSHA204A I²C. Implemented from Microchip ATSHA204A DS40002025A §§6 and 8. No third-party decoder source is used.
 * Word address 3 selects a command packet; 0, 1 and 2 are Reset, Sleep and Idle.
 * Packet validation covers a single contiguous addressed segment, excluding its word address.
 * Bytes stream immediately; at most one unfinished two-byte semantic value is retained.
 */
import type { ItemOf, Provenance, Tick, Transaction, bool, u16, u32, u64, u8 } from 'uiharu'
import { BusDecoder, defineBusDecoder, eventSchema, join } from 'uiharu'
import { i2cEvents } from '../contracts/i2c'

type BusItem = ItemOf<typeof i2cEvents>
export type OverlayEnd = 'complete' | 'incomplete' | 'aborted' | 'nack'
export type Field =
  | 'reset'
  | 'sleep'
  | 'idle'
  | 'command-flag'
  | 'unknown-word-address'
  | 'count'
  | 'parameter-1'
  | 'parameter-2'
  | 'command-data'
  | 'response-byte'
  | 'status'
  | 'expected-crc'
  | 'crc'
  | 'checkmac'
  | 'derivekey'
  | 'gendig'
  | 'hmac'
  | 'info'
  | 'lock'
  | 'mac'
  | 'nonce'
  | 'random'
  | 'read'
  | 'updateextra'
  | 'write'
  | 'pause'
  | 'unknown-opcode'
export type DeviceEvent =
  | { kind: 'begin'; op: u64; tx: u64; read: bool; ack: bool }
  | { kind: 'pointer'; op: u64; register: u8; ack: bool }
  | { kind: 'byte'; op: u64; register: u8 | undefined; value: u8; read: bool; ack: bool }
  | { kind: 'field'; op: u64; register: u8; field: Field; value: u32; negative: bool; valid: bool }
  | { kind: 'end'; op: u64; reason: OverlayEnd }
export const deviceEvents = eventSchema<DeviceEvent>('atsha204a.events', 1).roles({
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
function packetCrc(crc: u16, byte: u8): u16 {
  let remainder: u16 = crc
  for (let bit: u8 = 0; bit < 8; bit += 1) {
    const feedback = ((remainder >> 15) & 1) !== ((byte >> (bit as u64)) & 1)
    remainder = feedback ? ((remainder << 1) ^ 0x8005) & 65535 : (remainder << 1) & 65535
  }
  return remainder
}
function opcodeField(opcode: u8): Field {
  switch (opcode) {
    case 0x28:
      return 'checkmac'
    case 0x1c:
      return 'derivekey'
    case 0x15:
      return 'gendig'
    case 0x11:
      return 'hmac'
    case 0x30:
      return 'info'
    case 0x17:
      return 'lock'
    case 0x08:
      return 'mac'
    case 0x16:
      return 'nonce'
    case 0x1b:
      return 'random'
    case 0x02:
      return 'read'
    case 0x20:
      return 'updateextra'
    case 0x12:
      return 'write'
    case 0x47:
      return 'pause'
    default:
      return 'unknown-opcode'
  }
}
class Device extends BusDecoder<DeviceConfig, BusItem, DeviceEvent, OverlayEnd> {
  pointer: u8 | undefined = undefined
  nextOp: u64 = 1
  open = false
  currentOp: u64 = 0
  half: u8 | undefined = undefined
  halfAt: Provenance | undefined = undefined
  position: u8 = 0
  count: u8 = 0
  packetError = false
  check: u16 = 0
  override hold(): Tick | undefined {
    return this.halfAt?.span.start
  }
  override onGap(): void {
    this.pointer = undefined
    this.half = undefined
    this.halfAt = undefined
    this.position = 0
    this.count = 0
    this.packetError = false
  }
  async close(reason: OverlayEnd, where: Provenance): Promise<void> {
    let ending = reason
    if (this.open) {
      const op = this.currentOp
      if (
        ending === 'complete' &&
        (this.packetError ||
          this.half !== undefined ||
          (this.count > 0 && this.position < this.count))
      )
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
    if (!read && register !== 3) return
    if (this.position === 0) {
      this.count = byte
      this.check = 0
      this.position = 1
      await this.field(
        op,
        register,
        'count',
        byte,
        false,
        byte >= (read ? 4 : 7) && byte <= 84,
        where,
      )
      if (byte < (read ? 4 : 7) || byte > 84) {
        this.count = 0
        this.packetError = true
        return
      }
      this.check = packetCrc(this.check, byte)
      return
    }
    if (this.count === 0 || this.position >= this.count) {
      this.packetError = true
      return
    }
    const index = this.position
    this.position += 1
    if (index === this.count - 2) {
      this.half = byte
      this.halfAt = where
      return
    }
    if (index === this.count - 1) {
      const received: u16 = (byte << 8) | this.half!
      const at = join(this.halfAt!, where)
      this.half = undefined
      this.halfAt = undefined
      await this.field(op, register, 'expected-crc', this.check, false, true, at)
      await this.field(op, register, 'crc', received, false, received === this.check, at)
      return
    }
    this.check = packetCrc(this.check, byte)
    if (read) {
      await this.field(
        op,
        register,
        this.count === 4 ? 'status' : 'response-byte',
        byte,
        false,
        true,
        where,
      )
      return
    }
    if (index === 1) {
      await this.field(op, register, opcodeField(byte), byte, false, true, where)
      return
    }
    if (index === 2) {
      await this.field(op, register, 'parameter-1', byte, false, true, where)
      return
    }
    if (index === 3) {
      this.half = byte
      this.halfAt = where
      return
    }
    if (index === 4) {
      const value: u16 = (byte << 8) | this.half!
      const at = join(this.halfAt!, where)
      this.half = undefined
      this.halfAt = undefined
      await this.field(op, register, 'parameter-2', value, false, true, at)
      return
    }
    await this.field(op, register, 'command-data', byte, false, true, where)
  }
  advance(_register: u8, _ack: bool): void {}
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
          this.position = 0
          this.count = 0
          this.packetError = false
          this.check = 0
          if (read) this.pointer = 0
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
            await this.field(
              op,
              item.byte,
              item.byte === 0
                ? 'reset'
                : item.byte === 1
                  ? 'sleep'
                  : item.byte === 2
                    ? 'idle'
                    : item.byte === 3
                      ? 'command-flag'
                      : 'unknown-word-address',
              item.byte,
              false,
              item.byte <= 3,
              where,
            )
            this.position = 0
            this.count = 0
            this.packetError = false
            this.check = 0
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
  name: 'atsha204a',
  category: 'device',
  config: { defaults: { address: 100 }, validate },
  input: i2cEvents,
  requires: () => ({}),
  output: deviceEvents,
  props: (config) => ({ address: config.address }),
  locales: {
    'en-US': {
      name: 'ATSHA204A I²C',
      summary: 'Command and response packets with parameters and CRC16',
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
      name: 'ATSHA204A I²C',
      summary: '带参数和 CRC16 的命令及响应数据包',
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
