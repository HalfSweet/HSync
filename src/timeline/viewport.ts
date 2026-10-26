/**
 * Pure viewport arithmetic: which samples the timeline shows and where they land in pixels.
 * The track is the waveform area right of the label column; x is in CSS pixels from its left.
 */

export interface Viewport {
  /** First visible sample; may be fractional. */
  start: number
  /** Samples per CSS pixel. */
  samplesPerPixel: number
  /** Width of the track in CSS pixels. */
  width: number
}

/** One sample never spans more than this many pixels. */
export const MAX_PIXELS_PER_SAMPLE = 200

export function viewportEnd(view: Viewport): number {
  return view.start + view.width * view.samplesPerPixel
}

export function sampleToX(view: Viewport, sample: number): number {
  return (sample - view.start) / view.samplesPerPixel
}

export function xToSample(view: Viewport, x: number): number {
  return view.start + x * view.samplesPerPixel
}

/** Keeps the zoom between one sample per {@link MAX_PIXELS_PER_SAMPLE} and the whole capture,
 * and the visible range inside the capture. */
export function clampViewport(view: Viewport, sampleCount: number): Viewport {
  if (view.width <= 0 || sampleCount <= 0) return view
  const fitted = sampleCount / view.width
  const samplesPerPixel = Math.min(
    Math.max(view.samplesPerPixel, 1 / MAX_PIXELS_PER_SAMPLE),
    fitted,
  )
  const visible = view.width * samplesPerPixel
  const start = Math.min(Math.max(view.start, 0), Math.max(0, sampleCount - visible))
  return { start, samplesPerPixel, width: view.width }
}

/** The whole capture. */
export function fitViewport(width: number, sampleCount: number): Viewport {
  return { start: 0, samplesPerPixel: width > 0 ? sampleCount / width : 1, width }
}

/** Zooms by `factor` (above 1 zooms in) keeping the sample under `anchorX` in place. */
export function zoomViewport(
  view: Viewport,
  factor: number,
  anchorX: number,
  sampleCount: number,
): Viewport {
  // Clamp the scale before moving the anchor, so zooming past a limit cannot pan the view.
  const zoomed = clampViewport(
    { ...view, samplesPerPixel: view.samplesPerPixel / factor },
    sampleCount,
  )
  return clampViewport(
    {
      ...zoomed,
      start: view.start + anchorX * (view.samplesPerPixel - zoomed.samplesPerPixel),
    },
    sampleCount,
  )
}

/** Moves the view by `dx` pixels; positive values move towards later samples. */
export function panViewport(view: Viewport, dx: number, sampleCount: number): Viewport {
  return clampViewport({ ...view, start: view.start + dx * view.samplesPerPixel }, sampleCount)
}

/** Shows samples `from..to`. */
export function rangeViewport(width: number, from: number, to: number, sampleCount: number) {
  return clampViewport(
    { start: from, samplesPerPixel: Math.max(to - from, 1e-9) / width, width },
    sampleCount,
  )
}
