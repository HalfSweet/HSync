import { defineStore } from 'pinia'
import { computed, shallowRef, watch } from 'vue'

import type { ChannelRef } from '@/timeline/groups'
import type { Measure } from '@/timeline/measure'
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
import { usePreferencesStore } from './preferences'

/** A stretch of time chosen on the ruler or by clicking a decoded block (DESIGN.md §6.6). */
export interface Segment {
  /** First sample (A). */
  from: number
  /** Last sample (B); equal to `from` while the drag has not moved. */
  to: number
  /** The decoded block the segment was taken from, if any. */
  blockId?: string
}

/** What the status bar reads out about the timeline, so that nothing covers the waveforms. */
export interface Readouts {
  /** The pulse under the pointer (DESIGN.md §6.4), with its channel. */
  pulse: { channel: ChannelRef; primary: string; secondary: string | null } | null
  /** The edge measurement (DESIGN.md §6.5); `done` once both edges are chosen. */
  measure: { primary: string | null; secondary: string | null; done: boolean } | null
}

/**
 * Which samples the timeline shows, the segment the user marked, the edges they measure and
 * what the status bar reads out about them.
 */
export const useViewStore = defineStore('view', () => {
  const capture = useCaptureStore()
  const preferences = usePreferencesStore()
  const viewport = shallowRef<Viewport>({ start: 0, samplesPerPixel: 1, width: 0 })
  const segment = shallowRef<Segment | null>(null)
  const measure = shallowRef<Measure | null>(null)
  const readouts = shallowRef<Readouts>({ pulse: null, measure: null })
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
      ? fitViewport(width, capture.extent)
      : clampViewport({ ...viewport.value, width }, capture.extent)
  }

  function fit(): void {
    update(fitViewport(viewport.value.width, capture.extent), true)
  }

  /** Zooms by `factor` around `anchorX` (the middle of the track by default). */
  function zoom(factor: number, anchorX = viewport.value.width / 2): void {
    update(zoomViewport(viewport.value, factor, anchorX, capture.extent))
  }

  function pan(dx: number): void {
    update(panViewport(viewport.value, dx, capture.extent))
  }

  function showRange(from: number, to: number): void {
    update(rangeViewport(viewport.value.width, from, to, capture.extent))
  }

  function centerOn(sample: number): void {
    const half = (viewport.value.width * viewport.value.samplesPerPixel) / 2
    update(clampViewport({ ...viewport.value, start: sample - half }, capture.extent))
  }

  function setSegment(next: Segment | null): void {
    segment.value = next
  }

  function setMeasure(next: Measure | null): void {
    measure.value = next
  }

  function setReadouts(next: Readouts): void {
    readouts.value = next
  }

  /** The shown capture's sample rate, and whether a device took it. */
  let shown: { samplerate: number; device: boolean } | null = null

  // A new capture drops the segment and the measurement. One taken with the device after another
  // keeps the zoom and position in time, as DSView does, so repeated captures stay where the user
  // looks; anything else starts fully zoomed out.
  watch(
    () => capture.summary?.id,
    () => {
      segment.value = null
      measure.value = null
      const previous = shown
      const device = capture.summary?.source.kind === 'device'
      shown = capture.summary ? { samplerate: capture.samplerate, device } : null
      if (autoFit || !device || !previous?.device || previous.samplerate <= 0) {
        fit()
        return
      }
      const scale = capture.samplerate / previous.samplerate
      viewport.value = clampViewport(
        {
          ...viewport.value,
          start: viewport.value.start * scale,
          samplesPerPixel: viewport.value.samplesPerPixel * scale,
        },
        capture.extent,
      )
    },
  )

  // A capture that ends early shrinks; stay fitted unless the user zoomed or panned.
  watch(
    () => capture.extent,
    () => {
      if (autoFit) fit()
      else viewport.value = clampViewport(viewport.value, capture.extent)
    },
  )

  // DSView's "trigger position in the middle": once a capture finds its trigger, centre it,
  // unless the whole capture is in view anyway.
  watch(
    () => [capture.summary?.id, capture.triggered] as const,
    ([id, triggered], [previousId, wasTriggered]) => {
      if (!triggered || (id === previousId && wasTriggered)) return
      if (preferences.centerTrigger && !autoFit) centerOn(capture.triggerSample)
    },
  )

  // DSView's "auto scroll to the latest data": while samples stream in, keep the newest at the
  // right edge, unless the whole capture is in view anyway.
  watch(
    () => [capture.sampleCount, capture.dropped] as const,
    () => {
      if (!capture.capturing || autoFit || !preferences.followLatest) return
      const span = viewport.value.width * viewport.value.samplesPerPixel
      const start = Math.max(0, capture.sampleCount - span)
      if (start !== viewport.value.start) {
        viewport.value = clampViewport({ ...viewport.value, start }, capture.extent)
      }
    },
  )

  return {
    viewport,
    end,
    ruler,
    segment,
    measure,
    readouts,
    setWidth,
    fit,
    zoom,
    pan,
    showRange,
    centerOn,
    setSegment,
    setMeasure,
    setReadouts,
  }
})
