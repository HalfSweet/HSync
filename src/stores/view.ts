import { defineStore } from 'pinia'
import { computed, shallowRef, watch } from 'vue'

import { rulerTicks } from '@/timeline/ticks'
import {
  clampViewport,
  fitViewport,
  panViewport,
  rangeViewport,
  viewportEnd,
  zoomViewport,
  type Viewport,
} from '@/timeline/viewport'

import { useCaptureStore } from './capture'

/** A stretch of time chosen on the ruler or by clicking a decoded block (DESIGN.md §9.6). */
export interface Segment {
  /** First sample (A). */
  from: number
  /** Last sample (B); equal to `from` while the drag has not moved. */
  to: number
  /** The decoded block the segment was taken from, if any. */
  blockId?: string
}

/** Which samples the timeline shows, and the segment the user marked. */
export const useViewStore = defineStore('view', () => {
  const capture = useCaptureStore()
  const viewport = shallowRef<Viewport>({ start: 0, samplesPerPixel: 1, width: 0 })
  const segment = shallowRef<Segment | null>(null)
  /** Refit to the whole capture when the width changes, until the user zooms or pans. */
  let autoFit = true

  const end = computed(() => viewportEnd(viewport.value))
  const ruler = computed(() =>
    rulerTicks(viewport.value, {
      samplerate: capture.samplerate,
      triggerSample: capture.triggerSample,
    }),
  )

  function update(next: Viewport, fitting = false): void {
    autoFit = fitting
    viewport.value = next
  }

  function setWidth(width: number): void {
    if (width === viewport.value.width) return
    viewport.value = autoFit
      ? fitViewport(width, capture.sampleCount)
      : clampViewport({ ...viewport.value, width }, capture.sampleCount)
  }

  function fit(): void {
    update(fitViewport(viewport.value.width, capture.sampleCount), true)
  }

  /** Zooms by `factor` around `anchorX` (the middle of the track by default). */
  function zoom(factor: number, anchorX = viewport.value.width / 2): void {
    update(zoomViewport(viewport.value, factor, anchorX, capture.sampleCount))
  }

  function pan(dx: number): void {
    update(panViewport(viewport.value, dx, capture.sampleCount))
  }

  function showRange(from: number, to: number): void {
    update(rangeViewport(viewport.value.width, from, to, capture.sampleCount))
  }

  function centerOn(sample: number): void {
    const half = (viewport.value.width * viewport.value.samplesPerPixel) / 2
    update(clampViewport({ ...viewport.value, start: sample - half }, capture.sampleCount))
  }

  function setSegment(next: Segment | null): void {
    segment.value = next
  }

  // A new capture starts fully zoomed out without a segment.
  watch(
    () => capture.summary?.id,
    () => {
      segment.value = null
      fit()
    },
  )

  return {
    viewport,
    end,
    ruler,
    segment,
    setWidth,
    fit,
    zoom,
    pan,
    showRange,
    centerOn,
    setSegment,
  }
})
