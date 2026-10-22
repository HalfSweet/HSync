// SPDX-License-Identifier: Apache-2.0
/* eslint-disable @typescript-eslint/no-unnecessary-type-assertion -- DSL integer widths and empty configuration records carry IR meaning. */

/** AD7910/AD7920 16-clock conversion frames, four leading zero bits and optional two trailing zeros; fixed-point voltage with explicit reference and early-CS power-down requests.
 * Public specification: https://www.analog.com/media/en/technical-documentation/data-sheets/ad7910_7920.pdf .
 * Exactly 16 clocks per selected frame are required. Partial frames are drained;
 * short, overlong and truncated frames never emit a completed semantic value. */
import type { bool, Provenance, Transaction, u8, u16, u32, u64 } from 'uiharu'
import { BusDecoder, defineBusDecoder, eventSchema, mulDiv } from 'uiharu'
import { spiEvents, type SpiWord } from './spi'
export type End = 'complete' | 'incomplete' | 'aborted' | 'invalid-length'
export type Mode = 'conversion' | 'power-down-request' | 'invalid-short-frame'
export type DeviceEvent =
  | { kind: 'begin'; tx: u64 }
  | { kind: 'mode'; tx: u64; mode: Mode; clocks: u8 }
  | {
      kind: 'sample'
      tx: u64
      code: u16
      bits: u8
      leading: u8
      trailing: u8
      microvolts: u32
      valid: bool
    }
  | { kind: 'end'; tx: u64; reason: End }
export const deviceEvents = eventSchema<DeviceEvent>('ad79x0.events', 1).roles({
  begin: 'framing',
  mode: 'control',
  sample: 'data',
  end: 'framing',
})
export interface Config {
  tenBit: bool
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
    for (let i: u8 = 0; i < 16; i++) {
      const word = await tx.itemOrEnd()
      if (word === undefined) {
        await this.out.emit({
          kind: 'mode',
          tx: tx.id,
          mode: i >= 2 && i <= 10 ? 'power-down-request' : 'invalid-short-frame',
          clocks: i as u8,
        })
        this.open = undefined
        await this.out.emit({ kind: 'end', tx: tx.id, reason: 'invalid-length' })
        return
      }
      value = (value << 1) | (word.miso! & 1)
    }
    if ((await tx.itemOrEnd()) !== undefined) {
      await tx.drain()
      this.open = undefined
      await this.out.emit({ kind: 'end', tx: tx.id, reason: 'invalid-length' })
      return
    }
    const where = this.consumed()
    await this.out.emit({ kind: 'mode', tx: tx.id, mode: 'conversion', clocks: 16 }, where)
    const bits: u8 = this.config.tenBit ? 10 : 12
    const code: u16 = this.config.tenBit
      ? (((value >> 2) & 0x3ff) as u16)
      : ((value & 0xfff) as u16)
    await this.out.emit(
      {
        kind: 'sample',
        tx: tx.id,
        code,
        bits,
        leading: ((value >> 12) & 15) as u8,
        trailing: this.config.tenBit ? ((value & 3) as u8) : 0,
        microvolts: mulDiv(code, this.config.referenceMicrovolts, 1 << bits) as u32,
        valid: value >> 12 === 0 && (!this.config.tenBit || (value & 3) === 0),
      },
      where,
    )
    this.open = undefined
    await this.out.emit({ kind: 'end', tx: tx.id, reason: 'complete' })
  }
}
export const device = defineBusDecoder({
  name: 'ad79x0',
  category: 'device',
  config: { defaults: { tenBit: false, referenceMicrovolts: 3300000 } },
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
      name: 'AD79X0',
      summary: 'AD7910/7920 conversion voltage, padding checks and power-down requests',
      fields: { tenBit: 'AD7910 10-bit mode', referenceMicrovolts: 'Reference voltage (µV)' },
      events: { begin: 'Begin', mode: 'Mode', sample: 'Sample', end: 'End' },
      eventFields: {
        tx: 'Transaction',
        mode: 'Mode',
        clocks: 'Clocks',
        code: 'Code',
        bits: 'Bits',
        leading: 'Leading',
        trailing: 'Trailing',
        microvolts: 'Voltage (µV)',
        valid: 'Valid',
        reason: 'Reason',
      },
      values: {},
    },
    'zh-CN': {
      name: 'AD79X0',
      summary: '解析AD7910/7920转换电压、填充位检查及掉电请求',
      fields: { tenBit: 'AD7910十位模式', referenceMicrovolts: '参考电压（微伏）' },
      events: { begin: '开始', mode: '模式', sample: '采样', end: '结束' },
      eventFields: {
        tx: '事务号',
        mode: '模式',
        clocks: '时钟数',
        code: '编码',
        bits: '位数',
        leading: '前导位',
        trailing: '尾随位',
        microvolts: '电压（微伏）',
        valid: '有效',
        reason: '结束原因',
      },
      values: {},
    },
  },
  decoder: Device,
})
