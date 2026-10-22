// SPDX-License-Identifier: Apache-2.0
/* eslint-disable @typescript-eslint/no-empty-object-type -- DSL integer widths and empty configuration records carry IR meaning. */
/** CC1101 headers, status/strobes, register bursts and FIFO bytes; RF packet/state interpretation is outside this transport.
 * Public manufacturer specification: https://www.ti.com/lit/gpn/cc1101 .
 * Each transfer is bounded state and streaming data. Partial CS frames are skipped;
 * missing required data closes incomplete/aborted rather than inventing bytes. */
import type { bool, Provenance, Transaction, u8, u16, u32, u64 } from 'uiharu'
import { BusDecoder, defineBusDecoder, eventSchema } from 'uiharu'
import { spiEvents, type SpiWord } from './spi'
export type RegisterName =
  | 'IOCFG2'
  | 'IOCFG1'
  | 'IOCFG0'
  | 'FIFOTHR'
  | 'SYNC1'
  | 'SYNC0'
  | 'PKTLEN'
  | 'PKTCTRL1'
  | 'PKTCTRL0'
  | 'ADDR'
  | 'CHANNR'
  | 'FSCTRL1'
  | 'FSCTRL0'
  | 'FREQ2'
  | 'FREQ1'
  | 'FREQ0'
  | 'MDMCFG4'
  | 'MDMCFG3'
  | 'MDMCFG2'
  | 'MDMCFG1'
  | 'MDMCFG0'
  | 'DEVIATN'
  | 'MCSM2'
  | 'MCSM1'
  | 'MCSM0'
  | 'FOCCFG'
  | 'BSCFG'
  | 'AGCTRL2'
  | 'AGCTRL1'
  | 'AGCTRL0'
  | 'WOREVT1'
  | 'WOREVT0'
  | 'WORCTRL'
  | 'FREND1'
  | 'FREND0'
  | 'FSCAL3'
  | 'FSCAL2'
  | 'FSCAL1'
  | 'FSCAL0'
  | 'RCCTRL1'
  | 'RCCTRL0'
  | 'FSTEST'
  | 'PTEST'
  | 'AGCTEST'
  | 'TEST2'
  | 'TEST1'
  | 'TEST0'
  | 'PARTNUM'
  | 'VERSION'
  | 'FREQEST'
  | 'LQI'
  | 'RSSI'
  | 'MARCSTATE'
  | 'WORTIME1'
  | 'WORTIME0'
  | 'PKTSTATUS'
  | 'VCO_VC_DAC'
  | 'TXBYTES'
  | 'RXBYTES'
  | 'RCCTRL1_STATUS'
  | 'RCCTRL0_STATUS'
  | 'PATABLE'
  | 'FIFO'
  | 'unknown'
