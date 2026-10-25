/**
 * The demo's analog channels in the browser mock: the two sine waves of `hsync-demo`'s
 * `analog.rs`, whose column extents follow from the formula rather than from samples.
 */
import { ANALOG_BASE, analogNumber } from '@/analog/channels'

import type { AnalogBinsResult, AnalogColumns } from '../analogBins'
import type { BinsRequest } from '../bindings/BinsRequest'

interface Sine {
  hertz: number
  centre: number
  amplitude: number
}

/** A0: 1 kHz between 0 and 3.3 V; A1: 3 kHz, ±2.5 V. */
const SINES: readonly Sine[] = [
  { hertz: 1_000, centre: 1.65, amplitude: 1.65 },
  { hertz: 3_000, centre: 0, amplitude: 2.5 },
]

/** The demo's analog channels. */
export const MOCK_ANALOG: readonly number[] = SINES.map((_, n) => ANALOG_BASE + n)

/** Their names before the user gives them others. */
export function mockAnalogNames(): { index: number; name: string }[] {
  return MOCK_ANALOG.map((index) => ({ index, name: analogNumber(index) }))
}

/** Lowest and highest volts of samples `from..=last` at `samplerate`. */
function extent(sine: Sine, samplerate: number, from: number, last: number): [number, number] {
  const turns = (sample: number) => (sine.hertz * sample) / samplerate
  const volts = (at: number) => sine.centre + sine.amplitude * Math.sin(2 * Math.PI * at)
  const [p0, p1] = [turns(from), turns(last)]
  const [peak, trough] = [sine.centre + sine.amplitude, sine.centre - sine.amplitude]
  if (p1 - p0 >= 1) return [trough, peak]
  // Peaks at a quarter turn, troughs at three quarters.
  const reaches = (phase: number) => Math.floor(p1 - phase) >= Math.ceil(p0 - phase)
  const ends = [volts(p0), volts(p1)]
  return [reaches(0.75) ? trough : Math.min(...ends), reaches(0.25) ? peak : Math.max(...ends)]
}

/**
 * The columns `hsync_session::encode_analog_bins` would answer for the demo's sines, `length`
 * samples of them captured so far.
 */
export function mockAnalogBins(
  request: BinsRequest,
  samplerate: number,
  length: number,
): AnalogBinsResult {
  const n = request.end - request.start
  const bins = Math.min(request.bins, n)
  const channels = new Map<number, AnalogColumns>()
  for (const index of request.channels) {
    const sine = SINES[index - ANALOG_BASE]
    if (!sine) throw new Error(`no analog channel ${index}`)
    const columns = new Float32Array(2 * bins).fill(Number.NaN)
    for (let column = 0; column < bins; column++) {
      const from = request.start + Math.floor((n * column) / bins)
      const to = Math.max(from + 1, request.start + Math.floor((n * (column + 1)) / bins))
      if (from >= length) break
      const [low, high] = extent(sine, samplerate, from, Math.min(to, length) - 1)
      columns[2 * column] = low
      columns[2 * column + 1] = high
    }
    const [low, high] =
      length > 0 ? extent(sine, samplerate, 0, length - 1) : [Number.NaN, Number.NaN]
    channels.set(index, { low, high, columns })
  }
  return { start: request.start, end: request.end, bins, channels }
}
