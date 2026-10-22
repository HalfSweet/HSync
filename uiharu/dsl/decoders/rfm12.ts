// SPDX-License-Identifier: Apache-2.0
/* eslint-disable @typescript-eslint/no-empty-object-type, @typescript-eslint/no-unnecessary-type-assertion -- DSL integer widths and empty configuration records carry IR meaning. */
/** HopeRF RFM12B command/status words, command table page14:
 * https://www.hoperf.com/uploads/RFM12BDatasheet_1695367948.pdf .
 * Raw 16-bit control and simultaneous status; no over-the-air packet inference. */
import type { Provenance, Transaction, u8, u16, u32, u64 } from 'uiharu'
import { BusDecoder, defineBusDecoder, eventSchema } from 'uiharu'
import { spiEvents, type SpiWord } from './spi'
export type Command =
  | 'configuration'
  | 'power-management'
  | 'frequency'
  | 'data-rate'
  | 'receiver-control'
  | 'data-filter'
  | 'fifo-reset'
  | 'sync-pattern'
  | 'read-fifo'
  | 'afc'
  | 'tx-configuration'
  | 'pll'
  | 'write-tx'
  | 'wake-timer'
  | 'low-duty-cycle'
  | 'low-battery-clock'
  | 'status-read'
  | 'unknown'
export type Field =
  | 'tx-register-enabled'
  | 'fifo-mode-enabled'
  | 'band-code'
  | 'crystal-load-code'
  | 'pin16-vdi'
  | 'vdi-response'
  | 'clock-auto-lock'
  | 'clock-fast-lock'
  | 'analog-filter'
  | 'data-quality-threshold'
  | 'fifo-interrupt-bits'
  | 'single-byte-sync'
  | 'fifo-always-fill'
  | 'fifo-fill-enabled'
  | 'sensitive-reset-disabled'
  | 'afc-mode'
  | 'afc-range-limit'
  | 'afc-strobe'
  | 'afc-fine'
  | 'afc-offset-enabled'
  | 'afc-enabled'
  | 'modulation-inverted'
  | 'clock-buffer-code'
  | 'phase-detector-delay'
  | 'dither-disabled'
  | 'pll-wide-bandwidth'
  | 'wake-exponent'
  | 'wake-mantissa'
  | 'duty-cycle-code'
  | 'duty-cycle-enabled'
  | 'clock-divider-code'
  | 'battery-threshold-code'
  | 'crystal-load-femtofarads'
  | 'frequency-valid'
  | 'frequency-hz'
  | 'battery-threshold-microvolts'
  | 'tx-deviation-hz'
  | 'tx-attenuation-millidb'
  | 'duty-cycle-basis-points'
  | 'receiver-enabled'
  | 'baseband-enabled'
  | 'transmitter-enabled'
  | 'synthesizer-enabled'
  | 'oscillator-enabled'
  | 'low-battery-enabled'
  | 'wake-timer-enabled'
  | 'clock-disabled'
  | 'frequency-code'
  | 'data-rate-bps'
  | 'receiver-bandwidth'
  | 'lna-gain'
  | 'rssi-threshold'
  | 'tx-deviation'
  | 'tx-power'
  | 'fifo-byte'
  | 'sync-byte'
  | 'ready'
  | 'power-on-reset'
  | 'fifo-overflow'
  | 'wake-up'
  | 'external-interrupt'
  | 'low-battery'
  | 'fifo-empty'
  | 'rssi'
  | 'data-quality'
  | 'clock-recovery-lock'
  | 'afc-toggle'
  | 'frequency-offset'
export type End = 'complete' | 'incomplete' | 'aborted' | 'invalid-length'
export type Event =
  | { kind: 'begin'; tx: u64 }
  | { kind: 'control'; tx: u64; command: u16; status: u16; opcode: u8 }
  | { kind: 'command'; tx: u64; command: Command }
  | { kind: 'field'; tx: u64; field: Field; value: u32 }
  | { kind: 'wake-period'; tx: u64; microseconds: u64 }
  | { kind: 'end'; tx: u64; reason: End }
