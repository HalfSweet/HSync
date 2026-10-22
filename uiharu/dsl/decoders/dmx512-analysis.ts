// SPDX-License-Identifier: Apache-2.0
/** ANSI E1.11 DMX512-A receiver timing and streamed slot interpretation.
 * Wire facts: https://tsp.esta.org/tsp/documents/published_docs.php (E1.11).
 * Every UART character is processed without a packet buffer. Alternate start
 * codes are identified, without claiming their application payload is decoded.
 */
import type { Provenance, Range, Tick, bool, u8, u32, u64 } from 'uiharu'
import {
  StreamDecoder,
  defineStreamDecoder,
  eventSchema,
  mulDivCeil,
  over,
  saturatingSub,
} from 'uiharu'
import { uartEvents, type UartEvent } from './uart'
export interface Config {
  minBreakUs: u32
  minMabUs: u32
  maxMarkUs: u32
  minBreakIntervalUs: u32
  maxResetIntervalUs: u32
  showZero: bool
}
export type StartCode = 'dimmer' | 'text' | 'rdm' | 'system-information' | 'other'
export type Reason =
  | 'short-break'
  | 'short-mab'
  | 'long-mark'
  | 'short-break-interval'
  | 'long-reset-interval'
  | 'too-many-slots'
  | 'framing-error'
  | 'missing-start-code'
export type Interval = 'mab' | 'inter-slot' | 'inter-packet' | 'break-to-break' | 'reset-to-reset'
export type EndReason = 'break' | 'aborted'
export type Event =
  | { kind: 'begin'; packet: u64; breakTicks: u64 }
  | { kind: 'start-code'; packet: u64; code: u8; profile: StartCode }
  | { kind: 'slot'; packet: u64; slot: u32; code: u8; value: u8; channel: bool }
  | {
      kind: 'timing'
      packet: u64
      interval: Interval
      ticks: u64
    }
  | { kind: 'error'; packet: u64; reason: Reason; slot: u32; ticks: u64 | undefined }
  | { kind: 'end'; packet: u64; slots: u32; reason: EndReason }
