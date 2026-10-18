/**
 * Where a read-out floating over the waveforms goes (DESIGN.md §6.4, §6.5), when the settings
 * put the read-outs there rather than in the status bar: against the edge of the row it reads
 * out, beside the pointer or centred on a measurement, and always inside the visible tracks.
 */

/** Pixels between the pointer and a read-out beside it, clear of the cursor's arrow. */
export const POINTER_GAP = 12

export interface FloatAnchor {
  /** Track x the read-out belongs to: the pointer, or the middle of a measurement. */
  x: number
  /** `beside` starts the read-out right of `x` (ends it left of `x` near the right edge);
   * `centre` centres it on `x`. */
  align: 'beside' | 'centre'
  /** The row it reads out, in the rows' coordinates. */
  top: number
  bottom: number
  /** The side of the row it sits on, unless there is no room there. */
  side: 'above' | 'below'
}

/** The visible part of the tracks: track x from `left` to `right`, rows' y from `top` (under
 * the ruler) to `bottom`. */
export interface FloatBounds {
  left: number
  right: number
  top: number
  bottom: number
}

/** The read-out's top-left corner, given its size. */
export function placeFloat(
  anchor: FloatAnchor,
  size: { width: number; height: number },
  bounds: FloatBounds,
): { left: number; top: number } {
  let left = anchor.align === 'centre' ? anchor.x - size.width / 2 : anchor.x + POINTER_GAP
  if (anchor.align === 'beside' && left + size.width > bounds.right) {
    left = anchor.x - POINTER_GAP - size.width
  }
  // One wider than the tracks keeps its start in view.
  left = Math.max(bounds.left, Math.min(left, bounds.right - size.width))

  // The other side when the ruler or the bottom edge leaves no room on the one asked for.
  const top = { above: anchor.top - size.height, below: anchor.bottom }
  const fits = { above: top.above >= bounds.top, below: top.below + size.height <= bounds.bottom }
  const other = anchor.side === 'above' ? 'below' : 'above'
  const side = fits[anchor.side] || !fits[other] ? anchor.side : other
  return { left: Math.round(left), top: Math.round(top[side]) }
}
