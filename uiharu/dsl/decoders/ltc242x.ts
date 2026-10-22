// SPDX-License-Identifier: Apache-2.0
/* eslint-disable @typescript-eslint/no-unnecessary-type-assertion -- DSL integer widths and empty configuration records carry IR meaning. */

/** LTC2420 24-clock output, 20-bit code, EOC/SIG/EXR and LTC2422 channel identification; voltage magnitude uses configured reference.
 * Public specification: https://www.analog.com/media/en/technical-documentation/data-sheets/2420f.pdf .
 * Exactly 24 clocks per selected frame are required. Partial frames are drained;
 * short, overlong and truncated frames never emit a completed semantic value. */
import type { bool, Provenance, Transaction, u8, u32, u64 } from 'uiharu'
import { BusDecoder, defineBusDecoder, eventSchema, mulDiv } from 'uiharu'
import { spiEvents, type SpiWord } from './spi'
export type End = 'complete' | 'incomplete' | 'aborted' | 'invalid-length'
export type DeviceEvent =
  | { kind: 'begin'; tx: u64 }
  | {
      kind: 'sample'
      tx: u64
      code: u32
      eoc: bool
      dummy: bool
      positive: bool
      extended: bool
      channel: u8
      microvolts: u32
      negative: bool
      valid: bool
    }
  | { kind: 'end'; tx: u64; reason: End }
export const deviceEvents = eventSchema<DeviceEvent>('ltc242x.events', 1).roles({
  begin: 'framing',
  sample: 'data',
  end: 'framing',
})
export type Model = 'ltc2420' | 'ltc2421' | 'ltc2422'
export interface Config {
  model: Model
  referenceMicrovolts: u32
}
class Device extends BusDecoder<Config, SpiWord, DeviceEvent, End> {
  open: u64 | undefined = undefined
  override async recover(reason: End, where: Provenance): Promise<void> {
    if (this.open !== undefined) {
      const tx = this.open
      this.open = undefined
      await this.out.emit({ kind: 'end', tx, reason }, where)
    }
  }
  override async transaction(tx: Transaction<SpiWord>): Promise<void> {
    this.open = tx.id
    await this.out.emit({ kind: 'begin', tx: tx.id })
    let value: u32 = 0
    for (let i = 0; i < 24; i++) value = (value << 1) | ((await tx.item()).miso! & 1)
    if ((await tx.itemOrEnd()) !== undefined) {
      await tx.drain()
      this.open = undefined
      await this.out.emit({ kind: 'end', tx: tx.id, reason: 'invalid-length' })
      return
    }
    const code: u32 = value & 0xfffff
    const negative = (value & 0x200000) === 0
    const magnitude: u32 = negative
      ? (value & 0x1fffff) === 0
        ? 0
        : 0x200000 - (value & 0x1fffff)
      : value & 0x1fffff
    await this.out.emit({
      kind: 'sample',
      tx: tx.id,
      code,
      eoc: (value & 0x800000) !== 0,
      dummy: (value & 0x400000) !== 0,
      positive: (value & 0x200000) !== 0,
      extended: (value & 0x100000) !== 0,
      channel: this.config.model === 'ltc2422' ? (((value >> 22) & 1) as u8) : 0,
      microvolts: mulDiv(magnitude, this.config.referenceMicrovolts, 1048576) as u32,
      negative,
      valid:
        (value & 0x800000) === 0 &&
        (this.config.model === 'ltc2422' || (value & 0x400000) === 0) &&
        (!negative || (value & 0x100000) !== 0 || code === 0),
    })
    this.open = undefined
    await this.out.emit({ kind: 'end', tx: tx.id, reason: 'complete' })
  }
}
export const device = defineBusDecoder({
  name: 'ltc242x',
  category: 'device',
  config: { defaults: { model: 'ltc2422', referenceMicrovolts: 3300000 } },
  input: spiEvents,
  requires: () => ({ wordBits: 1, bitOrder: 'msb-first', cs: true, miso: true }),
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
      name: 'LTC242X',
      summary: 'LTC2420/21/22 conversion status, channel and signed voltage',
      fields: { model: 'Model', referenceMicrovolts: 'Reference voltage (µV)' },
      events: { begin: 'Begin', sample: 'Sample', end: 'End' },
      eventFields: {
        tx: 'Transaction',
        code: 'Code',
        eoc: 'End of conversion',
        dummy: 'Dummy',
        positive: 'Positive',
        extended: 'Extended',
        channel: 'Channel',
        microvolts: 'Voltage (µV)',
        negative: 'Negative',
        valid: 'Valid',
        reason: 'Reason',
      },
      values: { model: { ltc2420: 'LTC2420', ltc2421: 'LTC2421', ltc2422: 'LTC2422' } },
    },
    'zh-CN': {
      name: 'LTC242X',
      summary: '解析LTC2420/21/22转换状态、通道及有符号电压',
      fields: { model: '型号', referenceMicrovolts: '参考电压（微伏）' },
      events: { begin: '开始', sample: '采样', end: '结束' },
      eventFields: {
        tx: '事务号',
        code: '编码',
        eoc: '转换结束',
        dummy: '空位',
        positive: '正号',
        extended: '扩展范围',
        channel: '通道',
        microvolts: '电压（微伏）',
        negative: '负数',
        valid: '有效',
        reason: '结束原因',
      },
      values: { model: { ltc2420: 'LTC2420', ltc2421: 'LTC2421', ltc2422: 'LTC2422' } },
    },
  },
  decoder: Device,
})
