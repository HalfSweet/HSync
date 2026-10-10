/**
 * Windows of samples fetched for the timeline (waveform columns, decode lanes): wider than the
 * view, so panning draws from data already at hand and asks the backend again only once the
 * view leaves it or the zoom changes.
 */

/** Samples `start..end` at a resolution of `samplesPerPixel`. */
export interface SampleWindow {
  start: number
  end: number
  samplesPerPixel: number
}

/** How far a fetched window reaches beyond the view on each side, in view widths. */
export const OVERSCAN = 1

/** Resolutions this close count as the same zoom. */
const SAME_ZOOM = 0.02

/** The window to fetch for `visible`: `share` of its width more on each side, within `extent`. */
export function overscan(visible: SampleWindow, extent: number, share = OVERSCAN): SampleWindow {
  const margin = (visible.end - visible.start) * share
  return {
    start: Math.max(0, Math.floor(visible.start - margin)),
    end: Math.max(0, Math.min(extent, Math.ceil(visible.end + margin))),
    samplesPerPixel: visible.samplesPerPixel,
  }
}

/**
 * Whether data fetched for `fetched` serves `visible`: it spans the visible samples (those
 * within `extent`) at about the same resolution.
 */
export function covers(fetched: SampleWindow, visible: SampleWindow, extent: number): boolean {
  const ratio = fetched.samplesPerPixel / visible.samplesPerPixel
  if (!(Math.abs(ratio - 1) <= SAME_ZOOM)) return false
  const start = Math.max(0, visible.start)
  const end = Math.min(extent, visible.end)
  return fetched.start <= Math.floor(start) && Math.ceil(end) <= fetched.end
}
