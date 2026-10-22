// SPDX-License-Identifier: Apache-2.0
/* eslint-disable @typescript-eslint/no-empty-object-type -- DSL integer widths and empty configuration records carry IR meaning. */
/** ST25R3916/3917 space-A/B and test register transfers, FIFO 80/9F, named status and direct commands; passive-target memory commands are left unsupported.
 * Public manufacturer specification: https://www.st.com/resource/en/datasheet/st25r3916.pdf .
 * Each transfer is bounded state and streaming data. Partial CS frames are skipped;
 * missing required data closes incomplete/aborted rather than inventing bytes. */
import type { bool, Provenance, Transaction, u8, u16, u32, u64 } from 'uiharu'
import { BusDecoder, defineBusDecoder, eventSchema } from 'uiharu'
import { spiEvents, type SpiWord } from './spi'
export type RegisterName =
  | 'io-configuration-1'
  | 'io-configuration-2'
  | 'operation-control'
  | 'mode-definition'
  | 'bit-rate-definition'
  | 'iso14443a-nfc'
  | 'iso14443b-1'
  | 'iso14443b-felica'
  | 'nfcip1-passive-target'
  | 'stream-mode'
  | 'auxiliary-definition'
  | 'receiver-configuration-1'
  | 'receiver-configuration-2'
  | 'receiver-configuration-3'
  | 'receiver-configuration-4'
  | 'mask-receive-timer'
  | 'no-response-timer-1'
  | 'no-response-timer-2'
  | 'timer-emv-control'
  | 'general-purpose-timer-1'
  | 'general-purpose-timer-2'
  | 'ppon2-field-waiting'
  | 'mask-main-interrupt'
  | 'mask-timer-nfc-interrupt'
  | 'mask-error-wake-interrupt'
  | 'mask-passive-target-interrupt'
  | 'main-interrupt'
  | 'timer-nfc-interrupt'
  | 'error-wake-interrupt'
  | 'passive-target-interrupt'
  | 'fifo-status-1'
  | 'fifo-status-2'
  | 'collision-display'
  | 'passive-target-display'
  | 'transmitted-bytes-1'
  | 'transmitted-bytes-2'
  | 'bit-rate-detection'
  | 'adc-output'
  | 'antenna-tuning-control-1'
  | 'antenna-tuning-control-2'
  | 'tx-driver'
  | 'passive-target-modulation'
  | 'external-field-activation-threshold'
  | 'external-field-deactivation-threshold'
  | 'regulator-voltage-control'
  | 'rssi-display'
  | 'gain-reduction-state'
  | 'capacitive-sensor-control'
  | 'capacitive-sensor-display'
  | 'auxiliary-display'
  | 'wake-up-timer-control'
  | 'amplitude-measurement-configuration'
  | 'amplitude-measurement-reference'
  | 'amplitude-measurement-auto-average'
  | 'amplitude-measurement-display'
  | 'phase-measurement-configuration'
  | 'phase-measurement-reference'
  | 'phase-measurement-auto-average'
  | 'phase-measurement-display'
  | 'capacitance-measurement-configuration'
  | 'capacitance-measurement-reference'
  | 'capacitance-measurement-auto-average'
  | 'capacitance-measurement-display'
  | 'ic-identity'
  | 'emd-suppression-configuration'
  | 'subcarrier-start-timer'
  | 'p2p-receiver-configuration-1'
  | 'correlator-configuration-1'
  | 'correlator-configuration-2'
  | 'squelch-timer'
  | 'nfc-field-on-guard-timer'
  | 'auxiliary-modulation-setting'
  | 'tx-driver-timing'
  | 'resistive-am-modulation'
  | 'tx-driver-timing-display'
  | 'regulator-display'
  | 'overshoot-protection-1'
  | 'overshoot-protection-2'
  | 'undershoot-protection-1'
  | 'undershoot-protection-2'
  | 'analog-test-observation-1'
  | 'unknown'
