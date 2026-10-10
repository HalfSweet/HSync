/**
 * Ruler and overview ticks. Times are relative to the trigger (DESIGN.md §5.1, §9.6); majors
 * follow the 1-2-5 sequence and carry labels in one unit for the whole ruler.
 */
import { digitsFor, inUnit, niceStep, timeUnitFor, type TimeUnit } from '@/utils/units'

import type { Viewport } from './viewport'

export interface Tick {
  /** Position in CSS pixels from the left of the track. */
  x: number
  /** Seconds from the trigger. */
  time: number
  major: boolean
  /** Text of a major tick, in the ruler's unit. */
  label: string
}

export interface Ruler {
  /** Seconds between major ticks: the time per division. */
  step: number
  unit: TimeUnit
  ticks: Tick[]
}

export interface TimeBase {
  samplerate: number
  triggerSample: number
}

/** Minimum distance between major ticks, so labels never touch. */
export const MIN_MAJOR_SPACING = 70

function label(time: number, unit: TimeUnit, digits: number): string {
  const text = inUnit(time, unit).toFixed(digits)
  return /^-0(\.0*)?$/.test(text) ? text.slice(1) : text
}

/** Minor divisions per major step: fifths for 1 and 5, quarters for 2. */
function minorDivisions(step: number): number {
  const mantissa = Math.round(step / 10 ** Math.floor(Math.log10(step) + 1e-9))
  return mantissa === 2 ? 4 : 5
}

export function rulerTicks(view: Viewport, base: TimeBase, minSpacing = MIN_MAJOR_SPACING): Ruler {
  if (view.width <= 0 || base.samplerate <= 0) return { step: 0, unit: 's', ticks: [] }
  const secondsPerPixel = view.samplesPerPixel / base.samplerate
  const step = niceStep(secondsPerPixel * minSpacing)
  const minor = step / minorDivisions(step)
  const t0 = (view.start - base.triggerSample) / base.samplerate
  const t1 = t0 + view.width * secondsPerPixel
  const unit = timeUnitFor(Math.max(Math.abs(t0), Math.abs(t1), step))
  const digits = digitsFor(inUnit(step, unit))
  const toX = (time: number) => (time - t0) / secondsPerPixel

  const ticks: Tick[] = []
  const majorsPerMinor = Math.round(step / minor)
  for (let k = Math.ceil(t0 / minor - 1e-9); k * minor <= t1 + 1e-12; k++) {
    const time = k * minor
    const major = k % majorsPerMinor === 0
    ticks.push({ x: toX(time), time, major, label: major ? label(time, unit, digits) : '' })
  }
  return { step, unit, ticks }
}

/** Ticks of the whole-capture overview: majors only, at least `minSpacing` pixels apart. */
export function overviewTicks(
  width: number,
  sampleCount: number,
  base: TimeBase,
  minSpacing = 120,
): Ruler {
  if (width <= 0 || sampleCount <= 0) return { step: 0, unit: 's', ticks: [] }
  const ruler = rulerTicks(
    { start: 0, samplesPerPixel: sampleCount / width, width },
    base,
    minSpacing,
  )
  return { ...ruler, ticks: ruler.ticks.filter((tick) => tick.major) }
}
