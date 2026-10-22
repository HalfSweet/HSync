// SPDX-License-Identifier: Apache-2.0

/** MCP23008 / MCP23017. Implemented from Microchip MCP23008 DS21919E and MCP23017 DS20001952C §§3.2 and 3.5. No third-party decoder source is used.
 * Register pointers persist across STOP, but unknown pointers never become fabricated zeroes.
 * Bytes stream immediately; at most one unfinished two-byte semantic value is retained.
 */
import type { ItemOf, Provenance, Tick, Transaction, bool, u16, u32, u64, u8 } from 'uiharu'
import { BusDecoder, defineBusDecoder, eventSchema } from 'uiharu'
import { i2cEvents } from '../contracts/i2c'

type BusItem = ItemOf<typeof i2cEvents>
export type OverlayEnd = 'complete' | 'incomplete' | 'aborted' | 'nack'
export type Field =
  | 'input-direction-mask'
  | 'polarity-mask'
  | 'interrupt-enable-mask'
  | 'interrupt-default-mask'
  | 'interrupt-control-mask'
  | 'configuration'
  | 'pullup-mask'
  | 'interrupt-flag-mask'
  | 'interrupt-capture-mask'
  | 'gpio-mask'
  | 'output-latch-mask'
  | 'port'
  | 'unknown'
export type DeviceEvent =
  | { kind: 'begin'; op: u64; tx: u64; read: bool; ack: bool }
  | { kind: 'pointer'; op: u64; register: u8; ack: bool }
  | { kind: 'byte'; op: u64; register: u8 | undefined; value: u8; read: bool; ack: bool }
  | { kind: 'field'; op: u64; register: u8; field: Field; value: u32; negative: bool; valid: bool }
  | { kind: 'end'; op: u64; reason: OverlayEnd }
export const deviceEvents = eventSchema<DeviceEvent>('mcp230xx.events', 1).roles({
  begin: { role: 'address', errorWhen: { ack: false } },
  pointer: 'address',
  byte: 'data',
  field: { role: 'data', errorWhen: { valid: false } },
  end: 'framing',
})
export type Model = 'mcp23008' | 'mcp23017'

export interface DeviceConfig {
  address: u16
  model: Model
  initialBank: bool
  initialSeqOff: bool
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
  bank = this.config.initialBank
  seqOff = this.config.initialSeqOff
  modeKnown = true
  override hold(): Tick | undefined {
    return this.halfAt?.span.start
  }
  override onGap(): void {
    this.pointer = undefined
    this.half = undefined
    this.halfAt = undefined
    this.modeKnown = false
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
  async interpret(op: u64, register: u8, byte: u8, read: bool, where: Provenance): Promise<void> {
    if (!this.modeKnown) {
      await this.field(op, register, 'unknown', byte, false, true, where)
      return
    }
    let index: u8 = register
    let port: u8 = 0
    if (this.config.model === 'mcp23017') {
      if (this.bank) {
        index = register & 15
        port = (register >> 4) & 1
      } else {
        index = register >> 1
        port = register & 1
      }
    }
    let field: Field = 'unknown'
    switch (index) {
      case 0:
        field = 'input-direction-mask'
        break
      case 1:
        field = 'polarity-mask'
        break
      case 2:
        field = 'interrupt-enable-mask'
        break
      case 3:
        field = 'interrupt-default-mask'
        break
      case 4:
        field = 'interrupt-control-mask'
        break
      case 5:
        field = 'configuration'
        break
      case 6:
        field = 'pullup-mask'
        break
      case 7:
        field = 'interrupt-flag-mask'
        break
      case 8:
        field = 'interrupt-capture-mask'
        break
      case 9:
        field = 'gpio-mask'
        break
      case 10:
        field = 'output-latch-mask'
        break
      default:
        break
    }
    await this.field(op, register, field, byte, false, true, where)
    await this.field(op, register, 'port', port, false, true, where)
    if (index === 5) {
      this.seqOff = (byte & 0x20) !== 0
      if (this.config.model === 'mcp23017') this.bank = (byte & 0x80) !== 0
    }
    if (!read && index > 10) this.pointer = undefined
  }
  advance(register: u8, ack: bool): void {
    if (!this.modeKnown) {
      this.pointer = undefined
      return
    }
    if (!ack) return
    if (this.config.model === 'mcp23008') {
      this.pointer = this.seqOff ? register : register >= 10 ? 0 : register + 1
      return
    }
    if (this.seqOff) {
      this.pointer = this.bank ? register : register ^ 1
      return
    }
    this.pointer = register >= (this.bank ? 0x1a : 0x15) ? 0 : register + 1
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
  name: 'mcp230xx',
  category: 'device',
  config: {
    defaults: { address: 32, model: 'mcp23017', initialBank: false, initialSeqOff: false },
    validate,
  },
  input: i2cEvents,
  requires: () => ({}),
  output: deviceEvents,
  props: (config) => ({ address: config.address }),
  locales: {
    'en-US': {
      name: 'MCP23008 / MCP23017',
      summary: 'GPIO register banks, masks and observed IOCON changes',
      fields: {
        address: 'I²C address',
        model: 'Device model',
        initialBank: 'Initial BANK bit',
        initialSeqOff: 'Initial SEQOP bit',
      },
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
      name: 'MCP23008 / MCP23017',
      summary: 'GPIO 寄存器组、掩码和观测到的 IOCON 配置变化',
      fields: {
        address: 'I²C 地址',
        model: '器件型号',
        initialBank: '初始 BANK 位',
        initialSeqOff: '初始 SEQOP 位',
      },
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
