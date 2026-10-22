// SPDX-License-Identifier: Apache-2.0
/* eslint-disable @typescript-eslint/no-empty-object-type, @typescript-eslint/no-unnecessary-type-assertion -- DSL integer widths and empty configuration records carry IR meaning. */

/** ADF4351 32-bit latch words and every nonreserved register field; RF frequency/state not inferred.
 * Public specification: https://www.analog.com/media/en/technical-documentation/data-sheets/adf4351.pdf .
 * Exactly 32 clocks per selected frame are required. Partial frames are drained;
 * short, overlong and truncated frames never emit a completed semantic value. */
import type { bool, Provenance, Transaction, u8, u16, u32, u64 } from 'uiharu'
import { BusDecoder, defineBusDecoder, eventSchema } from 'uiharu'
import { spiEvents, type SpiWord } from './spi'
export type End = 'complete' | 'incomplete' | 'aborted' | 'invalid-length'
export type Field =
  | 'integer'
  | 'fraction'
  | 'phase-adjust'
  | 'prescaler-89'
  | 'phase'
  | 'modulus'
  | 'noise-mode'
  | 'muxout'
  | 'reference-doubler'
  | 'reference-divide2'
  | 'reference-counter'
  | 'rf-divider-double-buffer'
  | 'charge-pump-current'
  | 'lock-detect-integer'
  | 'lock-detect-precision'
  | 'detector-positive'
  | 'power-down'
  | 'charge-pump-three-state'
  | 'counter-reset'
  | 'band-select-high'
  | 'antibacklash-3ns'
  | 'charge-cancel'
  | 'cycle-slip-reduction'
  | 'clock-divider-mode'
  | 'clock-divider'
  | 'feedback-fundamental'
  | 'rf-divider-code'
  | 'band-select-divider'
  | 'vco-power-down'
  | 'mute-till-lock'
  | 'aux-fundamental'
  | 'aux-enabled'
  | 'aux-power'
  | 'rf-enabled'
  | 'rf-power'
  | 'lock-detect-pin-mode'
export type DeviceEvent =
  | { kind: 'begin'; tx: u64 }
  | { kind: 'register'; tx: u64; address: u8; value: u32 }
  | { kind: 'divider'; tx: u64; integer: u16; fraction: u16 }
  | { kind: 'modulus'; tx: u64; modulus: u16; phase: u16; prescaler89: bool }
  | {
      kind: 'reference'
      tx: u64
      counter: u16
      doubler: bool
      divide2: bool
      chargePump: u8
      powerDown: bool
    }
  | {
      kind: 'output'
      tx: u64
      divider: u8
      feedbackFundamental: bool
      enabled: bool
      power: u8
      bandSelectClock: u8
    }
  | { kind: 'field'; tx: u64; address: u8; field: Field; value: u32 }
  | { kind: 'end'; tx: u64; reason: End }
