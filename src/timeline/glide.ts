/**
 * The view gliding to a new zoom or position (DESIGN.md §13): a notch of the wheel, the zoom and
 * pan keys and the zoom buttons move the view there over a few frames instead of at once.
 */
import type { Viewport } from './viewport'

/** A glide from one view to another, both kept within the capture. */
export interface Glide {
  from: Viewport
  to: Viewport
  /** When it started, in milliseconds (an animation frame's time). */
  start: number
  /** Milliseconds. */
  duration: number
}

/** Quadratic ease-out: twice the average speed at the start, coming to rest at the end. */
export function easeOut(t: number): number {
  return 1 - (1 - t) * (1 - t)
}

/** Below this change of zoom, a glide is a pan. */
const SAME_ZOOM = 1e-6

/**
 * The view a share `p` of the way from `from` to `to`. The zoom changes by the same ratio in
 * equal steps of `p`, so a glide through several notches zooms as evenly as one through one,
 * around the point of the track that shows the same sample in both views, which stays still;
 * a pan moves evenly. Every view on the way is within the capture when both ends are.
 */
export function between(from: Viewport, to: Viewport, p: number): Viewport {
  if (p >= 1) return to
  const ratio = to.samplesPerPixel / from.samplesPerPixel
  if (Math.abs(ratio - 1) < SAME_ZOOM) {
    return { ...to, start: from.start + (to.start - from.start) * p }
  }
  const fixed = (to.start - from.start) / (from.samplesPerPixel - to.samplesPerPixel)
  const anchor = from.start + fixed * from.samplesPerPixel
  const samplesPerPixel = from.samplesPerPixel * ratio ** p
  return { start: anchor - fixed * samplesPerPixel, samplesPerPixel, width: to.width }
}

/** Where the glide is at `now`. */
export function glideAt(glide: Glide, now: number): Viewport {
  const t = glide.duration > 0 ? (now - glide.start) / glide.duration : 1
  return between(glide.from, glide.to, easeOut(Math.min(Math.max(t, 0), 1)))
}

/** Whether the glide has reached its end at `now`. */
export function glideDone(glide: Glide, now: number): boolean {
  return now - glide.start >= glide.duration
}