export const events = eventSchema<Event>('rfm12.events', 1).roles({
  begin: 'framing',
  control: 'command',
  command: 'command',
  field: 'data',
  'wake-period': 'data',
  end: 'framing',
})
export interface Config {}
class Device extends BusDecoder<Config, SpiWord, Event, End> {
  open: u64 | undefined = undefined
  band: u8 = 0
  wakeMantissa: u8 | undefined = undefined
  override async recover(reason: End, where: Provenance): Promise<void> {
    this.band = 0
    this.wakeMantissa = undefined
    if (this.open !== undefined) {
      const tx = this.open
      this.open = undefined
      await this.out.emit({ kind: 'end', tx, reason }, where)
    }
  }

  commandName(value: u16): Command {
    if ((value & 0xff00) === 0x8000) return 'configuration'
    if ((value & 0xff00) === 0x8200) return 'power-management'
    if ((value & 0xf000) === 0xa000) return 'frequency'
    if ((value & 0xff00) === 0xc600) return 'data-rate'
    if ((value & 0xf800) === 0x9000) return 'receiver-control'
    if ((value & 0xff00) === 0xc200) return 'data-filter'
    if ((value & 0xff00) === 0xca00) return 'fifo-reset'
    if ((value & 0xff00) === 0xce00) return 'sync-pattern'
    if ((value & 0xff00) === 0xb000) return 'read-fifo'
    if ((value & 0xff00) === 0xc400) return 'afc'
    if ((value & 0xf800) === 0x9800) return 'tx-configuration'
    if ((value & 0xff00) === 0xcc00) return 'pll'
    if ((value & 0xff00) === 0xb800) return 'write-tx'
    if ((value & 0xe000) === 0xe000) return 'wake-timer'
    if ((value & 0xff00) === 0xc800) return 'low-duty-cycle'
    if ((value & 0xff00) === 0xc000) return 'low-battery-clock'
    if (value === 0) return 'status-read'
    return 'unknown'
  }
  async field(tx: u64, field: Field, value: u32, where: Provenance): Promise<void> {
    await this.out.emit({ kind: 'field', tx, field, value }, where)
  }
  async parameters(tx: u64, command: Command, value: u16, where: Provenance): Promise<void> {
    if (command === 'configuration') {
      await this.field(tx, 'tx-register-enabled', (value >> 7) & 1, where)
      await this.field(tx, 'fifo-mode-enabled', (value >> 6) & 1, where)
      await this.field(tx, 'band-code', (value >> 4) & 3, where)
      await this.field(tx, 'crystal-load-code', (value >> 0) & 15, where)
    }
    if (command === 'receiver-control') {
      await this.field(tx, 'pin16-vdi', (value >> 10) & 1, where)
      await this.field(tx, 'vdi-response', (value >> 8) & 3, where)
    }
    if (command === 'data-filter') {
      await this.field(tx, 'clock-auto-lock', (value >> 7) & 1, where)
      await this.field(tx, 'clock-fast-lock', (value >> 6) & 1, where)
      await this.field(tx, 'analog-filter', (value >> 4) & 1, where)
      await this.field(tx, 'data-quality-threshold', (value >> 0) & 7, where)
    }
    if (command === 'fifo-reset') {
      await this.field(tx, 'fifo-interrupt-bits', (value >> 4) & 15, where)
      await this.field(tx, 'single-byte-sync', (value >> 3) & 1, where)
      await this.field(tx, 'fifo-always-fill', (value >> 2) & 1, where)
      await this.field(tx, 'fifo-fill-enabled', (value >> 1) & 1, where)
      await this.field(tx, 'sensitive-reset-disabled', (value >> 0) & 1, where)
    }
    if (command === 'afc') {
      await this.field(tx, 'afc-mode', (value >> 6) & 3, where)
      await this.field(tx, 'afc-range-limit', (value >> 4) & 3, where)
      await this.field(tx, 'afc-strobe', (value >> 3) & 1, where)
      await this.field(tx, 'afc-fine', (value >> 2) & 1, where)
      await this.field(tx, 'afc-offset-enabled', (value >> 1) & 1, where)
      await this.field(tx, 'afc-enabled', (value >> 0) & 1, where)
    }
    if (command === 'tx-configuration') {
      await this.field(tx, 'modulation-inverted', (value >> 8) & 1, where)
    }
    if (command === 'pll') {
      await this.field(tx, 'clock-buffer-code', (value >> 5) & 3, where)
      await this.field(tx, 'phase-detector-delay', (value >> 3) & 1, where)
      await this.field(tx, 'dither-disabled', (value >> 2) & 1, where)
      await this.field(tx, 'pll-wide-bandwidth', (value >> 0) & 1, where)
    }
    if (command === 'wake-timer') {
      await this.field(tx, 'wake-exponent', (value >> 8) & 31, where)
      await this.field(tx, 'wake-mantissa', (value >> 0) & 255, where)
    }
    if (command === 'low-duty-cycle') {
      await this.field(tx, 'duty-cycle-code', (value >> 1) & 127, where)
      await this.field(tx, 'duty-cycle-enabled', (value >> 0) & 1, where)
    }
    if (command === 'low-battery-clock') {
      await this.field(tx, 'clock-divider-code', (value >> 5) & 7, where)
      await this.field(tx, 'battery-threshold-code', (value >> 0) & 15, where)
    }
    if (command === 'configuration') {
      this.band = ((value >> 4) & 3) as u8
      await this.field(tx, 'crystal-load-femtofarads', 8500 + (value & 15) * 500, where)
    }
    if (command === 'frequency') {
      const code: u32 = value & 4095
      const valid = code >= 96 && code <= 3903
      await this.field(tx, 'frequency-valid', valid ? 1 : 0, where)
      if (valid) {
        if (this.band === 1) await this.field(tx, 'frequency-hz', 430000000 + code * 2500, where)
        if (this.band === 2) await this.field(tx, 'frequency-hz', 860000000 + code * 5000, where)
        if (this.band === 3) await this.field(tx, 'frequency-hz', 900000000 + code * 7500, where)
      }
    }
    if (command === 'tx-configuration') {
      await this.field(tx, 'tx-deviation-hz', (((value >> 4) & 15) + 1) * 15000, where)
      await this.field(tx, 'tx-attenuation-millidb', (value & 7) * 2500, where)
    }
    if (command === 'wake-timer') {
      const exponent: u8 = ((value >> 8) & 31) as u8
      this.wakeMantissa = (value & 255) as u8
      const factor: u64 = (1 as u64) << exponent
      const microseconds: u64 = (value & 255) * factor * 1030 + 500
      await this.out.emit({ kind: 'wake-period', tx, microseconds }, where)
    }
    if (command === 'low-duty-cycle') {
      const mantissa = this.wakeMantissa
      if (mantissa !== undefined) {
        if (mantissa > 0)
          await this.field(
            tx,
            'duty-cycle-basis-points',
            ((((value >> 1) & 127) * 2 + 1) * 10000) / mantissa,
            where,
          )
      }
    }
    if (command === 'low-battery-clock')
      await this.field(tx, 'battery-threshold-microvolts', 2250000 + (value & 15) * 100000, where)
  }
  async interpret(tx: u64, value: u16, status: u16, where: Provenance): Promise<void> {
    const command = this.commandName(value)
    await this.out.emit({ kind: 'command', tx, command }, where)
    await this.parameters(tx, command, value, where)
    switch (command) {
      case 'power-management':
        await this.field(tx, 'receiver-enabled', (value >> 7) & 1, where)
        await this.field(tx, 'baseband-enabled', (value >> 6) & 1, where)
        await this.field(tx, 'transmitter-enabled', (value >> 5) & 1, where)
        await this.field(tx, 'synthesizer-enabled', (value >> 4) & 1, where)
        await this.field(tx, 'oscillator-enabled', (value >> 3) & 1, where)
        await this.field(tx, 'low-battery-enabled', (value >> 2) & 1, where)
        await this.field(tx, 'wake-timer-enabled', (value >> 1) & 1, where)
        await this.field(tx, 'clock-disabled', value & 1, where)
        break
      case 'frequency':
        await this.field(tx, 'frequency-code', value & 0xfff, where)
        break
      case 'data-rate':
        await this.field(
          tx,
          'data-rate-bps',
          10000000 / (29 * ((value & 127) + 1) * ((value & 128) !== 0 ? 8 : 1)),
          where,
        )
        break
      case 'receiver-control':
        await this.field(tx, 'receiver-bandwidth', (value >> 5) & 7, where)
        await this.field(tx, 'lna-gain', (value >> 3) & 3, where)
        await this.field(tx, 'rssi-threshold', value & 7, where)
        break
      case 'tx-configuration':
        await this.field(tx, 'tx-deviation', (value >> 4) & 15, where)
        await this.field(tx, 'tx-power', value & 7, where)
        break
      case 'read-fifo':
        await this.field(tx, 'fifo-byte', status & 255, where)
        break
      case 'write-tx':
        await this.field(tx, 'fifo-byte', value & 255, where)
        break
      case 'sync-pattern':
        await this.field(tx, 'sync-byte', value & 255, where)
        break
      default:
        break
    }
    await this.field(tx, 'ready', (status >> 15) & 1, where)
    await this.field(tx, 'power-on-reset', (status >> 14) & 1, where)
    await this.field(tx, 'fifo-overflow', (status >> 13) & 1, where)
    await this.field(tx, 'wake-up', (status >> 12) & 1, where)
    await this.field(tx, 'external-interrupt', (status >> 11) & 1, where)
    await this.field(tx, 'low-battery', (status >> 10) & 1, where)
    await this.field(tx, 'fifo-empty', (status >> 9) & 1, where)
    await this.field(tx, 'rssi', (status >> 8) & 1, where)
    await this.field(tx, 'data-quality', (status >> 7) & 1, where)
    await this.field(tx, 'clock-recovery-lock', (status >> 6) & 1, where)
    await this.field(tx, 'afc-toggle', (status >> 5) & 1, where)
    await this.field(tx, 'frequency-offset', status & 31, where)
  }
  override async transaction(tx: Transaction<SpiWord>): Promise<void> {
    this.open = tx.id
    await this.out.emit({ kind: 'begin', tx: tx.id })
    let command: u16 = 0
    let status: u16 = 0
    for (let i = 0; i < 2; i++) {
      const word = await tx.item()
      command = (command << 8) | word.mosi!
      status = (status << 8) | word.miso!
    }
    if ((await tx.itemOrEnd()) !== undefined) {
      await tx.drain()
      this.open = undefined
      await this.out.emit({ kind: 'end', tx: tx.id, reason: 'invalid-length' })
      return
    }
    const where = this.consumed()
    await this.out.emit(
      { kind: 'control', tx: tx.id, command, status, opcode: (command >> 8) as u8 },
      where,
    )
    await this.interpret(tx.id, command, status, where)
    this.open = undefined
    await this.out.emit({ kind: 'end', tx: tx.id, reason: 'complete' })
  }
}
export const device = defineBusDecoder({
  name: 'rfm12',
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
      name: 'RFM12',
      summary: 'RFM12B named commands, radio parameters and returned status bits',
      fields: {},
      events: {
        begin: 'Begin',
        control: 'Control',
        command: 'Command',
        field: 'Field',
        end: 'End',
        'wake-period': 'Wake period',
      },
      eventFields: {
        tx: 'Transaction',
        command: 'Command',
        status: 'Status',
        opcode: 'Opcode',
        field: 'Field',
        value: 'Value',
        reason: 'Reason',
        microseconds: 'Duration (µs)',
      },
      values: {},
    },
    'zh-CN': {
      name: 'RFM12',
      summary: '解析RFM12B命令、射频参数及返回状态位',
      fields: {},
      events: {
        begin: '开始',
        control: '控制字',
        command: '命令',
        field: '字段',
        end: '结束',
        'wake-period': '唤醒周期',
      },
      eventFields: {
        tx: '事务号',
        command: '命令',
        status: '状态',
        opcode: '操作码',
        field: '字段',
        value: '数值',
        reason: '结束原因',
        microseconds: '时长（微秒）',
      },
      values: {},
    },
  },
  decoder: Device,
})
