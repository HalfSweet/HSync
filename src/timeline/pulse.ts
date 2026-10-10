/**
 * The hover read-out of a pulse (DESIGN.md §6.4): its level and width within the viewport,
 * plus the period, frequency and duty cycle it makes with the next pulse, or the bit count of
 * UART lines, and where to mark the pulse on its track.
 */
import type { PulseInfo } from '@/ipc/bindings/PulseInfo'

import type { ChannelRef } from './groups'
import { sampleToX, viewportEnd, type Viewport } from './viewport'

export type PulseMode = 'data' | 'clock' | 'bits'

/** UART lines count bits, data lines show the pulse only, everything else reads a period. */
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
  /** Seconds of the visible part. */
  seconds: number
  /** Both edges are visible, so `seconds` is the whole pulse. */
  complete: boolean
  /** This pulse and the next one, when the next one ends at an edge. */
  period: number | null
  /** Share of the period at the high level, with `period`. */
  duty: number | null
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
  const complete =
    pulse.startIsEdge && pulse.endIsEdge && pulse.start >= viewStart && pulse.end <= viewEnd
  const seconds = samplerate > 0 ? (to - from) / samplerate : 0

  let period: number | null = null
  let duty: number | null = null
  if (complete && mode === 'clock' && pulse.nextEnd !== null && samplerate > 0) {
    // As DSView measures it: this pulse and the next make the period, with no guess at
    // whether the signal repeats.
    const own = pulse.end - pulse.start
    const next = pulse.nextEnd - pulse.end
    period = (own + next) / samplerate
    duty = (pulse.high ? own : next) / (own + next)
  }
  const bits =
    complete && mode === 'bits' && bitSeconds ? Math.max(1, Math.round(seconds / bitSeconds)) : null

  return { high: pulse.high, left, width, seconds, complete, period, duty, bits }
}
