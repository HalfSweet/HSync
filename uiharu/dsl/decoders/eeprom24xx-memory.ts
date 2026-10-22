// SPDX-License-Identifier: Apache-2.0
/** EEPROM memory addresses from Microchip 24LC04B DS20001708, 24LC256 DS20001203,
 * and 34AA04 DS20005271C. Observed writes are provisional until a normal STOP.
 * State is a counter, one address accumulator and bounded scalar bookkeeping. */
import type { ItemOf, Transaction, Provenance, bool, u8, u16, u32, u64 } from 'uiharu'
import { BusDecoder, defineBusDecoder, eventSchema } from 'uiharu'
import { i2cEvents } from '../contracts/i2c'
type BusItem = ItemOf<typeof i2cEvents>
export type Chip =
  | 'generic'
  | '24c02'
  | '24c04'
  | '24c08'
  | '24c16'
  | '24c32'
  | '24c64'
  | '24c128'
  | '24c256'
  | '24c512'
  | '34aa04'
export interface Config {
  chip: Chip
  address: u16
  capacity: u32
  pageBytes: u16
  addressBytes: u8
  blockBits: u8
  initialCounter: u32 | undefined
  initialBank: u8 | undefined
}
export type End = 'complete' | 'incomplete' | 'aborted' | 'nack'
export type Event =
  | { kind: 'begin'; op: u64; tx: u64; read: bool; ack: bool; device: u16 }
  | { kind: 'address'; op: u64; value: u32 | undefined; ack: bool }
  | {
      kind: 'data'
      op: u64
      address: u32 | undefined
      value: u8
      read: bool
      ack: bool
      valid: bool
    }
  | { kind: 'bank'; op: u64; bank: u8; readback: bool; valid: bool }
  | { kind: 'end'; op: u64; reason: End; writeRequested: bool }
