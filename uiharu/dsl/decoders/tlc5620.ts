// SPDX-License-Identifier: Apache-2.0
/* eslint-disable @typescript-eslint/no-unnecessary-type-assertion -- Casts retain field widths in IR. */
/** TI TLC5620 SLAS081E §§data interface, figs1–4, table2:
 * https://www.ti.com/lit/ds/symlink/tlc5620.pdf . DATA shifts on CLK falling.
 * LOAD falling latches the last eleven bits (leading byte padding is allowed).
 * LDAC low updates immediately; its falling edge updates all known DAC latches.
 * Per-DAC references are explicit integer microvolts. No analog settling claim. */
import type { Channel, Levels, Range, Tick, bool, u8, u16, u32, u64 } from 'uiharu'
import {
  LogicDecoder,
  at,
  atLeast,
  defineLogicDecoder,
  eventSchema,
  fromBool,
  mask,
  max,
  mulDiv,
  one,
  over,
} from 'uiharu'
export type End = 'complete' | 'incomplete' | 'aborted'
export type DeviceEvent =
  | {
      kind: 'dac'
      tx: u64
      channel: u8
      doubleRange: bool
      code: u8
      microvolts: u32
      latched: bool
    }
  | { kind: 'voltage-update'; channel: u8; microvolts: u32 }
  | { kind: 'update-all'; knownMask: u8 }
  | { kind: 'end'; tx: u64; reason: End; bits: u8 }
