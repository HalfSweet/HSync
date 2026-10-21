import { defineStore } from 'pinia'
import { computed, shallowRef, watch } from 'vue'

import { onNextFrame } from '@/motion/frames'
import { motionDuration } from '@/motion/timing'
import { glideAt, glideDone, type Glide } from '@/timeline/glide'
import type { ChannelRef } from '@/timeline/groups'
import type { Measure } from '@/timeline/measure'
import { rulerTicks } from '@/timeline/ticks'
import {
  clampViewport,
  fitViewport,
  panViewport,
  rangeViewport,
  viewportEnd,
  xToSample,
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

  /** The glide under way (DESIGN.md §13), if any. */
  let glide: Glide | null = null
  /** Milliseconds between the frames of the last glide. */
  let frameTime = 1000 / 60
  let lastFrame: number | null = null
  let stepQueued = false

  function queueStep(): void {
    if (stepQueued) return
    stepQueued = true
    onNextFrame(stepGlide)
  }

  function stepGlide(now: number): void {
    stepQueued = false
    const current = glide
    if (!current) {
      lastFrame = null
      return
    }
    if (lastFrame !== null) frameTime = Math.min(Math.max(now - lastFrame, 4), 50)
    lastFrame = now
    // Each frame shows where the glide is when the frame appears, a frame after its time, so
    // the first frame already moves.
    if (Number.isNaN(current.start)) current.start = now - frameTime
    viewport.value = glideAt(current, now)
    if (glideDone(current, now)) {
      glide = null
      lastFrame = null
    } else {
      queueStep()
    }
  }

  /** Glides to `target`, from wherever the view is now; at once with reduced motion. */
  function glideTo(target: Viewport): void {
    autoFit = false
    const duration = motionDuration('view')
    if (duration <= 0) {
      glide = null
      viewport.value = target
      return
    }
    glide = { from: viewport.value, to: target, start: Number.NaN, duration }
    queueStep()
  }

  /** Jumps to `next`, ending any glide. */
  function update(next: Viewport, fitting = false): void {
    glide = null
    autoFit = fitting
    viewport.value = next
  }

  /** Changes the view at once; a glide under way carries on from the changed view. */
  function adjust(change: (view: Viewport) => Viewport): void {
    if (glide) glide = { ...glide, from: change(glide.from), to: change(glide.to) }
    viewport.value = change(viewport.value)
  }

  function setWidth(width: number): void {
    if (width === viewport.value.width) return
    if (autoFit) update(fitViewport(width, capture.extent), true)
    else adjust((view) => clampViewport({ ...view, width }, capture.extent))
  }

  function fit(): void {
    update(fitViewport(viewport.value.width, capture.extent), true)
  }

  /** Zooms by `factor` around `anchorX` (the middle of the track by default). */
  function zoom(factor: number, anchorX = viewport.value.width / 2): void {
    autoFit = false
    adjust((view) => zoomViewport(view, factor, anchorX, capture.extent))
  }

  function pan(dx: number): void {
    autoFit = false
    adjust((view) => panViewport(view, dx, capture.extent))
  }

  /**
   * Glides by `factor` around `anchorX` (the middle of the track by default). The sample under
   * it now ends there; during a glide, the zoom adds to where it was going.
   */
  function glideZoom(factor: number, anchorX = viewport.value.width / 2): void {
    const target = glide?.to ?? viewport.value
    const samplesPerPixel = target.samplesPerPixel / factor
    const anchor = xToSample(viewport.value, anchorX)
    glideTo(
      clampViewport(
        { start: anchor - anchorX * samplesPerPixel, samplesPerPixel, width: target.width },
        capture.extent,
      ),
    )
  }

  /** Glides by `dx` pixels of the view it is going to; positive is towards later samples. */
  function glidePan(dx: number): void {
    glideTo(panViewport(glide?.to ?? viewport.value, dx, capture.extent))
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
      const kept = glide?.to ?? viewport.value
      update(
        clampViewport(
          {
            ...kept,
            start: kept.start * scale,
            samplesPerPixel: kept.samplesPerPixel * scale,
          },
          capture.extent,
        ),
      )
    },
  )

  // A capture that ends early shrinks; stay fitted unless the user zoomed or panned.
  watch(
    () => capture.extent,
    () => {
      if (autoFit) fit()
      else adjust((view) => clampViewport(view, capture.extent))
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
      const latest = (view: Viewport) => {
        const start = Math.max(0, capture.sampleCount - view.width * view.samplesPerPixel)
        return start === view.start ? view : clampViewport({ ...view, start }, capture.extent)
      }
      if (glide || latest(viewport.value) !== viewport.value) adjust(latest)
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
    glideZoom,
    glidePan,
    showRange,
    centerOn,
    setSegment,
    setMeasure,
    setReadouts,
  }
})
