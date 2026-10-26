/**
 * Ruler and overview ticks. Times are relative to the trigger (DESIGN.md §5, §6.1); majors
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

/** Minimum distance between major ticks; long labels push them further apart. */
export const MIN_MAJOR_SPACING = 70

/** Room between the text of neighbouring labels, their 3 px padding included. */
export const LABEL_GAP = 12

/**
 * Width of a label's text in CSS pixels, set as tick labels are: 11 px (`--text-caption`) IBM
 * Plex Sans, whose figures are all 0.6 em wide.
 */
export function labelWidth(text: string): number {
  let width = 0
  for (const char of text) width += char === '.' ? 3 : char === '-' ? 4.4 : 6.6
  return width
}

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
  const t0 = (view.start - base.triggerSample) / base.samplerate
  const t1 = t0 + view.width * secondsPerPixel
  // Labels lengthen with the time and with the decimals a fine step needs, such as 10.80115195
  // deep into a long capture: take larger steps until the widest, at an end of the view, fits.
  let step = niceStep(secondsPerPixel * minSpacing)
  let unit: TimeUnit
  let digits: number
  for (;;) {
    unit = timeUnitFor(Math.max(Math.abs(t0), Math.abs(t1), step))
    digits = digitsFor(inUnit(step, unit))
    const widest = Math.max(
      labelWidth(label(t0, unit, digits)),
      labelWidth(label(t1, unit, digits)),
    )
    if (step / secondsPerPixel >= widest + LABEL_GAP) break
    step = niceStep(step * 1.5)
  }
  const minor = step / minorDivisions(step)
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