function registerName(space: Space, address: u16): RegisterName {
  if (space === 'a') {
    if (address === 0) return 'io-configuration-1'
    if (address === 1) return 'io-configuration-2'
    if (address === 2) return 'operation-control'
    if (address === 3) return 'mode-definition'
    if (address === 4) return 'bit-rate-definition'
    if (address === 5) return 'iso14443a-nfc'
    if (address === 6) return 'iso14443b-1'
    if (address === 7) return 'iso14443b-felica'
    if (address === 8) return 'nfcip1-passive-target'
    if (address === 9) return 'stream-mode'
    if (address === 10) return 'auxiliary-definition'
    if (address === 11) return 'receiver-configuration-1'
    if (address === 12) return 'receiver-configuration-2'
    if (address === 13) return 'receiver-configuration-3'
    if (address === 14) return 'receiver-configuration-4'
    if (address === 15) return 'mask-receive-timer'
    if (address === 16) return 'no-response-timer-1'
    if (address === 17) return 'no-response-timer-2'
    if (address === 18) return 'timer-emv-control'
    if (address === 19) return 'general-purpose-timer-1'
    if (address === 20) return 'general-purpose-timer-2'
    if (address === 21) return 'ppon2-field-waiting'
    if (address === 22) return 'mask-main-interrupt'
    if (address === 23) return 'mask-timer-nfc-interrupt'
    if (address === 24) return 'mask-error-wake-interrupt'
    if (address === 25) return 'mask-passive-target-interrupt'
    if (address === 26) return 'main-interrupt'
    if (address === 27) return 'timer-nfc-interrupt'
    if (address === 28) return 'error-wake-interrupt'
    if (address === 29) return 'passive-target-interrupt'
    if (address === 30) return 'fifo-status-1'
    if (address === 31) return 'fifo-status-2'
    if (address === 32) return 'collision-display'
    if (address === 33) return 'passive-target-display'
    if (address === 34) return 'transmitted-bytes-1'
    if (address === 35) return 'transmitted-bytes-2'
    if (address === 36) return 'bit-rate-detection'
    if (address === 37) return 'adc-output'
    if (address === 38) return 'antenna-tuning-control-1'
    if (address === 39) return 'antenna-tuning-control-2'
    if (address === 40) return 'tx-driver'
    if (address === 41) return 'passive-target-modulation'
    if (address === 42) return 'external-field-activation-threshold'
    if (address === 43) return 'external-field-deactivation-threshold'
    if (address === 44) return 'regulator-voltage-control'
    if (address === 45) return 'rssi-display'
    if (address === 46) return 'gain-reduction-state'
    if (address === 47) return 'capacitive-sensor-control'
    if (address === 48) return 'capacitive-sensor-display'
    if (address === 49) return 'auxiliary-display'
    if (address === 50) return 'wake-up-timer-control'
    if (address === 51) return 'amplitude-measurement-configuration'
    if (address === 52) return 'amplitude-measurement-reference'
    if (address === 53) return 'amplitude-measurement-auto-average'
    if (address === 54) return 'amplitude-measurement-display'
    if (address === 55) return 'phase-measurement-configuration'
    if (address === 56) return 'phase-measurement-reference'
    if (address === 57) return 'phase-measurement-auto-average'
    if (address === 58) return 'phase-measurement-display'
    if (address === 59) return 'capacitance-measurement-configuration'
    if (address === 60) return 'capacitance-measurement-reference'
    if (address === 61) return 'capacitance-measurement-auto-average'
    if (address === 62) return 'capacitance-measurement-display'
    if (address === 63) return 'ic-identity'
  }
  if (space === 'b') {
    if (address === 5) return 'emd-suppression-configuration'
    if (address === 6) return 'subcarrier-start-timer'
    if (address === 11) return 'p2p-receiver-configuration-1'
    if (address === 12) return 'correlator-configuration-1'
    if (address === 13) return 'correlator-configuration-2'
    if (address === 15) return 'squelch-timer'
    if (address === 21) return 'nfc-field-on-guard-timer'
    if (address === 40) return 'auxiliary-modulation-setting'
    if (address === 41) return 'tx-driver-timing'
    if (address === 42) return 'resistive-am-modulation'
    if (address === 43) return 'tx-driver-timing-display'
    if (address === 44) return 'regulator-display'
    if (address === 48) return 'overshoot-protection-1'
    if (address === 49) return 'overshoot-protection-2'
    if (address === 50) return 'undershoot-protection-1'
    if (address === 51) return 'undershoot-protection-2'
  }
  if (space === 'test' && address === 1) return 'analog-test-observation-1'
  return 'unknown'
}
export type Direct =
  | 'set-default'
  | 'stop'
  | 'transmit-crc'
  | 'transmit-no-crc'
  | 'reqa'
  | 'wupa'
  | 'initial-field-on'
  | 'response-field-on'
  | 'sense'
  | 'sleep'
  | 'mask-rx'
  | 'unmask-rx'
  | 'change-am'
  | 'measure-amplitude'
  | 'reset-rx-gain'
  | 'adjust-regulators'
  | 'calibrate-driver'
  | 'measure-phase'
  | 'clear-rssi'
  | 'clear-fifo'
  | 'transparent'
  | 'calibrate-capacitance'
  | 'measure-capacitance'
  | 'measure-supply'
  | 'start-general-timer'
  | 'start-wake-timer'
  | 'start-mask-timer'
  | 'start-response-timer'
  | 'start-ppon2-timer'
  | 'stop-response-timer'
  | 'unknown'
export type Space = 'a' | 'b' | 'test'
export type Operation =
  'fifo-write' | 'fifo-read' | 'direct-command' | 'read-register' | 'write-register'