export const deviceEvents = eventSchema<DeviceEvent>('adf435x.events', 1).roles({
  begin: 'framing',
  register: 'data',
  divider: 'data',
  modulus: 'data',
  reference: 'data',
  output: 'data',
  field: 'data',
  end: 'framing',
})
export interface Config {}
class Device extends BusDecoder<Config, SpiWord, DeviceEvent, End> {
  open: u64 | undefined = undefined
  override async recover(reason: End, where: Provenance): Promise<void> {
    if (this.open !== undefined) {
      const tx = this.open
      this.open = undefined
      await this.out.emit({ kind: 'end', tx, reason }, where)
    }
  }
  async fields(tx: u64, address: u8, value: u32, where: Provenance): Promise<void> {
    if (address === 0) {
      await this.out.emit(
        { kind: 'field', tx, address, field: 'integer', value: (value >> 15) & 65535 },
        where,
      )
      await this.out.emit(
        { kind: 'field', tx, address, field: 'fraction', value: (value >> 3) & 4095 },
        where,
      )
    }
    if (address === 1) {
      await this.out.emit(
        { kind: 'field', tx, address, field: 'phase-adjust', value: (value >> 28) & 1 },
        where,
      )
      await this.out.emit(
        { kind: 'field', tx, address, field: 'prescaler-89', value: (value >> 27) & 1 },
        where,
      )
      await this.out.emit(
        { kind: 'field', tx, address, field: 'phase', value: (value >> 15) & 4095 },
        where,
      )
      await this.out.emit(
        { kind: 'field', tx, address, field: 'modulus', value: (value >> 3) & 4095 },
        where,
      )
    }
    if (address === 2) {
      await this.out.emit(
        { kind: 'field', tx, address, field: 'noise-mode', value: (value >> 29) & 3 },
        where,
      )
      await this.out.emit(
        { kind: 'field', tx, address, field: 'muxout', value: (value >> 26) & 7 },
        where,
      )
      await this.out.emit(
        { kind: 'field', tx, address, field: 'reference-doubler', value: (value >> 25) & 1 },
        where,
      )
      await this.out.emit(
        { kind: 'field', tx, address, field: 'reference-divide2', value: (value >> 24) & 1 },
        where,
      )
      await this.out.emit(
        { kind: 'field', tx, address, field: 'reference-counter', value: (value >> 14) & 1023 },
        where,
      )
      await this.out.emit(
        { kind: 'field', tx, address, field: 'rf-divider-double-buffer', value: (value >> 13) & 1 },
        where,
      )
      await this.out.emit(
        { kind: 'field', tx, address, field: 'charge-pump-current', value: (value >> 9) & 15 },
        where,
      )
      await this.out.emit(
        { kind: 'field', tx, address, field: 'lock-detect-integer', value: (value >> 8) & 1 },
        where,
      )
      await this.out.emit(
        { kind: 'field', tx, address, field: 'lock-detect-precision', value: (value >> 7) & 1 },
        where,
      )
      await this.out.emit(
        { kind: 'field', tx, address, field: 'detector-positive', value: (value >> 6) & 1 },
        where,
      )
      await this.out.emit(
        { kind: 'field', tx, address, field: 'power-down', value: (value >> 5) & 1 },
        where,
      )
      await this.out.emit(
        { kind: 'field', tx, address, field: 'charge-pump-three-state', value: (value >> 4) & 1 },
        where,
      )
      await this.out.emit(
        { kind: 'field', tx, address, field: 'counter-reset', value: (value >> 3) & 1 },
        where,
      )
    }
    if (address === 3) {
      await this.out.emit(
        { kind: 'field', tx, address, field: 'band-select-high', value: (value >> 23) & 1 },
        where,
      )
      await this.out.emit(
        { kind: 'field', tx, address, field: 'antibacklash-3ns', value: (value >> 22) & 1 },
        where,
      )
      await this.out.emit(
        { kind: 'field', tx, address, field: 'charge-cancel', value: (value >> 21) & 1 },
        where,
      )
      await this.out.emit(
        { kind: 'field', tx, address, field: 'cycle-slip-reduction', value: (value >> 18) & 1 },
        where,
      )
      await this.out.emit(
        { kind: 'field', tx, address, field: 'clock-divider-mode', value: (value >> 15) & 3 },
        where,
      )
      await this.out.emit(
        { kind: 'field', tx, address, field: 'clock-divider', value: (value >> 3) & 4095 },
        where,
      )
    }
    if (address === 4) {
      await this.out.emit(
        { kind: 'field', tx, address, field: 'feedback-fundamental', value: (value >> 23) & 1 },
        where,
      )
      await this.out.emit(
        { kind: 'field', tx, address, field: 'rf-divider-code', value: (value >> 20) & 7 },
        where,
      )
      await this.out.emit(
        { kind: 'field', tx, address, field: 'band-select-divider', value: (value >> 12) & 255 },
        where,
      )
      await this.out.emit(
        { kind: 'field', tx, address, field: 'vco-power-down', value: (value >> 11) & 1 },
        where,
      )
      await this.out.emit(
        { kind: 'field', tx, address, field: 'mute-till-lock', value: (value >> 10) & 1 },
        where,
      )
      await this.out.emit(
        { kind: 'field', tx, address, field: 'aux-fundamental', value: (value >> 9) & 1 },
        where,
      )
      await this.out.emit(
        { kind: 'field', tx, address, field: 'aux-enabled', value: (value >> 8) & 1 },
        where,
      )
      await this.out.emit(
        { kind: 'field', tx, address, field: 'aux-power', value: (value >> 6) & 3 },
        where,
      )
      await this.out.emit(
        { kind: 'field', tx, address, field: 'rf-enabled', value: (value >> 5) & 1 },
        where,
      )
      await this.out.emit(
        { kind: 'field', tx, address, field: 'rf-power', value: (value >> 3) & 3 },
        where,
      )
    }
    if (address === 5) {
      await this.out.emit(
        { kind: 'field', tx, address, field: 'lock-detect-pin-mode', value: (value >> 22) & 3 },
        where,
      )
    }
  }
  override async transaction(tx: Transaction<SpiWord>): Promise<void> {
    this.open = tx.id
    await this.out.emit({ kind: 'begin', tx: tx.id })
    let value: u32 = 0
    for (let i = 0; i < 32; i++) value = (value << 1) | ((await tx.item()).mosi! & 1)
    if ((await tx.itemOrEnd()) !== undefined) {
      await tx.drain()
      this.open = undefined
      await this.out.emit({ kind: 'end', tx: tx.id, reason: 'invalid-length' })
      return
    }
    const where = this.consumed()
    const address: u8 = (value & 7) as u8
    await this.out.emit({ kind: 'register', tx: tx.id, address, value }, where)
    await this.fields(tx.id, address, value, where)
    if (address === 0)
      await this.out.emit(
        {
          kind: 'divider',
          tx: tx.id,
          integer: ((value >> 15) & 0xffff) as u16,
          fraction: ((value >> 3) & 0xfff) as u16,
        },
        where,
      )
    else if (address === 1)
      await this.out.emit(
        {
          kind: 'modulus',
          tx: tx.id,
          modulus: ((value >> 3) & 0xfff) as u16,
          phase: ((value >> 15) & 0xfff) as u16,
          prescaler89: (value & 0x8000000) !== 0,
        },
        where,
      )
    else if (address === 2)
      await this.out.emit(
        {
          kind: 'reference',
          tx: tx.id,
          counter: ((value >> 14) & 0x3ff) as u16,
          doubler: (value & 0x2000000) !== 0,
          divide2: (value & 0x1000000) !== 0,
          chargePump: ((value >> 9) & 15) as u8,
          powerDown: (value & 32) !== 0,
        },
        where,
      )
    else if (address === 4)
      await this.out.emit(
        {
          kind: 'output',
          tx: tx.id,
          divider: ((value >> 20) & 7) as u8,
          feedbackFundamental: (value & 0x800000) !== 0,
          enabled: (value & 32) !== 0,
          power: ((value >> 3) & 3) as u8,
          bandSelectClock: ((value >> 12) & 255) as u8,
        },
        where,
      )
    this.open = undefined
    await this.out.emit({ kind: 'end', tx: tx.id, reason: 'complete' })
  }
}
export const device = defineBusDecoder({
  name: 'adf435x',
  category: 'device',
  config: { defaults: {} },
  input: spiEvents,
  requires: () => ({ wordBits: 1, bitOrder: 'msb-first', cs: true, mosi: true }),
  output: deviceEvents,
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
      name: 'ADF435X',
      summary: 'ADF4351 integer, fractional, reference and RF output register fields',
      fields: {},
      events: {
        begin: 'Begin',
        register: 'Register',
        divider: 'Divider',
        modulus: 'Modulus',
        reference: 'Reference',
        output: 'Output',
        end: 'End',
        field: 'Field',
      },
      eventFields: {
        tx: 'Transaction',
        address: 'Address',
        value: 'Value',
        integer: 'Integer',
        fraction: 'Fraction',
        modulus: 'Modulus',
        phase: 'Phase',
        prescaler89: 'Prescaler89',
        counter: 'Counter',
        doubler: 'Doubler',
        divide2: 'Divide2',
        chargePump: 'Charge Pump',
        powerDown: 'Power Down',
        divider: 'Divider',
        feedbackFundamental: 'Feedback Fundamental',
        enabled: 'Enabled',
        power: 'Power',
        bandSelectClock: 'Band Select Clock',
        reason: 'Reason',
        field: 'Field',
      },
      values: {},
    },
    'zh-CN': {
      name: 'ADF435X',
      summary: '解析ADF4351整数、分数、参考及射频输出寄存器字段',
      fields: {},
      events: {
        begin: '开始',
        register: '寄存器',
        divider: '分频器',
        modulus: '模数',
        reference: '参考配置',
        output: '输出',
        end: '结束',
        field: '字段',
      },
      eventFields: {
        tx: '事务号',
        address: '地址',
        value: '数值',
        integer: '整数分频',
        fraction: '分数分频',
        modulus: '模数',
        phase: '相位',
        prescaler89: '8/9预分频',
        counter: '参考计数器',
        doubler: '倍频',
        divide2: '二分频',
        chargePump: '电荷泵',
        powerDown: '掉电',
        divider: '分频器',
        feedbackFundamental: '基波反馈',
        enabled: '启用',
        power: '输出功率',
        bandSelectClock: '频段选择时钟',
        reason: '结束原因',
        field: '字段',
      },
      values: {},
    },
  },
  decoder: Device,
})
