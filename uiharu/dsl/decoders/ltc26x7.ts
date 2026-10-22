// SPDX-License-Identifier: Apache-2.0

/** LTC2607 / LTC2617 / LTC2627. Implemented from Analog Devices 26071727fa pp14–15, figures3–4 and table2. No third-party decoder source is used.
 * Register pointers persist across STOP, but unknown pointers never become fabricated zeroes.
 * Bytes stream immediately; at most one unfinished two-byte semantic value is retained.
 */
import type { ItemOf, Provenance, Tick, Transaction, bool, u16, u32, u64, u8 } from 'uiharu'
import { BusDecoder, defineBusDecoder, eventSchema, join, mulDiv } from 'uiharu'
import { i2cEvents } from '../contracts/i2c'

type BusItem = ItemOf<typeof i2cEvents>
export type OverlayEnd = 'complete' | 'incomplete' | 'aborted' | 'nack'
export type Field =
  | 'write-input'
  | 'update'
  | 'write-update'
  | 'power-down'
  | 'no-operation'
  | 'reserved'
  | 'channel'
  | 'dac-code'
  | 'output-microvolts'
export type DeviceEvent =
  | { kind: 'begin'; op: u64; tx: u64; read: bool; ack: bool }
  | { kind: 'pointer'; op: u64; register: u8; ack: bool }
  | { kind: 'byte'; op: u64; register: u8 | undefined; value: u8; read: bool; ack: bool }
  | { kind: 'field'; op: u64; register: u8; field: Field; value: u32; negative: bool; valid: bool }
  | { kind: 'end'; op: u64; reason: OverlayEnd }
export const deviceEvents = eventSchema<DeviceEvent>('ltc26x7.events', 1).roles({
  begin: { role: 'address', errorWhen: { ack: false } },
  pointer: 'address',
  byte: 'data',
  field: { role: 'data', errorWhen: { valid: false } },
  end: 'framing',
})
export interface DeviceConfig {
  address: u16
  model: Model
  referenceMicrovolts: u32
}
function validate(config: DeviceConfig): string | undefined {
  if (config.referenceMicrovolts === 0 || config.referenceMicrovolts > 5500000)
    return 'decode.config.reference'
  if (config.address > 127) return 'decode.config.address'
  return undefined
}
export type Model = 'ltc2607' | 'ltc2617' | 'ltc2627'
class Device extends BusDecoder<DeviceConfig, BusItem, DeviceEvent, OverlayEnd> {
  pointer: u8 | undefined = undefined
  nextOp: u64 = 1
  open = false
  currentOp: u64 = 0
  half: u8 | undefined = undefined
  halfAt: Provenance | undefined = undefined
  done = false
  commandAt: Provenance | undefined = undefined
  override hold(): Tick | undefined {
    const first = this.commandAt
    if (first !== undefined) return first.span.start
    const half = this.halfAt
    if (half !== undefined) return half.span.start
    return undefined
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
      if (
        ending === 'complete' &&
        (this.half !== undefined || (this.commandAt !== undefined && !this.done))
      )
        ending = 'incomplete'
      this.half = undefined
      this.halfAt = undefined
      this.commandAt = undefined
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
    if (read || this.done) return
    if (this.half === undefined) {
      this.half = byte
      this.halfAt = where
      return
    }
    const raw: u16 = (this.half << 8) | byte
    const at = join(this.commandAt!, where)
    this.half = undefined
    this.halfAt = undefined
    this.commandAt = undefined
    this.done = true
    const command: u8 = register >> 4
    const channel: u8 = register & 15
    let field: Field = 'reserved'
    switch (command) {
      case 0:
        field = 'write-input'
        break
      case 1:
        field = 'update'
        break
      case 3:
        field = 'write-update'
        break
      case 4:
        field = 'power-down'
        break
      case 15:
        field = 'no-operation'
        break
      default:
        break
    }
    await this.field(op, register, field, command, false, field !== 'reserved', at)
    await this.field(
      op,
      register,
      'channel',
      channel,
      false,
      channel === 0 || channel === 1 || channel === 15,
      at,
    )
    const value: u32 =
      this.config.model === 'ltc2607' ? raw : this.config.model === 'ltc2617' ? raw >> 2 : raw >> 4
    const paddingOk =
      this.config.model === 'ltc2607' ||
      (this.config.model === 'ltc2617' ? (raw & 3) === 0 : (raw & 15) === 0)
    await this.field(op, register, 'dac-code', value, false, paddingOk, at)
    if (command === 0 || command === 3)
      await this.field(
        op,
        register,
        'output-microvolts',
        mulDiv(
          value,
          this.config.referenceMicrovolts,
          this.config.model === 'ltc2607' ? 65536 : this.config.model === 'ltc2617' ? 16384 : 4096,
        ) as u32,
        false,
        paddingOk && (channel === 0 || channel === 1 || channel === 15),
        at,
      )
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
          active =
            !item.tenBit &&
            (item.address === this.config.address || item.address === 0x73) &&
            !item.read &&
            item.ack
          read = item.read
          first = !read
          if (!item.tenBit && (item.address === this.config.address || item.address === 0x73)) {
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
            this.done = false
            this.commandAt = where
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
  name: 'ltc26x7',
  category: 'device',
  config: { defaults: { address: 16, model: 'ltc2607', referenceMicrovolts: 1500000 }, validate },
  input: i2cEvents,
  requires: () => ({}),
  output: deviceEvents,
  props: (config) => ({ address: config.address }),
  locales: {
    'en-US': {
      name: 'LTC2607 / LTC2617 / LTC2627',
      summary: 'Three-byte DAC commands, channel selection and codes',
      fields: { address: 'I²C address', model: 'DAC model', referenceMicrovolts: 'Reference (µV)' },
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
      name: 'LTC2607 / LTC2617 / LTC2627',
      summary: '三字节 DAC 命令、通道选择和数码',
      fields: { address: 'I²C 地址', model: 'DAC 型号', referenceMicrovolts: '参考电压（µV）' },
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