export type Field =
  | 'io-configuration-1'
  | 'io-configuration-2'
  | 'operation-control'
  | 'mode'
  | 'bit-rate'
  | 'interrupt-main'
  | 'interrupt-timer-nfc'
  | 'interrupt-error-wake'
  | 'fifo-status-1'
  | 'fifo-status-2'
  | 'identity'
  | 'oscillator-enabled'
  | 'rx-enabled'
  | 'rx-channel'
  | 'tx-enabled'
  | 'wake-enabled'
  | 'fifo-bytes-high'
  | 'fifo-overflow'
  | 'fifo-underflow'
export type End = 'complete' | 'incomplete' | 'aborted' | 'unsupported'
export type Event =
  | { kind: 'begin'; tx: u64 }
  | { kind: 'command'; tx: u64; opcode: u8; address: u16; read: bool; status: u8 | undefined }
  | {
      kind: 'register'
      tx: u64
      space: Space
      address: u16
      register: RegisterName
      value: u8
      read: bool
    }
  | { kind: 'data'; tx: u64; offset: u32; address: u16; value: u8; read: bool }
  | { kind: 'direct'; tx: u64; command: Direct }
  | { kind: 'space'; tx: u64; space: Space }
  | { kind: 'operation'; tx: u64; operation: Operation }
  | { kind: 'field'; tx: u64; address: u16; field: Field; value: u32 }
  | { kind: 'end'; tx: u64; reason: End }