function registerName(address: u16): RegisterName {
  if (address === 0) return 'IOCFG2'
  if (address === 1) return 'IOCFG1'
  if (address === 2) return 'IOCFG0'
  if (address === 3) return 'FIFOTHR'
  if (address === 4) return 'SYNC1'
  if (address === 5) return 'SYNC0'
  if (address === 6) return 'PKTLEN'
  if (address === 7) return 'PKTCTRL1'
  if (address === 8) return 'PKTCTRL0'
  if (address === 9) return 'ADDR'
  if (address === 10) return 'CHANNR'
  if (address === 11) return 'FSCTRL1'
  if (address === 12) return 'FSCTRL0'
  if (address === 13) return 'FREQ2'
  if (address === 14) return 'FREQ1'
  if (address === 15) return 'FREQ0'
  if (address === 16) return 'MDMCFG4'
  if (address === 17) return 'MDMCFG3'
  if (address === 18) return 'MDMCFG2'
  if (address === 19) return 'MDMCFG1'
  if (address === 20) return 'MDMCFG0'
  if (address === 21) return 'DEVIATN'
  if (address === 22) return 'MCSM2'
  if (address === 23) return 'MCSM1'
  if (address === 24) return 'MCSM0'
  if (address === 25) return 'FOCCFG'
  if (address === 26) return 'BSCFG'
  if (address === 27) return 'AGCTRL2'
  if (address === 28) return 'AGCTRL1'
  if (address === 29) return 'AGCTRL0'
  if (address === 30) return 'WOREVT1'
  if (address === 31) return 'WOREVT0'
  if (address === 32) return 'WORCTRL'
  if (address === 33) return 'FREND1'
  if (address === 34) return 'FREND0'
  if (address === 35) return 'FSCAL3'
  if (address === 36) return 'FSCAL2'
  if (address === 37) return 'FSCAL1'
  if (address === 38) return 'FSCAL0'
  if (address === 39) return 'RCCTRL1'
  if (address === 40) return 'RCCTRL0'
  if (address === 41) return 'FSTEST'
  if (address === 42) return 'PTEST'
  if (address === 43) return 'AGCTEST'
  if (address === 44) return 'TEST2'
  if (address === 45) return 'TEST1'
  if (address === 46) return 'TEST0'
  if (address === 48) return 'PARTNUM'
  if (address === 49) return 'VERSION'
  if (address === 50) return 'FREQEST'
  if (address === 51) return 'LQI'
  if (address === 52) return 'RSSI'
  if (address === 53) return 'MARCSTATE'
  if (address === 54) return 'WORTIME1'
  if (address === 55) return 'WORTIME0'
  if (address === 56) return 'PKTSTATUS'
  if (address === 57) return 'VCO_VC_DAC'
  if (address === 58) return 'TXBYTES'
  if (address === 59) return 'RXBYTES'
  if (address === 60) return 'RCCTRL1_STATUS'
  if (address === 61) return 'RCCTRL0_STATUS'
  if (address === 62) return 'PATABLE'
  if (address === 63) return 'FIFO'
  return 'unknown'
}
export type Operation =
  | 'frequency-synth-on'
  | 'crystal-off'
  | 'wake-on-radio'
  | 'reset-wor-timer'
  | 'unknown'
  | 'reset'
  | 'calibrate'
  | 'receive'
  | 'transmit'
  | 'idle'
  | 'power-down'
  | 'flush-rx'
  | 'flush-tx'
  | 'no-op'
  | 'read-register'
  | 'write-register'
export type Field =
  | 'packet-length'
  | 'device-address'
  | 'channel'
  | 'frequency-msb'
  | 'frequency-middle'
  | 'frequency-lsb'
  | 'rssi'
  | 'pa-table'
  | 'fifo'
  | 'crc-enabled'
  | 'whitening'
  | 'packet-length-mode'
  | 'packet-format'
  | 'modulation'
  | 'manchester'
  | 'sync-mode'
  | 'preamble-code'
  | 'link-quality'
  | 'crc-ok'
  | 'marc-state'
  | 'tx-fifo-bytes'
  | 'tx-underflow'
  | 'rx-fifo-bytes'
  | 'rx-overflow'
export type End = 'complete' | 'incomplete' | 'aborted' | 'unsupported'
export type Event =
  | { kind: 'begin'; tx: u64 }
  | { kind: 'command'; tx: u64; opcode: u8; address: u16; read: bool; status: u8 | undefined }
  | { kind: 'register'; tx: u64; address: u16; register: RegisterName; value: u8; read: bool }
  | { kind: 'data'; tx: u64; offset: u32; address: u16; value: u8; read: bool }
  | { kind: 'operation'; tx: u64; operation: Operation }
  | { kind: 'field'; tx: u64; address: u16; field: Field; value: u32 }
  | { kind: 'end'; tx: u64; reason: End }