export const deviceEvents = eventSchema<DeviceEvent>('tlc5620.events', 1).roles({
  dac: 'data',
  'voltage-update': 'data',
  'update-all': 'control',
  end: 'framing',
})
export interface Config {
  clk: Channel
  data: Channel
  load: Channel | undefined
  ldac: Channel | undefined
  ldacLowWhenAbsent: bool
  wordAligned: bool
  vrefAMicrovolts: u32
  vrefBMicrovolts: u32
  vrefCMicrovolts: u32
  vrefDMicrovolts: u32
}
function highest(config: Config): u8 {
  return max(max(config.clk, config.data), max(config.load ?? 0, config.ldac ?? 0))
}
function validate(config: Config): string | undefined {
  if (config.load === undefined && !config.wordAligned) return 'tlc5620.config.alignment'
  if (highest(config) >= 64) return 'tlc5620.config.channel'
  if (
    config.clk === config.data ||
    config.clk === config.load ||
    config.clk === config.ldac ||
    config.data === config.load ||
    config.data === config.ldac ||
    (config.load !== undefined && config.load === config.ldac)
  )
    return 'tlc5620.config.duplicateChannel'
  if (
    config.vrefAMicrovolts > 10000000 ||
    config.vrefBMicrovolts > 10000000 ||
    config.vrefCMicrovolts > 10000000 ||
    config.vrefDMicrovolts > 10000000
  )
    return 'tlc5620.config.reference'
  return undefined
}
class Tlc extends LogicDecoder<Config, DeviceEvent> {
  readonly watch = mask(
    this.config.clk,
    this.config.load ?? this.config.clk,
    this.config.ldac ?? this.config.clk,
  )
  aligned = true
  bits: u8 = 0
  word: u16 = 0
  first: Tick = 0
  tx: u64 = 0
  a: u32 | undefined = undefined
  b: u32 | undefined = undefined
  c: u32 | undefined = undefined
  d: u32 | undefined = undefined
  override hold(): Tick | undefined {
    return this.bits > 0 ? this.first : undefined
  }
  async cut(tick: Tick): Promise<void> {
    if (this.bits > 0) {
      this.tx += 1
      await this.out.emit(
        { kind: 'end', tx: this.tx, reason: 'aborted', bits: this.bits },
        at(tick),
      )
    }
    this.bits = 0
    this.word = 0
    this.a = undefined
    this.b = undefined
    this.c = undefined
    this.d = undefined
  }
  override async onGap(range: Range): Promise<void> {
    if (this.config.load === undefined) this.aligned = false
    await this.cut(range.start)
  }
  override async onEnd(tick: Tick): Promise<void> {
    await this.cut(tick)
  }
  reference(channel: u8): u32 {
    if (channel === 0) return this.config.vrefAMicrovolts
    if (channel === 1) return this.config.vrefBMicrovolts
    if (channel === 2) return this.config.vrefCMicrovolts
    return this.config.vrefDMicrovolts
  }
  async latch(tick: Tick, levels: Levels, latched: bool): Promise<void> {
    this.tx += 1
    const bits = this.bits
    this.bits = 0
    if (bits < 11) {
      await this.out.emit({ kind: 'end', tx: this.tx, reason: 'incomplete', bits }, one(tick))
      return
    }
    const channel: u8 = ((this.word >> 9) & 3) as u8
    const code: u8 = (this.word & 255) as u8
    const doubleRange = (this.word & 256) !== 0
    const microvolts: u32 = mulDiv(code, this.reference(channel), doubleRange ? 128 : 256) as u32
    if (latched) {
      if (channel === 0) this.a = microvolts
      else if (channel === 1) this.b = microvolts
      else if (channel === 2) this.c = microvolts
      else this.d = microvolts
    }
    await this.out.emit(
      { kind: 'dac', tx: this.tx, channel, code, doubleRange, microvolts, latched },
      over(this.first, tick + 1, tick),
    )
    const ldac = this.config.ldac
    let low = this.config.ldacLowWhenAbsent
    if (ldac !== undefined) low = !levels.high(ldac)
    if (latched && low)
      await this.out.emit({ kind: 'voltage-update', channel, microvolts }, one(tick))
    await this.out.emit({ kind: 'end', tx: this.tx, reason: 'complete', bits }, one(tick))
  }
  async updateAll(tick: Tick): Promise<void> {
    let knownMask: u8 = 0
    if (this.a !== undefined) knownMask |= 1
    if (this.b !== undefined) knownMask |= 2
    if (this.c !== undefined) knownMask |= 4
    if (this.d !== undefined) knownMask |= 8
    await this.out.emit({ kind: 'update-all', knownMask }, one(tick))
    const a = this.a
    const b = this.b
    const c = this.c
    const d = this.d
    if (a !== undefined)
      await this.out.emit({ kind: 'voltage-update', channel: 0, microvolts: a }, one(tick))
    if (b !== undefined)
      await this.out.emit({ kind: 'voltage-update', channel: 1, microvolts: b }, one(tick))
    if (c !== undefined)
      await this.out.emit({ kind: 'voltage-update', channel: 2, microvolts: c }, one(tick))
    if (d !== undefined)
      await this.out.emit({ kind: 'voltage-update', channel: 3, microvolts: d }, one(tick))
  }
  override async run(): Promise<void> {
    await this.signal.sample()
    for (;;) {
      const edge = await this.signal.nextChange(this.watch)
      const load = this.config.load
      const ldac = this.config.ldac
      if (load !== undefined) {
        if (edge.changed.has(load) && !edge.levels.high(load)) {
          await this.latch(edge.tick, edge.levels, true)
          if (ldac !== undefined) {
            if (edge.changed.has(ldac) && !edge.levels.high(ldac)) await this.updateAll(edge.tick)
          }
          continue
        }
      }
      if (ldac !== undefined) {
        if (edge.changed.has(ldac) && !edge.levels.high(ldac)) {
          await this.updateAll(edge.tick)
          continue
        }
      }
      if (!edge.changed.has(this.config.clk) || edge.levels.high(this.config.clk)) continue
      if (load !== undefined) {
        if (!edge.levels.high(load)) continue
      } else if (!this.aligned) continue
      if (this.bits === 0) this.first = edge.tick
      this.word = ((this.word << 1) | fromBool(edge.levels.high(this.config.data))) & 0x7ff
      if (this.bits < 11) this.bits += 1
      if (load === undefined && this.bits === 11) await this.latch(edge.tick, edge.levels, false)
    }
  }
}
export const device = defineLogicDecoder({
  name: 'tlc5620',
  category: 'device',
  config: {
    defaults: {
      clk: 0,
      data: 1,
      load: 2,
      ldac: undefined,
      ldacLowWhenAbsent: true,
      wordAligned: false,
      vrefAMicrovolts: 3300000,
      vrefBMicrovolts: 3300000,
      vrefCMicrovolts: 3300000,
      vrefDMicrovolts: 3300000,
    },
    validate,
  },
  requires: (config) => ({ channels: atLeast(highest(config) + 1) }),
  output: deviceEvents,
  props: () => ({ fallingEdge: true, wordBits: 11 }),
  locales: {
    'en-US': {
      name: 'TLC5620',
      summary: 'TLC5620 LOAD/LDAC latches, per-DAC reference voltages and simultaneous updates',
      fields: {
        clk: 'Clock channel',
        data: 'Data',
        load: 'LOAD',
        ldac: 'LDAC',
        ldacLowWhenAbsent: 'LDAC absent: low',
        wordAligned: 'Aligned start without LOAD',
        vrefAMicrovolts: 'DAC A reference (µV)',
        vrefBMicrovolts: 'DAC B reference (µV)',
        vrefCMicrovolts: 'DAC C reference (µV)',
        vrefDMicrovolts: 'DAC D reference (µV)',
      },
      events: {
        dac: 'Dac',
        'voltage-update': 'Voltage update',
        'update-all': 'Update all',
        end: 'End',
      },
      eventFields: {
        tx: 'Transaction',
        channel: 'Channel',
        doubleRange: 'Double Range',
        code: 'Code',
        microvolts: 'Voltage (µV)',
        latched: 'Latched',
        knownMask: 'Known Mask',
        reason: 'Reason',
        bits: 'Bits',
      },
      values: {},
    },
    'zh-CN': {
      name: 'TLC5620',
      summary: '解析TLC5620载入与更新控制、各DAC参考电压及同步电压更新',
      fields: {
        clk: '时钟通道',
        data: '数据通道',
        load: 'LOAD',
        ldac: 'LDAC',
        ldacLowWhenAbsent: '未接LDAC时按低电平',
        wordAligned: '未接LOAD时已对齐字',
        vrefAMicrovolts: 'DAC A参考电压(µV)',
        vrefBMicrovolts: 'DAC B参考电压(µV)',
        vrefCMicrovolts: 'DAC C参考电压(µV)',
        vrefDMicrovolts: 'DAC D参考电压(µV)',
      },
      events: {
        dac: 'DAC输出',
        'voltage-update': '电压更新',
        'update-all': '全部DAC更新',
        end: '结束',
      },
      eventFields: {
        tx: '事务号',
        channel: '通道',
        doubleRange: '双倍量程',
        code: '编码',
        microvolts: '电压（微伏）',
        latched: '已锁存',
        knownMask: '已知DAC掩码',
        reason: '结束原因',
        bits: '位数',
      },
      values: {},
    },
  },
  decoder: Tlc,
})