export const events = eventSchema<Event>('st25r39xx-spi.events', 1).roles({
  begin: 'framing',
  command: 'command',
  data: 'data',
  register: 'data',
  direct: 'command',
  space: 'address',
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
  space: Space = 'a'
  override async recover(reason: End, where: Provenance): Promise<void> {
    if (this.open !== undefined) {
      const tx = this.open
      this.open = undefined
      await this.out.emit({ kind: 'end', tx, reason }, where)
    }
  }
  direct(opcode: u8): Direct {
    if (opcode === 192) return 'set-default'
    if (opcode === 193) return 'set-default'
    if (opcode === 194) return 'stop'
    if (opcode === 195) return 'stop'
    if (opcode === 196) return 'transmit-crc'
    if (opcode === 197) return 'transmit-no-crc'
    if (opcode === 198) return 'reqa'
    if (opcode === 199) return 'wupa'
    if (opcode === 200) return 'initial-field-on'
    if (opcode === 201) return 'response-field-on'
    if (opcode === 205) return 'sense'
    if (opcode === 206) return 'sleep'
    if (opcode === 208) return 'mask-rx'
    if (opcode === 209) return 'unmask-rx'
    if (opcode === 210) return 'change-am'
    if (opcode === 211) return 'measure-amplitude'
    if (opcode === 213) return 'reset-rx-gain'
    if (opcode === 214) return 'adjust-regulators'
    if (opcode === 216) return 'calibrate-driver'
    if (opcode === 217) return 'measure-phase'
    if (opcode === 218) return 'clear-rssi'
    if (opcode === 219) return 'clear-fifo'
    if (opcode === 220) return 'transparent'
    if (opcode === 221) return 'calibrate-capacitance'
    if (opcode === 222) return 'measure-capacitance'
    if (opcode === 223) return 'measure-supply'
    if (opcode === 224) return 'start-general-timer'
    if (opcode === 225) return 'start-wake-timer'
    if (opcode === 226) return 'start-mask-timer'
    if (opcode === 227) return 'start-response-timer'
    if (opcode === 228) return 'start-ppon2-timer'
    if (opcode === 232) return 'stop-response-timer'
    return 'unknown'
  }
  layout(opcode: u8): Layout {
    if (opcode >= 0xc0)
      return {
        address: opcode & 63,
        read: false,
        step: 0,
        count: 0,
        dummy: 0,
        valid: this.direct(opcode) !== 'unknown',
      }
    if (opcode === 0x80 || opcode === 0x9f)
      return { address: 0, read: opcode === 0x9f, step: 0, count: undefined, dummy: 0, valid: true }
    return {
      address: opcode & 63,
      read: (opcode & 64) !== 0,
      step: 1,
      count: undefined,
      dummy: 0,
      valid: opcode < 0x80,
    }
  }
  operation(opcode: u8, read: bool): Operation {
    if (opcode === 0x80) return 'fifo-write'
    if (opcode === 0x9f) return 'fifo-read'
    if (opcode >= 0xc0) return 'direct-command'
    if ((opcode & 0xc0) === 0x40) return 'read-register'
    if (opcode < 0x40) return 'write-register'
    return read ? 'read-register' : 'write-register'
  }
  async interpret(tx: u64, address: u16, value: u8, where: Provenance): Promise<void> {
    if (address === 0)
      await this.out.emit({ kind: 'field', tx, address, field: 'io-configuration-1', value }, where)
    if (address === 1)
      await this.out.emit({ kind: 'field', tx, address, field: 'io-configuration-2', value }, where)
    if (address === 2)
      await this.out.emit({ kind: 'field', tx, address, field: 'operation-control', value }, where)
    if (address === 3)
      await this.out.emit({ kind: 'field', tx, address, field: 'mode', value }, where)
    if (address === 4)
      await this.out.emit({ kind: 'field', tx, address, field: 'bit-rate', value }, where)
    if (address === 26)
      await this.out.emit({ kind: 'field', tx, address, field: 'interrupt-main', value }, where)
    if (address === 27)
      await this.out.emit(
        { kind: 'field', tx, address, field: 'interrupt-timer-nfc', value },
        where,
      )
    if (address === 28)
      await this.out.emit(
        { kind: 'field', tx, address, field: 'interrupt-error-wake', value },
        where,
      )
    if (address === 30)
      await this.out.emit({ kind: 'field', tx, address, field: 'fifo-status-1', value }, where)
    if (address === 31)
      await this.out.emit({ kind: 'field', tx, address, field: 'fifo-status-2', value }, where)
    if (address === 63)
      await this.out.emit({ kind: 'field', tx, address, field: 'identity', value }, where)
    if (address === 2) {
      await this.out.emit(
        { kind: 'field', tx, address, field: 'oscillator-enabled', value: value >> 7 },
        where,
      )
      await this.out.emit(
        { kind: 'field', tx, address, field: 'rx-enabled', value: (value >> 6) & 1 },
        where,
      )
      await this.out.emit(
        { kind: 'field', tx, address, field: 'rx-channel', value: (value >> 5) & 1 },
        where,
      )
      await this.out.emit(
        { kind: 'field', tx, address, field: 'tx-enabled', value: (value >> 3) & 1 },
        where,
      )
      await this.out.emit(
        { kind: 'field', tx, address, field: 'wake-enabled', value: (value >> 2) & 1 },
        where,
      )
    }
    if (address === 31) {
      await this.out.emit(
        { kind: 'field', tx, address, field: 'fifo-bytes-high', value: (value >> 6) & 3 },
        where,
      )
      await this.out.emit(
        { kind: 'field', tx, address, field: 'fifo-overflow', value: (value >> 4) & 1 },
        where,
      )
      await this.out.emit(
        { kind: 'field', tx, address, field: 'fifo-underflow', value: (value >> 5) & 1 },
        where,
      )
    }
  }
  override async transaction(tx: Transaction<SpiWord>): Promise<void> {
    this.space = 'a'
    this.open = tx.id
    await this.out.emit({ kind: 'begin', tx: tx.id })
    let offset: u32 = 0
    for (;;) {
      const first = await tx.itemOrEnd()
      if (first === undefined) break
      const opcode = first.mosi!
      if (opcode === 0xfb || opcode === 0xfc) {
        this.space = opcode === 0xfb ? 'b' : 'test'
        await this.out.emit({ kind: 'space', tx: tx.id, space: this.space })
        continue
      }
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
      if (opcode >= 0xc0)
        await this.out.emit({ kind: 'direct', tx: tx.id, command: this.direct(opcode) }, commandAt)
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
        if (opcode < 0x80 && address > 63) {
          await tx.drain()
          this.open = undefined
          await this.out.emit({ kind: 'end', tx: tx.id, reason: 'unsupported' })
          return
        }
        if (opcode < 0x80)
          await this.out.emit(
            {
              kind: 'register',
              tx: tx.id,
              space: this.space,
              address,
              register: registerName(this.space, address),
              value,
              read: layout.read,
            },
            where,
          )
        if (this.space === 'a' && opcode < 0x80) await this.interpret(tx.id, address, value, where)
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
  name: 'st25r39xx-spi',
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
      name: 'ST25R39XX-SPI',
      summary: 'ST25R3916/17 register spaces, FIFO, direct commands and status',
      fields: {},
      events: {
        begin: 'Begin',
        command: 'Command',
        data: 'Data',
        register: 'Register',
        direct: 'Direct',
        space: 'Space',
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
        command: 'Command',
        space: 'Space',
        operation: 'Operation',
        field: 'Field',
        reason: 'Reason',
      },
      values: {},
    },
    'zh-CN': {
      name: 'ST25R39XX-SPI',
      summary: '解析ST25R3916/17寄存器空间、FIFO、直接命令及状态',
      fields: {},
      events: {
        begin: '开始',
        command: '命令',
        data: '数据通道',
        register: '寄存器',
        direct: '直接命令',
        space: '寄存器空间',
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
        command: '命令',
        space: '寄存器空间',
        operation: '操作',
        field: '字段',
        reason: '结束原因',
      },
      values: {},
    },
  },
  decoder: Device,
})