export const events = eventSchema<Event>('eeprom24xx-memory.events', 1).roles({
  begin: 'command',
  address: 'address',
  data: { role: 'data', errorWhen: { valid: false } },
  bank: 'control',
  end: 'framing',
})
function capacity(c: Config): u32 {
  switch (c.chip) {
    case '24c02':
      return 256
    case '24c04':
    case '34aa04':
      return 512
    case '24c08':
      return 1024
    case '24c16':
      return 2048
    case '24c32':
      return 4096
    case '24c64':
      return 8192
    case '24c128':
      return 16384
    case '24c256':
      return 32768
    case '24c512':
      return 65536
    default:
      return c.capacity
  }
}
function page(c: Config): u16 {
  switch (c.chip) {
    case '24c02':
      return 8
    case '24c04':
    case '24c08':
    case '24c16':
    case '34aa04':
      return 16
    case '24c32':
    case '24c64':
      return 32
    case '24c128':
    case '24c256':
      return 64
    case '24c512':
      return 128
    default:
      return c.pageBytes
  }
}
function addressBytes(c: Config): u8 {
  return c.chip === 'generic' ? c.addressBytes : capacity(c) > 2048 ? 2 : 1
}
function blockBits(c: Config): u8 {
  return c.chip === 'generic'
    ? c.blockBits
    : c.chip === '24c04'
      ? 1
      : c.chip === '24c08'
        ? 2
        : c.chip === '24c16'
          ? 3
          : 0
}
function validate(c: Config): string | undefined {
  if (c.address < 0x50 || c.address > 0x57) return 'eeprom24xx-memory.config.address'
  if (addressBytes(c) < 1 || addressBytes(c) > 2 || blockBits(c) > 3)
    return 'eeprom24xx-memory.config.addressing'
  if (capacity(c) === 0 || capacity(c) > 524288 || page(c) === 0 || page(c) > capacity(c))
    return 'eeprom24xx-memory.config.geometry'
  if (
    (capacity(c) & (capacity(c) - 1)) !== 0 ||
    (page(c) & (page(c) - 1)) !== 0 ||
    capacity(c) % page(c) !== 0
  )
    return 'eeprom24xx-memory.config.geometry'
  if (
    capacity(c) > 1 << (addressBytes(c) * 8 + blockBits(c) + (c.chip === '34aa04' ? 1 : 0)) ||
    (c.address & ((1 << blockBits(c)) - 1)) !== 0
  )
    return 'eeprom24xx-memory.config.addressing'
  if (c.initialCounter !== undefined && c.initialCounter >= capacity(c))
    return 'eeprom24xx-memory.config.counter'
  if (c.initialBank !== undefined && c.initialBank > 1) return 'eeprom24xx-memory.config.bank'
  return undefined
}
class Device extends BusDecoder<Config, BusItem, Event, End> {
  counter: u32 | undefined = this.config.initialCounter
  bank: u8 | undefined = this.config.initialBank
  open = false
  next: u64 = 1
  op: u64 = 0
  read = false
  addressLeft: u8 = 0
  addressSeen: u8 = 0
  accumulated: u32 = 0
  blockBase: u32 = 0
  pageBase: u32 = 0
  writeCount: u32 = 0
  invalid = false
  special: u8 = 0
  dummy: u8 = 0
  pendingBank: u8 = 0
  override onGap(): void {
    this.counter = undefined
    this.bank = undefined
  }
  async close(reason: End, at: Provenance): Promise<void> {
    if (!this.open) return
    let ending = reason
    if (
      ending === 'complete' &&
      ((this.addressLeft !== 0 && this.addressSeen !== 0) ||
        (this.special === 1 && this.dummy !== 2) ||
        (this.special === 2 && this.dummy === 0))
    )
      ending = 'incomplete'
    if (this.special === 1) {
      if (ending === 'complete' && !this.invalid) this.bank = this.pendingBank
      else this.bank = undefined
    }
    if (this.special === 1)
      await this.out.emit(
        {
          kind: 'bank',
          op: this.op,
          bank: this.pendingBank,
          readback: false,
          valid: ending === 'complete' && !this.invalid,
        },
        at,
      )
    if (ending !== 'complete' && this.writeCount !== 0) this.counter = undefined
    await this.out.emit(
      {
        kind: 'end',
        op: this.op,
        reason: ending,
        writeRequested:
          ending === 'complete' && !this.read && this.writeCount !== 0 && !this.invalid,
      },
      at,
    )
    this.open = false
  }
  override async recover(reason: End, at: Provenance): Promise<void> {
    this.counter = undefined
    await this.close(reason, at)
  }
  override async transaction(tx: Transaction<BusItem>): Promise<void> {
    let active = false
    for (;;) {
      const item = await tx.itemOrEnd()
      if (item === undefined) {
        await this.close('complete', this.consumed())
        return
      }
      const at = this.consumed()
      switch (item.kind) {
        case 'address': {
          await this.close('incomplete', at)
          active = false
          const bits = blockBits(this.config)
          const blockMask: u16 = (1 << bits) - 1
          const normal =
            !item.tenBit && (item.address & ~blockMask) === (this.config.address & ~blockMask)
          const special =
            this.config.chip === '34aa04' &&
            !item.tenBit &&
            (item.address === 0x36 || (!item.read && item.address === 0x37))
          if (!normal && !special) continue
          this.op = this.next
          this.next += 1
          this.open = true
          this.read = item.read
          this.writeCount = 0
          this.invalid = false
          this.special = 0
          this.dummy = 0
          this.addressLeft = 0
          this.addressSeen = 0
          this.accumulated = 0
          await this.out.emit(
            {
              kind: 'begin',
              op: this.op,
              tx: tx.id,
              read: item.read,
              ack: item.ack,
              device: item.address,
            },
            at,
          )
          if (special) {
            active = true
            if (item.read) {
              this.special = 2
              this.bank = item.ack ? 0 : 1
              await this.out.emit(
                { kind: 'bank', op: this.op, bank: item.ack ? 0 : 1, readback: true, valid: true },
                at,
              )
            } else {
              this.special = 1
              this.pendingBank = item.address === 0x37 ? 1 : 0
              if (!item.ack) {
                this.invalid = true
                active = false
              }
            }
            continue
          }
          active = item.ack
          if (!item.ack) {
            await this.close('nack', at)
            continue
          }
          if (!item.read) {
            this.addressLeft = addressBytes(this.config)
            this.blockBase =
              ((item.address & blockMask) as u32) * (1 << (addressBytes(this.config) * 8))
            this.accumulated = 0
          } else if (bits !== 0 && this.counter !== undefined)
            this.counter =
              (((item.address & blockMask) as u32) * (1 << (addressBytes(this.config) * 8)) +
                (this.counter & ((1 << (addressBytes(this.config) * 8)) - 1))) %
              capacity(this.config)
          continue
        }
        case 'restart':
          await this.close(
            this.writeCount !== 0 || this.special === 1 ? 'incomplete' : 'complete',
            at,
          )
          active = false
          if (item.droppedBits !== 0) this.counter = undefined
          continue
        case 'data': {
          if (!active || !this.open) continue
          if (this.special !== 0) {
            if (this.dummy < 3) this.dummy += 1
            if (this.special === 1 && item.ack) this.invalid = true
            continue
          }
          if (this.addressLeft !== 0) {
            if (!item.ack) {
              this.counter = undefined
              this.invalid = true
              await this.close('nack', at)
              active = false
              continue
            }
            if (addressBytes(this.config) === 2)
              this.accumulated = (this.accumulated << 8) | item.byte
            else this.accumulated += item.byte
            this.addressSeen += 1
            this.addressLeft -= 1
            if (this.addressLeft === 0) {
              if (this.config.chip === '34aa04') {
                const bank = this.bank
                this.counter =
                  bank === undefined ? undefined : (bank as u32) * 256 + (this.accumulated & 255)
              } else this.counter = (this.blockBase + this.accumulated) % capacity(this.config)
              const counter = this.counter
              if (counter !== undefined) this.pageBase = counter - (counter % page(this.config))
              await this.out.emit(
                { kind: 'address', op: this.op, value: this.counter, ack: true },
                at,
              )
            }
            continue
          }
          const counter = this.counter
          await this.out.emit(
            {
              kind: 'data',
              op: this.op,
              address: counter,
              value: item.byte,
              read: this.read,
              ack: item.ack,
              valid: this.read || item.ack,
            },
            at,
          )
          if (!this.read && !item.ack) {
            this.invalid = true
            await this.close('nack', at)
            active = false
            continue
          }
          if (counter !== undefined) {
            if (this.read)
              this.counter =
                this.config.chip === '34aa04'
                  ? (counter & 256) | ((counter + 1) & 255)
                  : (counter + 1) % capacity(this.config)
            else this.counter = this.pageBase + ((counter + 1) % page(this.config))
          }
          if (!this.read) this.writeCount = 1
          // A read NACK ends data normally, but later bus bytes are not attributed to this device.
          if (this.read && !item.ack) {
            await this.close('complete', at)
            active = false
          }
          continue
        }
      }
    }
  }
}
export const device = defineBusDecoder({
  name: 'eeprom24xx-memory',
  category: 'device',
  config: {
    defaults: {
      chip: '24c256',
      address: 0x50,
      capacity: 32768,
      pageBytes: 64,
      addressBytes: 2,
      blockBits: 0,
      initialCounter: undefined,
      initialBank: undefined,
    },
    validate,
  },
  input: i2cEvents,
  requires: () => ({ bus: 'i2c', framing: 'start-stop' }),
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
      name: '24xx EEPROM memory',
      summary: 'Chip geometry, memory addresses, page wrapping and 34AA04 bank selection',
      fields: {
        chip: 'Chip profile',
        address: 'I²C base address',
        capacity: 'Generic capacity (bytes)',
        pageBytes: 'Generic page bytes',
        addressBytes: 'Generic address bytes',
        blockBits: 'Generic control block bits',
        initialCounter: 'Initial counter (optional)',
        initialBank: 'Initial 34AA04 bank (optional)',
      },
      values: { chip: { generic: 'Generic geometry' } },
      events: {
        begin: 'Begin',
        address: 'Memory address',
        data: 'Data byte',
        bank: 'Bank selection',
        end: 'End',
      },
      eventFields: {
        op: 'Operation',
        tx: 'I²C transaction',
        read: 'Read',
        ack: 'Acknowledged',
        device: 'Device',
        value: 'Value',
        address: 'Memory address',
        valid: 'Valid',
        bank: 'Bank',
        readback: 'Readback',
        reason: 'End reason',
        writeRequested: 'Write ended with STOP',
      },
    },
    'zh-CN': {
      name: '24xx EEPROM存储器',
      summary: '型号几何参数、存储地址、页内回绕及34AA04存储区选择',
      fields: {
        chip: '型号',
        address: 'I²C基地址',
        capacity: '通用容量（字节）',
        pageBytes: '通用页字节数',
        addressBytes: '通用地址字节数',
        blockBits: '通用控制块位数',
        initialCounter: '初始地址计数器（可选）',
        initialBank: '初始34AA04存储区（可选）',
      },
      values: { chip: { generic: '通用几何参数' } },
      events: {
        begin: '开始',
        address: '存储地址',
        data: '数据字节',
        bank: '存储区选择',
        end: '结束',
      },
      eventFields: {
        op: '操作号',
        tx: 'I²C事务',
        read: '读取',
        ack: '应答',
        device: '设备',
        value: '数值',
        address: '存储地址',
        valid: '有效',
        bank: '存储区',
        readback: '读回',
        reason: '结束原因',
        writeRequested: '写入以STOP结束',
      },
    },
  },
  decoder: Device,
})