export const events = eventSchema<Event>('cc1101.events', 1).roles({
  begin: 'framing',
  command: 'command',
  data: 'data',
  register: 'data',
  operation: 'command',
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
  override async recover(reason: End, where: Provenance): Promise<void> {
    if (this.open !== undefined) {
      const tx = this.open
      this.open = undefined
      await this.out.emit({ kind: 'end', tx, reason }, where)
    }
  }
  layout(opcode: u8): Layout {
    const address: u16 = opcode & 63
    const read = (opcode & 128) !== 0
    const burst = (opcode & 64) !== 0
    if (address >= 0x30 && address <= 0x3d) {
      if (!burst) return { address, read, step: 0, count: 0, dummy: 0, valid: address !== 0x37 }
      return { address, read, step: 0, count: 1, dummy: 0, valid: read }
    }
    return {
      address,
      read,
      step: burst && address < 0x3e ? 1 : 0,
      count: burst ? undefined : 1,
      dummy: 0,
      valid: address !== 0x2f,
    }
  }
  operation(opcode: u8, read: bool): Operation {
    const address: u8 = opcode & 63
    if ((opcode & 64) === 0 && address >= 0x30 && address <= 0x3d) {
      if (address === 48) return 'reset'
      if (address === 49) return 'frequency-synth-on'
      if (address === 50) return 'crystal-off'
      if (address === 51) return 'calibrate'
      if (address === 52) return 'receive'
      if (address === 53) return 'transmit'
      if (address === 54) return 'idle'
      if (address === 56) return 'wake-on-radio'
      if (address === 57) return 'power-down'
      if (address === 58) return 'flush-rx'
      if (address === 59) return 'flush-tx'
      if (address === 60) return 'reset-wor-timer'
      if (address === 61) return 'no-op'
      return 'unknown'
    }
    return read ? 'read-register' : 'write-register'
  }
  async interpret(tx: u64, address: u16, value: u8, where: Provenance): Promise<void> {
    if (address === 6)
      await this.out.emit({ kind: 'field', tx, address, field: 'packet-length', value }, where)
    if (address === 9)
      await this.out.emit({ kind: 'field', tx, address, field: 'device-address', value }, where)
    if (address === 10)
      await this.out.emit({ kind: 'field', tx, address, field: 'channel', value }, where)
    if (address === 13)
      await this.out.emit({ kind: 'field', tx, address, field: 'frequency-msb', value }, where)
    if (address === 14)
      await this.out.emit({ kind: 'field', tx, address, field: 'frequency-middle', value }, where)
    if (address === 15)
      await this.out.emit({ kind: 'field', tx, address, field: 'frequency-lsb', value }, where)
    if (address === 52)
      await this.out.emit({ kind: 'field', tx, address, field: 'rssi', value }, where)
    if (address === 62)
      await this.out.emit({ kind: 'field', tx, address, field: 'pa-table', value }, where)
    if (address === 63)
      await this.out.emit({ kind: 'field', tx, address, field: 'fifo', value }, where)
    if (address === 8) {
      await this.out.emit(
        { kind: 'field', tx, address, field: 'crc-enabled', value: (value >> 2) & 1 },
        where,
      )
      await this.out.emit(
        { kind: 'field', tx, address, field: 'whitening', value: (value >> 6) & 1 },
        where,
      )
      await this.out.emit(
        { kind: 'field', tx, address, field: 'packet-length-mode', value: value & 3 },
        where,
      )
      await this.out.emit(
        { kind: 'field', tx, address, field: 'packet-format', value: (value >> 4) & 3 },
        where,
      )
    }
    if (address === 18) {
      await this.out.emit(
        { kind: 'field', tx, address, field: 'modulation', value: (value >> 4) & 7 },
        where,
      )
      await this.out.emit(
        { kind: 'field', tx, address, field: 'manchester', value: (value >> 3) & 1 },
        where,
      )
      await this.out.emit(
        { kind: 'field', tx, address, field: 'sync-mode', value: value & 7 },
        where,
      )
    }
    if (address === 19) {
      await this.out.emit(
        { kind: 'field', tx, address, field: 'preamble-code', value: (value >> 4) & 7 },
        where,
      )
    }
    if (address === 51) {
      await this.out.emit(
        { kind: 'field', tx, address, field: 'link-quality', value: value & 127 },
        where,
      )
      await this.out.emit({ kind: 'field', tx, address, field: 'crc-ok', value: value >> 7 }, where)
    }
    if (address === 53) {
      await this.out.emit(
        { kind: 'field', tx, address, field: 'marc-state', value: value & 31 },
        where,
      )
    }
    if (address === 58) {
      await this.out.emit(
        { kind: 'field', tx, address, field: 'tx-fifo-bytes', value: value & 127 },
        where,
      )
      await this.out.emit(
        { kind: 'field', tx, address, field: 'tx-underflow', value: value >> 7 },
        where,
      )
    }
    if (address === 59) {
      await this.out.emit(
        { kind: 'field', tx, address, field: 'rx-fifo-bytes', value: value & 127 },
        where,
      )
      await this.out.emit(
        { kind: 'field', tx, address, field: 'rx-overflow', value: value >> 7 },
        where,
      )
    }
  }
  override async transaction(tx: Transaction<SpiWord>): Promise<void> {
    this.open = tx.id
    await this.out.emit({ kind: 'begin', tx: tx.id })
    let offset: u32 = 0
    for (;;) {
      const first = await tx.itemOrEnd()
      if (first === undefined) break
      const opcode = first.mosi!
      const layout = this.layout(opcode)
      const commandAt = this.consumed()
      await this.out.emit(
        {
          kind: 'command',
          tx: tx.id,
          opcode,
          address: layout.address,
          read: layout.read,
          status: first.miso,
        },
        commandAt,
      )
      await this.out.emit(
        { kind: 'operation', tx: tx.id, operation: this.operation(opcode, layout.read) },
        commandAt,
      )
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
        if (layout.address < 0x30 && address > 0x2e) {
          await tx.drain()
          this.open = undefined
          await this.out.emit({ kind: 'end', tx: tx.id, reason: 'unsupported' })
          return
        }
        const name: RegisterName =
          layout.address < 0x30 && address > 0x2e ? 'unknown' : registerName(address)
        await this.out.emit(
          { kind: 'register', tx: tx.id, address, register: name, value, read: layout.read },
          where,
        )
        await this.interpret(tx.id, address, value, where)
        count += 1
        offset += 1
        address += layout.step
      }
    }
    this.open = undefined
    await this.out.emit({ kind: 'end', tx: tx.id, reason: 'complete' })
  }
}
export const device = defineBusDecoder({
  name: 'cc1101',
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
      name: 'CC1101',
      summary: 'CC1101 strobes, FIFO traffic and packet/modem/status fields',
      fields: {},
      events: {
        begin: 'Begin',
        command: 'Command',
        data: 'Data',
        register: 'Register',
        operation: 'Operation',
        field: 'Field',
        end: 'End',
      },
      eventFields: {
        tx: 'Transaction',
        opcode: 'Opcode',
        address: 'Address',
        register: 'Register name',
        read: 'Read',
        status: 'Status',
        offset: 'Offset',
        value: 'Value',
        operation: 'Operation',
        field: 'Field',
        reason: 'Reason',
      },
      values: {},
    },
    'zh-CN': {
      name: 'CC1101',
      summary: '解析CC1101选通命令、FIFO数据及数据包、调制和状态字段',
      fields: {},
      events: {
        begin: '开始',
        command: '命令',
        data: '数据通道',
        register: '寄存器',
        operation: '操作',
        field: '字段',
        end: '结束',
      },
      eventFields: {
        tx: '事务号',
        opcode: '操作码',
        address: '地址',
        register: '寄存器名称',
        read: '读取',
        status: '状态',
        offset: '偏移',
        value: '数值',
        operation: '操作',
        field: '字段',
        reason: '结束原因',
      },
      values: {},
    },
  },
  decoder: Device,
})
