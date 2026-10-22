// SPDX-License-Identifier: Apache-2.0
/** Renesas AUD branch-address trace profile. Wire facts from Renesas AUD trace
 * documentation and the attributed libsigrokdecode AUD reference (fenugrec, 2016;
 * DreamSourceLab, 2019). Original bounded nibble parser, no translated GPL body. */
import type { Channel, Range, Tick, Provenance, Levels, bool, u8, u32, u64 } from 'uiharu'
import {
  LogicDecoder,
  defineLogicDecoder,
  eventSchema,
  mask,
  max,
  atLeast,
  over,
  one,
  join,
} from 'uiharu'
export interface Config {
  clock: Channel
  sync: Channel
  data0: Channel
  data1: Channel
  data2: Channel
  data3: Channel
  initialAddress: u32 | undefined
}
export type End = 'complete' | 'incomplete' | 'overlong' | 'aborted'
export type Event =
  | { kind: 'command'; code: u8; supported: bool }
  | { kind: 'nibble'; trace: u64; index: u8; value: u8 }
  | { kind: 'address'; trace: u64; value: u32 | undefined; bits: u8 }
  | { kind: 'end'; trace: u64; reason: End }
export const events = eventSchema<Event>('aud.events', 1).roles({
  command: 'command',
  nibble: 'data',
  address: 'address',
  end: 'framing',
})
function highest(c: Config): u8 {
  return max(max(c.clock, c.sync), max(max(c.data0, c.data1), max(c.data2, c.data3)))
}
function validate(c: Config): string | undefined {
  if (highest(c) >= 64) return 'aud.config.channel'
  if (
    c.clock === c.sync ||
    c.clock === c.data0 ||
    c.clock === c.data1 ||
    c.clock === c.data2 ||
    c.clock === c.data3 ||
    c.sync === c.data0 ||
    c.sync === c.data1 ||
    c.sync === c.data2 ||
    c.sync === c.data3 ||
    c.data0 === c.data1 ||
    c.data0 === c.data2 ||
    c.data0 === c.data3 ||
    c.data1 === c.data2 ||
    c.data1 === c.data3 ||
    c.data2 === c.data3
  )
    return 'aud.config.duplicateChannel'
  return undefined
}
function nibble(c: Config, l: Levels): u8 {
  return (
    (l.high(c.data0) ? 1 : 0) |
    (l.high(c.data1) ? 2 : 0) |
    (l.high(c.data2) ? 4 : 0) |
    (l.high(c.data3) ? 8 : 0)
  )
}
class Aud extends LogicDecoder<Config, Event> {
  readonly watch = mask(this.config.clock)
  trace: u64 = 0
  expected: u8 = 0
  count: u8 = 0
  value: u32 = this.config.initialAddress ?? 0
  known: bool = this.config.initialAddress !== undefined
  candidate: u32 = 0
  first: Provenance | undefined = undefined
  override hold(): Tick | undefined {
    return this.first?.span.start
  }
  override async onGap(range: Range): Promise<void> {
    await this.cut(range.start)
    this.known = false
  }
  override async onEnd(tick: Tick): Promise<void> {
    await this.cut(tick)
  }
  async cut(tick: Tick): Promise<void> {
    if (this.expected !== 0)
      await this.out.emit(
        { kind: 'end', trace: this.trace, reason: 'aborted' },
        over(tick, tick, tick),
      )
    this.expected = 0
    this.first = undefined
    this.known = false
  }
  async finish(at: Provenance): Promise<void> {
    if (this.expected === 0) return
    const first = this.first
    const placed = first === undefined ? at : join(first, at)
    let reason: End = 'complete'
    if (this.count < this.expected) reason = 'incomplete'
    else if (this.count > this.expected) reason = 'overlong'
    if (reason === 'complete') {
      if (this.expected === 8) this.known = true
      if (this.known) {
        this.value = this.candidate
        await this.out.emit(
          {
            kind: 'address',
            trace: this.trace,
            value: this.value,
            // eslint-disable-next-line @typescript-eslint/no-unnecessary-type-assertion -- The DSL IR requires an explicit u8 narrowing at this boundary.
            bits: (this.expected * 4) as u8,
          },
          placed,
        )
      } else
        await this.out.emit(
          // eslint-disable-next-line @typescript-eslint/no-unnecessary-type-assertion -- The DSL IR requires an explicit u8 narrowing at this boundary.
          { kind: 'address', trace: this.trace, value: undefined, bits: (this.expected * 4) as u8 },
          placed,
        )
    } else this.known = false
    await this.out.emit({ kind: 'end', trace: this.trace, reason }, at)
    this.expected = 0
    this.first = undefined
  }
  override async run(): Promise<void> {
    await this.signal.sample()
    for (;;) {
      const edge = await this.signal.nextChange(this.watch)
      if (!edge.levels.high(this.config.clock)) continue
      const at = one(edge.tick)
      const value = nibble(this.config, edge.levels)
      if (edge.levels.high(this.config.sync)) {
        await this.finish(at)
        const supported = value >= 8 && value <= 11
        await this.out.emit({ kind: 'command', code: value, supported }, at)
        if (supported) {
          this.trace += 1
          this.expected = value === 8 ? 1 : value === 9 ? 2 : value === 10 ? 4 : 8
          this.count = 0
          this.candidate = this.value
          this.first = at
        } else if (value !== 3) this.known = false
        continue
      }
      if (this.expected === 0) continue
      if (this.count < 8) {
        const shift: u64 = this.count * 4
        const target: u32 = 15 << shift
        this.candidate = (this.candidate & ~target) | ((value as u32) << shift)
      }
      if (this.count < 9) {
        await this.out.emit({ kind: 'nibble', trace: this.trace, index: this.count, value }, at)
        this.count += 1
      }
    }
  }
}
export const aud = defineLogicDecoder({
  name: 'aud',
  category: 'bus',
  config: {
    defaults: {
      clock: 0,
      sync: 1,
      data0: 2,
      data1: 3,
      data2: 4,
      data3: 5,
      initialAddress: undefined,
    },
    validate,
  },
  requires: (config) => ({ channels: atLeast(highest(config) + 1) }),
  output: events,
  props: () => ({ mode: 'branch-trace' }),
  locales: {
    'en-US': {
      name: 'AUD branch trace',
      summary: 'Bounded branch destination nibbles; unknown high address context stays unknown',
      fields: {
        clock: 'AUDCK',
        sync: 'nAUDSYNC',
        data0: 'AUDATA0',
        data1: 'AUDATA1',
        data2: 'AUDATA2',
        data3: 'AUDATA3',
        initialAddress: 'Initial address (optional)',
      },
      events: { command: 'Command', nibble: 'Address nibble', address: 'Destination', end: 'End' },
      eventFields: {
        code: 'Command code',
        supported: 'Supported',
        trace: 'Trace',
        index: 'Nibble index',
        value: 'Value',
        bits: 'Changed bits',
        reason: 'End reason',
      },
    },
    'zh-CN': {
      name: 'AUD分支跟踪',
      summary: '解析有界分支目标地址；未知的高地址位保持未知',
      fields: {
        clock: 'AUDCK',
        sync: 'nAUDSYNC',
        data0: 'AUDATA0',
        data1: 'AUDATA1',
        data2: 'AUDATA2',
        data3: 'AUDATA3',
        initialAddress: '初始地址（可选）',
      },
      events: { command: '命令', nibble: '地址半字节', address: '目标地址', end: '结束' },
      eventFields: {
        code: '命令编码',
        supported: '已支持',
        trace: '跟踪编号',
        index: '半字节索引',
        value: '数值',
        bits: '变化位数',
        reason: '结束原因',
      },
    },
  },
  decoder: Aud,
})
