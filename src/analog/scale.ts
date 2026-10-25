/**
 * The vertical scale of an analog row (DESIGN.md §6.2): like an oscilloscope's graticule, the
 * row is cut into {@link DIVISIONS} divisions of `perDiv` volts each, `centre` volts on its
 * middle line. By default the scale fits the channel's samples; the user can change the volts
 * per division in the 1-2-5 sequence and move the trace up and down.
 */

/** Divisions of an analog row, top to bottom. */
export const DIVISIONS = 4

/** Volts per division the scale steps through: 1-2-5 from 1 mV to 50 V. */
export const VOLTS_PER_DIV: readonly number[] = [1e-3, 1e-2, 1e-1, 1, 10].flatMap((decade) =>
  [1, 2, 5].map((factor) => Number((factor * decade).toPrecision(1))),
)

export interface VerticalScale {
  /** Volts per division. */
  perDiv: number
  /** Volts on the row's middle line. */
  centre: number
}

/** Room left above the highest sample and below the lowest, as a share of their span. */
const HEADROOM = 0.05

/**
 * The scale that shows samples from `low` to `high` volts: the fewest volts per division that
 * hold them with some room above and below, centred on a multiple of half a division.
 */
export function fitScale(low: number, high: number): VerticalScale {
  if (!Number.isFinite(low) || !Number.isFinite(high) || high < low) {
    return { perDiv: 1, centre: 0 }
  }
  const room = (high - low) * HEADROOM
  for (const perDiv of VOLTS_PER_DIV) {
    const half = perDiv / 2
    // Rounded off, so that the centre reads as the multiple it is.
    const centre = Number((Math.round((low + high) / 2 / half) * half).toPrecision(12)) || 0
    const reach = (perDiv * DIVISIONS) / 2
    if (centre - reach <= low - room && centre + reach >= high + room) return { perDiv, centre }
  }
  return { perDiv: VOLTS_PER_DIV.at(-1) ?? 50, centre: (low + high) / 2 }
}

/** Where `volts` falls in a row `height` pixels high, from its top; outside it when off scale. */
export function yOfVolts(scale: VerticalScale, volts: number, height: number): number {
  return height / 2 - ((volts - scale.centre) / scale.perDiv) * (height / DIVISIONS)
}

/** The volts at `y` pixels from the top of a row `height` pixels high. */
export function voltsAtY(scale: VerticalScale, y: number, height: number): number {
  return scale.centre + ((height / 2 - y) / (height / DIVISIONS)) * scale.perDiv
}

/**
 * The scale `steps` places along the 1-2-5 sequence (positive for more volts per division),
 * keeping the volts at `y` where they are.
 */
export function stepScale(
  scale: VerticalScale,
  steps: number,
  y: number,
  height: number,
): VerticalScale {
  const at = VOLTS_PER_DIV.findIndex((perDiv) => perDiv >= scale.perDiv * (1 - 1e-9))
  const from = at < 0 ? VOLTS_PER_DIV.length - 1 : at
  const index = Math.min(VOLTS_PER_DIV.length - 1, Math.max(0, from + steps))
  const perDiv = VOLTS_PER_DIV[index] ?? scale.perDiv
  const anchor = voltsAtY(scale, y, height)
  return { perDiv, centre: anchor - (anchor - scale.centre) * (perDiv / scale.perDiv) }
}

/** The scale with the trace moved `dy` pixels down in a row `height` pixels high. */
export function moveScale(scale: VerticalScale, dy: number, height: number): VerticalScale {
  return { ...scale, centre: scale.centre + (dy / (height / DIVISIONS)) * scale.perDiv }
}