export const events = eventSchema<Event>('dmx512-analysis.events', 1).roles({
  begin: 'framing',
  'start-code': 'command',
  slot: 'data',
  timing: 'control',
  error: 'error',
  end: 'framing',
})
function profile(code: u8): StartCode {
  if (code === 0) return 'dimmer'
  if (code === 0x17) return 'text'
  if (code === 0xcc) return 'rdm'
  if (code === 0xcf) return 'system-information'
  return 'other'
}
class Dmx512Analysis extends StreamDecoder<Config, UartEvent, Event> {
  packet: u64 = 0
  open = false
  code: u8 | undefined = undefined
  slots: u32 = 0
  previousBreak: Tick | undefined = undefined
  previousReset: Tick | undefined = undefined
  previousEnd: Tick | undefined = undefined
  async fail(reason: Reason, slot: u32, ticks: u64 | undefined, at: Provenance): Promise<void> {
    await this.out.emit({ kind: 'error', packet: this.packet, reason, slot, ticks }, at)
  }
  async close(reason: EndReason, at: Provenance): Promise<void> {
    if (this.open) {
      if (this.code === undefined) await this.fail('missing-start-code', 0, undefined, at)
      await this.out.emit({ kind: 'end', packet: this.packet, slots: this.slots, reason }, at)
    }
    this.open = false
  }
  override async onGap(range: Range): Promise<void> {
    await this.close('aborted', over(range.start, range.start, range.start))
    this.previousBreak = undefined
    this.previousReset = undefined
    this.previousEnd = undefined
  }
  override async onEnd(tick: Tick): Promise<void> {
    await this.close('aborted', over(tick, tick, tick))
  }
  async mark(end: Tick, interval: Interval, at: Provenance): Promise<void> {
    const start = this.previousEnd
    if (start === undefined) return
    const ticks = saturatingSub(end, start)
    await this.out.emit({ kind: 'timing', packet: this.packet, interval, ticks }, at)
    if (ticks > mulDivCeil(this.rate, this.config.maxMarkUs, 1000000))
      await this.fail('long-mark', this.slots, ticks, at)
    if (interval === 'mab' && ticks < mulDivCeil(this.rate, this.config.minMabUs, 1000000))
      await this.fail('short-mab', 0, ticks, at)
  }
  override async run(): Promise<void> {
    for (;;) {
      const received = await this.input.next()
      const e = received.event
      const at = received.at
      if (e.kind === 'break') {
        await this.mark(at.span.start, 'inter-packet', at)
        await this.close('break', over(at.span.start, at.span.start, at.settled))
        this.packet += 1
        this.open = true
        this.code = undefined
        this.slots = 0
        await this.out.emit({ kind: 'begin', packet: this.packet, breakTicks: e.ticks }, at)
        if (e.ticks < mulDivCeil(this.rate, this.config.minBreakUs, 1000000))
          await this.fail('short-break', 0, e.ticks, at)
        const last = this.previousBreak
        if (last !== undefined) {
          const ticks = saturatingSub(at.span.start, last)
          await this.out.emit(
            { kind: 'timing', packet: this.packet, interval: 'break-to-break', ticks },
            at,
          )
          if (ticks < mulDivCeil(this.rate, this.config.minBreakIntervalUs, 1000000))
            await this.fail('short-break-interval', 0, ticks, at)
        }
        this.previousBreak = at.span.start
        this.previousEnd = at.span.end
      } else if (e.kind === 'frame' && this.open) {
        const first = this.code === undefined
        await this.mark(at.span.start, first ? 'mab' : 'inter-slot', at)
        if (first) {
          const code: u8 = e.value as u8
          this.code = code
          await this.out.emit(
            { kind: 'start-code', packet: this.packet, code, profile: profile(code) },
            at,
          )
          if (code === 0) {
            const last = this.previousReset
            if (last !== undefined) {
              const ticks = saturatingSub(at.span.end, last)
              await this.out.emit(
                { kind: 'timing', packet: this.packet, interval: 'reset-to-reset', ticks },
                at,
              )
              if (ticks > mulDivCeil(this.rate, this.config.maxResetIntervalUs, 1000000))
                await this.fail('long-reset-interval', 0, ticks, at)
            }
            this.previousReset = at.span.end
          }
        } else {
          this.slots += 1
          const code = this.code
          if (code !== undefined && (code !== 0 || this.config.showZero || e.value !== 0))
            await this.out.emit(
              {
                kind: 'slot',
                packet: this.packet,
                slot: this.slots,
                code,
                value: e.value as u8,
                channel: code === 0,
              },
              at,
            )
          if (this.slots === 513) await this.fail('too-many-slots', this.slots, undefined, at)
        }
        if (!e.framingOk) await this.fail('framing-error', first ? 0 : this.slots, undefined, at)
        this.previousEnd = at.span.end
      }
    }
  }
}
export const dmx512Analysis = defineStreamDecoder({
  name: 'dmx512-analysis',
  category: 'bus',
  config: {
    defaults: {
      minBreakUs: 88,
      minMabUs: 8,
      maxMarkUs: 1000000,
      minBreakIntervalUs: 1196,
      maxResetIntervalUs: 1250000,
      showZero: false,
    },
  },
  input: uartEvents,
  requires: () => ({
    baud: 250000,
    dataBits: 8,
    parity: 'none',
    stopBits: 'two',
    bitOrder: 'lsb-first',
  }),
  output: events,
  locales: {
    'en-US': {
      name: 'DMX512 timing and slots',
      summary:
        'Receiver timing checks, zero-channel filtering and alternate start-code identification',
      fields: {
        minBreakUs: 'Minimum BREAK (µs)',
        minMabUs: 'Minimum MAB (µs)',
        maxMarkUs: 'Maximum MARK (µs)',
        minBreakIntervalUs: 'Minimum BREAK interval (µs)',
        maxResetIntervalUs: 'Maximum RESET interval (µs)',
        showZero: 'Show zero dimmer values',
      },
      events: {
        begin: 'Begin',
        'start-code': 'Start code',
        slot: 'Slot',
        timing: 'Timing',
        error: 'Protocol error',
        end: 'End',
      },
      eventFields: {
        packet: 'Packet',
        breakTicks: 'BREAK ticks',
        code: 'Start code',
        profile: 'Start-code profile',
        slot: 'Slot',
        value: 'Value',
        channel: 'Dimmer channel',
        interval: 'Interval',
        ticks: 'Ticks',
        reason: 'Reason',
        slots: 'Data slots',
      },
    },
    'zh-CN': {
      name: 'DMX512时序和槽位',
      summary: '接收端时序检查、零值通道过滤和备用起始码识别',
      fields: {
        minBreakUs: '最短BREAK（µs）',
        minMabUs: '最短MAB（µs）',
        maxMarkUs: '最长MARK（µs）',
        minBreakIntervalUs: '最短BREAK间隔（µs）',
        maxResetIntervalUs: '最长RESET间隔（µs）',
        showZero: '显示零值调光通道',
      },
      events: {
        begin: '开始',
        'start-code': '起始码',
        slot: '槽位',
        timing: '时序',
        error: '协议错误',
        end: '结束',
      },
      eventFields: {
        packet: '包',
        breakTicks: 'BREAK采样数',
        code: '起始码',
        profile: '起始码格式',
        slot: '槽位',
        value: '值',
        channel: '调光通道',
        interval: '间隔',
        ticks: '采样数',
        reason: '原因',
        slots: '数据槽位',
      },
    },
  },
  decoder: Dmx512Analysis,
})
