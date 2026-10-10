/**
 * Number formatting for times, frequencies and counts (DESIGN.md §10.3). Units are the same in
 * every language; a value and its unit are separated by one space.
 */

export type TimeUnit = 'ns' | 'µs' | 'ms' | 's'

const TIME_SCALE: Record<TimeUnit, number> = { ns: 1e-9, µs: 1e-6, ms: 1e-3, s: 1 }

/** The unit that keeps `seconds` between 1 and 1000 (or seconds for anything longer). */
export function timeUnitFor(seconds: number): TimeUnit {
  const magnitude = Math.abs(seconds)
  if (magnitude < 1e-6) return 'ns'
  if (magnitude < 1e-3) return 'µs'
  if (magnitude < 1) return 'ms'
  return 's'
}

/** `seconds` expressed in `unit`. */
export function inUnit(seconds: number, unit: TimeUnit): number {
  return seconds / TIME_SCALE[unit]
}

/** Formats a time with a fixed number of fraction digits in the best unit, e.g. `540.0 µs`. */
export function formatTime(seconds: number, digits = 1, unit = timeUnitFor(seconds)): string {
  return `${trimNegativeZero(inUnit(seconds, unit).toFixed(digits))} ${unit}`
}

/**
 * Formats a time with three significant digits, e.g. `5.24 ms`, `50 µs`, as the design shows
 * durations and time per division.
 */
export function formatTimeShort(seconds: number): string {
  const unit = timeUnitFor(seconds)
  return `${significant(inUnit(seconds, unit), 3)} ${unit}`
}

/**
 * Formats a span the way the segment label and hover read-outs do: below a millisecond in µs
 * with one decimal, otherwise in ms with four (`540.0 µs`, `1.0460 ms`).
 */
export function formatSpan(seconds: number): string {
  return Math.abs(seconds) < 1e-3 ? formatTime(seconds, 1, 'µs') : formatTime(seconds, 4, 'ms')
}

export type FrequencyUnit = 'Hz' | 'kHz' | 'MHz' | 'GHz'

function frequencyUnit(hz: number): [FrequencyUnit, number] {
  const magnitude = Math.abs(hz)
  if (magnitude >= 1e9) return ['GHz', 1e9]
  if (magnitude >= 1e6) return ['MHz', 1e6]
  if (magnitude >= 1e3) return ['kHz', 1e3]
  return ['Hz', 1]
}

/** Formats a frequency with a fixed number of fraction digits, e.g. `100.0 kHz`. */
export function formatFrequency(hz: number, digits = 1): string {
  const [unit, scale] = frequencyUnit(hz)
  return `${(hz / scale).toFixed(digits)} ${unit}`
}

/** Formats a measured frequency with three significant digits, e.g. `99.6 kHz`, `1.32 MHz`. */
export function formatFrequencyShort(hz: number): string {
  const [unit, scale] = frequencyUnit(hz)
  return `${Number((hz / scale).toPrecision(3))} ${unit}`
}

/** Formats a sample rate without superfluous zeros, e.g. `25 MHz`, `1.25 MHz`, `500 Hz`. */
export function formatSamplerate(hz: number): string {
  const [unit, scale] = frequencyUnit(hz)
  return `${Number((hz / scale).toPrecision(6))} ${unit}`
}

/**
 * Formats a count with digit grouping. Chinese uses a narrow no-break space between groups
 * (`131 072`) as the design does; other languages follow Intl.
 */
export function formatCount(count: number, locale: string): string {
  if (locale.startsWith('zh')) {
    return Math.round(count)
      .toString()
      .replace(/\B(?=(\d{3})+(?!\d))/g, ' ')
  }
  return new Intl.NumberFormat(locale, { maximumFractionDigits: 0 }).format(count)
}

/** The smallest value of the 1-2-5 sequence that is at least `value`. */
export function niceStep(value: number): number {
  if (!(value > 0) || !Number.isFinite(value)) return 1
  const decade = 10 ** Math.floor(Math.log10(value))
  for (const factor of [1, 2, 5, 10]) {
    if (factor * decade >= value * (1 - 1e-9)) return factor * decade
  }
  return 10 * decade
}

/** Number of fraction digits needed to show multiples of `step` exactly. */
export function digitsFor(step: number): number {
  if (!(step > 0)) return 0
  return Math.max(0, -Math.floor(Math.log10(step) + 1e-9))
}

function significant(value: number, digits: number): string {
  if (value === 0) return '0'
  return String(Number(value.toPrecision(digits)))
}

function trimNegativeZero(text: string): string {
  return /^-0(\.0*)?$/.test(text) ? text.slice(1) : text
}
