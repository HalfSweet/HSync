// SPDX-License-Identifier: Apache-2.0
/** Panchip original XN297 2012 and explicit XN297L V4.6 models. XN297L primary:
 * https://datasheet.lcsc.com/datasheet/pdf/45bf1129bd364889783cdc4ddfb5f751.pdf . Original datasheet §§7.3/7.6 tables6/8:
 * https://deviationtx.com/media/kunena/attachments/1511/xn297datasheet.pdf .
 * SPI commands, 64-byte payload bound, 5/7/5-byte DEMOD/RF/BB calibration
 * registers and common digital register/status fields. No over-air scrambling,
 * analog calibration meaning or unsupported clone register banks are inferred. */
import type { bool, Provenance, Transaction, u8, u32, u64 } from 'uiharu'
import { BusDecoder, defineBusDecoder, eventSchema } from 'uiharu'
import { spiEvents, type SpiWord } from './spi'
export type Command =
  | 'read-register'
  | 'write-register'
  | 'read-rx-payload'
  | 'write-tx-payload'
  | 'read-payload-width'
  | 'write-ack-payload'
  | 'write-noack-payload'
  | 'flush-tx'
  | 'flush-rx'
  | 'reuse-tx'
  | 'activate'
  | 'nop'
  | 'unknown'
export type Register =
  | 'config'
  | 'auto-ack'
  | 'rx-enable'
  | 'address-width'
  | 'retry'
  | 'channel'
  | 'rf-setup'
  | 'status'
  | 'observe-tx'
  | 'rssi'
  | 'rx-address'
  | 'tx-address'
  | 'rx-payload-width'
  | 'fifo-status'
  | 'demod-cal'
  | 'dynamic-payload'
  | 'feature'
  | 'rf-cal'
  | 'bb-cal'
  | 'rf-cal2'
  | 'dem-cal2'
  | 'unknown'
export type Field =
  | 'rx-ready'
  | 'tx-sent'
  | 'max-retries'
  | 'rx-pipe'
  | 'tx-full'
  | 'power-up'
  | 'receive-mode'
  | 'crc-enabled'
  | 'irq-mask'
  | 'auto-ack-mask'
  | 'rx-pipe-mask'
  | 'address-bytes'
  | 'retry-delay-code'
  | 'retry-count'
  | 'channel'
  | 'tx-power-code'
  | 'data-rate-code'
  | 'packet-loss'
  | 'retry-observed'
  | 'fifo-status'
  | 'dynamic-payload-mask'
  | 'payload-length-mode'
  | 'dynamic-payload'
  | 'ack-payload'
  | 'dynamic-ack'
export type End = 'complete' | 'incomplete' | 'aborted' | 'too-long' | 'unsupported' | 'invalid-key'
export type Event =
  | {
      kind: 'command'
      tx: u64
      command: Command
      register: Register
      address: u8 | undefined
      pipe: u8 | undefined
      status: u8
    }
  | { kind: 'data'; tx: u64; offset: u8; value: u8; read: bool }
  | { kind: 'field'; tx: u64; field: Field; value: u32 }
  | { kind: 'end'; tx: u64; reason: End }
