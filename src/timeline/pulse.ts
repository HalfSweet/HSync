/**
 * The hover read-out of a pulse (DESIGN.md §9.6): its level and width within the viewport,
 * plus the period of clocks or the bit count of UART lines.
 */
import type { PulseInfo } from '@/ipc/bindings/PulseInfo'

import type { ChannelRef } from './groups'
import { sampleToX, viewportEnd, type Viewport } from './viewport'

export type PulseMode = 'data' | 'clock' | 'bits'

/** UART lines count bits, data lines show the pulse only, everything else may be a clock. */
export function pulseMode(channel: ChannelRef): PulseMode {
  switch (channel.role) {
    case 'rxtx':
    case 'rx':
    case 'tx':
      return 'bits'
    case 'sda':
    case 'miso':
    case 'mosi':
    case 'cs':
    case 'can_rx':
      return 'data'
    default:
      return 'clock'
  }
}

export interface PulseReading {
  high: boolean
  /** Visible part of the pulse in track pixels. */
  left: number
  width: number
  /** Centre of the read-out, kept between 9 % and 91 % of the track. */
  center: number
  /** Seconds of the visible part. */
  seconds: number
  /** Both edges are visible, so `seconds` is the whole pulse. */
  complete: boolean
  /** Period of a regular clock: this pulse and the next have similar widths. */
  period: number | null
  /** Bits the pulse spans on a UART line. */
  bits: number | null
}

export function readPulse(
  pulse: PulseInfo,
  view: Viewport,
  samplerate: number,
  mode: PulseMode,
  bitSeconds: number | null = null,
): PulseReading {
  const viewStart = view.start
  const viewEnd = viewportEnd(view)
  const from = Math.max(pulse.start, viewStart)
  const to = Math.min(pulse.end, viewEnd)
  const left = sampleToX(view, from)
  const width = Math.max(0, sampleToX(view, to) - left)
  const center = Math.min(view.width * 0.91, Math.max(view.width * 0.09, left + width / 2))
  const complete =
    pulse.startIsEdge && pulse.endIsEdge && pulse.start >= viewStart && pulse.end <= viewEnd
  const seconds = samplerate > 0 ? (to - from) / samplerate : 0

  let period: number | null = null
  if (complete && mode === 'clock' && pulse.nextEnd !== null && samplerate > 0) {
    const own = pulse.end - pulse.start
    const next = pulse.nextEnd - pulse.end
    if (Math.abs(own - next) / Math.max(own, next) < 0.34) period = (own + next) / samplerate
  }
  const bits =
    complete && mode === 'bits' && bitSeconds ? Math.max(1, Math.round(seconds / bitSeconds)) : null

  return { high: pulse.high, left, width, center, seconds, complete, period, bits }
}
