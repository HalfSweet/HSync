/**
 * Flinging the timeline after a quick drag, DSView's "quick scroll": the release speed of the
 * pointer carries on and dies away exponentially, as scroll views do.
 */

/** A pointer position on the track and when it was there. */
export interface PointerSample {
  x: number
  /** Milliseconds. */
  t: number
}

/** Only the last moves before release count towards its speed. */
const RECENT_MS = 100
/** Releases slower than this, in px/ms, just stop. */
export const FLING_MIN_SPEED = 0.5
/** Time constant of the slowdown. */
const DECAY_MS = 325
/** A fling slower than this, in px/ms, is over. */
const STOP_SPEED = 0.02

/** Pointer speed at release in px/ms from its recent positions; positive is to the right. */
export function releaseSpeed(samples: readonly PointerSample[], now: number): number {
  const recent = samples.filter((sample) => now - sample.t <= RECENT_MS)
  const first = recent[0]
  const last = recent.at(-1)
  if (!first || !last || last.t <= first.t) return 0
  return (last.x - first.x) / (last.t - first.t)
}

/**
 * One frame of a fling at `speed` px/ms lasting `dt` ms: how far it moves and its speed after,
 * or `null` once it has stopped. The distance integrates the decay over the frame, so it does
 * not depend on the frame rate.
 */
export function flingStep(speed: number, dt: number): { dx: number; speed: number } | null {
  if (Math.abs(speed) < STOP_SPEED) return null
  const decay = Math.exp(-dt / DECAY_MS)
  return { dx: speed * DECAY_MS * (1 - decay), speed: speed * decay }
}