export const events = eventSchema<Event>('xn297.events', 1).roles({
  command: 'command',
  data: 'data',
  field: 'data',
  end: 'framing',
})
export type Model = 'original' | 'xn297l'
export interface Config {
  model: Model
}
class Xn extends BusDecoder<Config, SpiWord, Event, End> {
  open: u64 | undefined = undefined
  override async recover(reason: End, where: Provenance): Promise<void> {
    if (this.open !== undefined) {
      const tx = this.open
      this.open = undefined
      await this.out.emit({ kind: 'end', tx, reason }, where)
    }
  }
  name(address: u8): Register {
    if (this.config.model === 'xn297l') {
      if (address === 26) return 'rf-cal2'
      if (address === 27) return 'dem-cal2'
    }
    if (address === 0) return 'config'
    if (address === 1) return 'auto-ack'
    if (address === 2) return 'rx-enable'
    if (address === 3) return 'address-width'
    if (address === 4) return 'retry'
    if (address === 5) return 'channel'
    if (address === 6) return 'rf-setup'
    if (address === 7) return 'status'
    if (address === 8) return 'observe-tx'
    if (address === 9) return 'rssi'
    if (address >= 10 && address <= 15) return 'rx-address'
    if (address === 16) return 'tx-address'
    if (address >= 17 && address <= 22) return 'rx-payload-width'
    if (address === 23) return 'fifo-status'
    if (address === 25) return 'demod-cal'
    if (address === 28) return 'dynamic-payload'
    if (address === 29) return 'feature'
    if (address === 30) return 'rf-cal'
    if (address === 31) return 'bb-cal'
    return 'unknown'
  }
  width(address: u8): u8 {
    if (this.config.model === 'xn297l') {
      if (address === 26) return 6
      if (address === 27 || address === 30) return 3
      if (address === 25 || address === 31) return 1
    }
    if (address === 30) return 7
    if (address === 25 || address === 31 || address === 10 || address === 11 || address === 16)
      return 5
    return 1
  }
  async field(tx: u64, field: Field, value: u32, where: Provenance): Promise<void> {
    await this.out.emit({ kind: 'field', tx, field, value }, where)
  }
  async status(tx: u64, value: u8, where: Provenance): Promise<void> {
    await this.field(tx, 'rx-ready', (value >> 6) & 1, where)
    await this.field(tx, 'tx-sent', (value >> 5) & 1, where)
    await this.field(tx, 'max-retries', (value >> 4) & 1, where)
    await this.field(tx, 'rx-pipe', (value >> 1) & 7, where)
    await this.field(tx, 'tx-full', value & 1, where)
  }
  async interpret(tx: u64, address: u8, value: u8, where: Provenance): Promise<void> {
    if (address === 0) {
      await this.field(tx, 'power-up', (value >> 1) & 1, where)
      await this.field(tx, 'receive-mode', value & 1, where)
      await this.field(tx, 'crc-enabled', (value >> 3) & 1, where)
      await this.field(tx, 'irq-mask', (value >> 4) & 7, where)
    } else if (address === 1) await this.field(tx, 'auto-ack-mask', value & 63, where)
    else if (address === 2) await this.field(tx, 'rx-pipe-mask', value & 63, where)
    else if (address === 3) await this.field(tx, 'address-bytes', (value & 3) + 2, where)
    else if (address === 4) {
      await this.field(tx, 'retry-delay-code', value >> 4, where)
      await this.field(tx, 'retry-count', value & 15, where)
    } else if (address === 5) await this.field(tx, 'channel', value & 127, where)
    else if (address === 6) {
      await this.field(tx, 'tx-power-code', (value >> 1) & 3, where)
      await this.field(tx, 'data-rate-code', (value >> 3) & 1, where)
    } else if (address === 7) await this.status(tx, value, where)
    else if (address === 8) {
      await this.field(tx, 'packet-loss', value >> 4, where)
      await this.field(tx, 'retry-observed', value & 15, where)
    } else if (address === 23) await this.field(tx, 'fifo-status', value, where)
    else if (address === 28) await this.field(tx, 'dynamic-payload-mask', value & 63, where)
    else if (address === 29) {
      await this.field(tx, 'payload-length-mode', (value >> 3) & 3, where)
      await this.field(tx, 'dynamic-payload', (value >> 2) & 1, where)
      await this.field(tx, 'ack-payload', (value >> 1) & 1, where)
      await this.field(tx, 'dynamic-ack', value & 1, where)
    }
  }
  override async transaction(tx: Transaction<SpiWord>): Promise<void> {
    const first = await tx.item()
    const opcode = first.mosi!
    this.open = tx.id
    let command: Command = 'unknown'
    let address: u8 | undefined = undefined
    let pipe: u8 | undefined = undefined
    let register: Register = 'unknown'
    let read = false
    let maximum: u8 = 0
    let minimum: u8 = 0
    if (opcode < 64) {
      const registerAddress: u8 = opcode & 31
      address = registerAddress
      register = this.name(registerAddress)
      read = opcode < 32
      command = read ? 'read-register' : 'write-register'
      minimum = 1
      maximum = this.width(registerAddress)
    } else if (opcode === 0x61) {
      command = 'read-rx-payload'
      read = true
      minimum = 1
      maximum = 64
    } else if (opcode === 0xa0 || opcode === 0xb0 || (opcode >= 0xa8 && opcode <= 0xad)) {
      command =
        opcode === 0xa0
          ? 'write-tx-payload'
          : opcode === 0xb0
            ? 'write-noack-payload'
            : 'write-ack-payload'
      if (opcode >= 0xa8 && opcode <= 0xad) pipe = opcode & 7
      minimum = 1
      maximum = 64
    } else if (opcode === 0x60) {
      command = 'read-payload-width'
      read = true
      minimum = 1
      maximum = 1
    } else if (opcode === 0xe1) command = 'flush-tx'
    else if (opcode === 0xe2) command = 'flush-rx'
    else if (opcode === 0xe3) command = 'reuse-tx'
    else if (opcode === 0xff) command = 'nop'
    else if (opcode === 0x50) {
      command = 'activate'
      minimum = 1
      maximum = 1
    }
    const where = this.consumed()
    await this.out.emit(
      { kind: 'command', tx: tx.id, command, register, address, pipe, status: first.miso! },
      where,
    )
    await this.status(tx.id, first.miso!, where)
    if (command === 'unknown') {
      await tx.drain()
      this.open = undefined
      await this.out.emit({ kind: 'end', tx: tx.id, reason: 'unsupported' })
      return
    }
    let count: u8 = 0
    let reason: End = 'complete'
    for (;;) {
      const word = await tx.itemOrEnd()
      if (word === undefined) break
      if (count >= maximum) {
        reason = 'too-long'
        await tx.drain()
        break
      }
      const value = read ? word.miso! : word.mosi!
      const at = this.consumed()
      await this.out.emit({ kind: 'data', tx: tx.id, offset: count, value, read }, at)
      if (address !== undefined) {
        if (count === 0) await this.interpret(tx.id, address, value, at)
      }
      if (command === 'activate' && value !== 0x73) reason = 'invalid-key'
      count += 1
    }
    if (count < minimum) reason = 'incomplete'
    this.open = undefined
    await this.out.emit({ kind: 'end', tx: tx.id, reason })
  }
}
export const xn297 = defineBusDecoder({
  name: 'xn297',
  category: 'device',
  config: { defaults: { model: 'original' } },
  input: spiEvents,
  requires: () => ({ wordBits: 8, bitOrder: 'msb-first', cs: true, mosi: true, miso: true }),
  output: events,
  props: (config) => ({ chip: config.model, maxPayloadBytes: 64 }),
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
      name: 'XN297',
      summary: 'Panchip XN297 commands, 64-byte payloads and calibration registers',
      fields: { model: 'Model' },
      events: { command: 'Command', data: 'Data', field: 'Field', end: 'End' },
      eventFields: {
        tx: 'Transaction',
        command: 'Command',
        register: 'Register',
        address: 'Address',
        pipe: 'Pipe',
        status: 'Status',
        offset: 'Offset',
        value: 'Value',
        read: 'Read',
        field: 'Field',
        reason: 'Reason',
      },
      values: { model: { original: 'XN297 (original 2012)', xn297l: 'XN297L (V4.6)' } },
    },
    'zh-CN': {
      name: 'XN297',
      summary: '解析磐启XN297命令、64字节载荷及校准寄存器',
      fields: { model: '芯片型号' },
      events: { command: '命令', data: '数据通道', field: '字段', end: '结束' },
      eventFields: {
        tx: '事务号',
        command: '命令',
        register: '寄存器',
        address: '地址',
        pipe: '接收管道',
        status: '状态',
        offset: '偏移',
        value: '数值',
        read: '读取',
        field: '字段',
        reason: '结束原因',
      },
      values: { model: { original: 'XN297（原始2012版）', xn297l: 'XN297L（V4.6）' } },
    },
  },
  decoder: Xn,
})
