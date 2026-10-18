// SPDX-License-Identifier: Apache-2.0

/**
 * The PWM decoder: the cycles of one pulse-width modulated line, each measured in ticks from
 * one leading edge to the next.
 *
 * Pulse-width modulation follows no published specification; the rules are these:
 *
 * - the line is active high or active low as configured: its leading edge is the change from
 *   inactive to active, its trailing edge the change back;
 * - a cycle runs from one leading edge to the next: its period is the ticks between them, its
 *   active time the ticks from the leading edge to the trailing edge in between;
 * - a cycle is reported once the next leading edge is seen; it covers the ticks from its own
 *   leading edge up to that one and is settled there; duty cycle and frequency are the
 *   reader's business, the stream carries ticks;
 * - progress is passed on, held back to the leading edge of the cycle under way, whose period
 *   is not known yet;
 * - the first sample of the stream and the first after a gap are never edges, so the first
 *   cycle begins at the first leading edge seen after them;
 * - a cycle cut short by a gap or by the end of the samples is not reported, and a line that
 *   stays at one level has no cycles.
 *
 * Limits: one line, and no glitch filter: every change of level is an edge, so a glitch
 * makes cycles of its own.
 */

import type { Channel, Tick, bool, u64 } from 'uiharu'
import {
  LogicDecoder,
  MAX_CHANNELS,
  atLeast,
  defineLogicDecoder,
  eventSchema,
  mask,
  over,
} from 'uiharu'

/** What travels on a PWM event stream. */
export type PwmEvent = {
  /** One cycle, from a leading edge, where the line turned active, to the next one. */
  kind: 'cycle'
  /** Ticks from the leading edge to the next one. */
  periodTicks: u64
  /** Ticks the line was active: from the leading edge to the trailing edge, where it turned
   * inactive again. */
  activeTicks: u64
}

/** The schema of PWM event streams. */
export const pwmEvents = eventSchema<PwmEvent>('pwm.events', 1)

/** Which level of the line is active. */
export type Polarity = 'active-high' | 'active-low'

/** Which line carries the signal and which of its levels is active. */
export interface PwmConfig {
  /** The line's channel. */
  channel: Channel
  /** Which level is active. */
  polarity: Polarity
}

/** Whether the line is active when it is high (`high`) or low. */
export function isActive(polarity: Polarity, high: bool): bool {
  return high === (polarity === 'active-high')
}

class Pwm extends LogicDecoder<PwmConfig, PwmEvent> {
  readonly watch = mask(this.config.channel)
  /** The leading edge of the cycle under way, and its trailing edge once seen. */
  leading: Tick | undefined = undefined
  trailing: Tick | undefined = undefined

  /** The cycle under way will span from its leading edge. */
  override hold(): Tick | undefined {
    return this.leading
  }

  override onGap(): void {
    this.leading = undefined
    this.trailing = undefined
  }

  override async run(): Promise<void> {
    for (;;) {
      const edge = await this.signal.nextChange(this.watch)
      if (!isActive(this.config.polarity, edge.levels.high(this.config.channel))) {
        if (this.leading !== undefined) this.trailing = edge.tick
        continue
      }
      if (this.leading !== undefined && this.trailing !== undefined) {
        await this.out.emit(
          {
            kind: 'cycle',
            periodTicks: edge.tick - this.leading,
            activeTicks: this.trailing - this.leading,
          },
          over(this.leading, edge.tick, edge.tick),
        )
      }
      this.leading = edge.tick
      this.trailing = undefined
    }
  }
}

/** The PWM decoder node: logic samples in, `pwm.events` out. */
export const pwm = defineLogicDecoder({
  name: 'pwm',
  category: 'line',
  config: {
    defaults: { channel: 0, polarity: 'active-high' },
    validate: (config) =>
      config.channel >= MAX_CHANNELS
        ? `channel ${config.channel} is out of range; channels are 0 to ${MAX_CHANNELS - 1}`
        : undefined,
  },
  requires: (config) => ({ channels: atLeast(config.channel + 1) }),
  output: pwmEvents,
  props: (config) => ({ polarity: config.polarity }),
  locales: {
    'en-US': {
      name: 'PWM',
      summary: 'Each cycle of a PWM line: its period and its active time',
      fields: { channel: 'Signal line', polarity: 'Polarity' },
      values: { polarity: { 'active-high': 'Active high', 'active-low': 'Active low' } },
      events: { cycle: 'Cycle' },
      eventFields: { periodTicks: 'Period (ticks)', activeTicks: 'Active time (ticks)' },
    },
    'zh-CN': {
      name: 'PWM',
      summary: '测量 PWM 信号线的每个周期：周期与有效时长',
      fields: { channel: '信号线', polarity: '极性' },
      values: { polarity: { 'active-high': '高电平有效', 'active-low': '低电平有效' } },
      events: { cycle: '周期' },
      eventFields: { periodTicks: '周期 (采样点)', activeTicks: '有效时长 (采样点)' },
    },
  },
  decoder: Pwm,
})
