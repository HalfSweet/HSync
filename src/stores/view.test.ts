import { createPinia, setActivePinia } from 'pinia'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { nextTick } from 'vue'

import type { CaptureSummary } from '@/ipc/bindings/CaptureSummary'
import type { LiveStatus } from '@/ipc/bindings/LiveStatus'
import { panViewport, xToSample, zoomViewport } from '@/timeline/viewport'

import { useCaptureStore } from './capture'
import { usePreferencesStore } from './preferences'
import { useViewStore } from './view'

vi.mock('@/ipc/backend', () => ({
  getBackend: () => Promise.resolve({}),
}))

/** The glide's duration (`--motion-view`), zero with reduced motion. */
const motion = vi.hoisted(() => ({ view: 100 }))
vi.mock('@/motion/timing', () => ({ motionDuration: () => motion.view }))

function summary(
  id: number,
  kind: 'device' | 'demo',
  samplerate: number,
  sampleCount: number,
  triggerSample: number | null = null,
): CaptureSummary {
  return {
    id,
    source: kind === 'device' ? { kind, driver: 'virtual-demo', model: 'Demo' } : { kind },
    info: {
      samplerate,
      sampleCount,
      triggerSample,
      triggerTimeMs: null,
      channels: [0],
      analog: [],
    },
    channels: [{ index: 0, name: '0' }],
    analog: [],
    decoders: null,
    session: null,
  }
}

function live(captureId: number, samples: number, limit: number): LiveStatus {
  return {
    captureId,
    phase: 'capturing',
    samples,
    limit,
    triggerSample: null,
    hardwareTriggered: false,
    hardwareSamples: null,
    overflow: false,
    dropped: 0,
    end: null,
    error: null,
  }
}

describe('view store', () => {
  beforeEach(() => {
    localStorage.clear()
    setActivePinia(createPinia())
  })

  it('keeps the zoom in time from one device capture to the next', async () => {
    const capture = useCaptureStore()
    const view = useViewStore()
    capture.summary = summary(1, 'device', 1_000_000, 100_000)
    await nextTick()
    view.setWidth(1000)
    view.zoom(4)
    view.centerOn(50_000)
    const before = view.viewport
    // Ten times the rate: ten times the samples per pixel, at the same time.
    capture.summary = summary(2, 'device', 10_000_000, 1_000_000)
    await nextTick()
    expect(view.viewport.samplesPerPixel).toBeCloseTo(before.samplesPerPixel * 10)
    expect(view.viewport.start).toBeCloseTo(before.start * 10)

    // A file or the demo starts fully zoomed out.
    capture.summary = summary(3, 'demo', 10_000_000, 1_000_000)
    await nextTick()
    expect(view.viewport.samplesPerPixel).toBeCloseTo(1000)
  })

  it('centres the trigger once found, unless fitted to the whole capture', async () => {
    const capture = useCaptureStore()
    const view = useViewStore()
    capture.summary = summary(1, 'device', 1_000_000, 100_000)
    await nextTick()
    view.setWidth(1000)
    view.zoom(10)
    capture.summary = summary(2, 'device', 1_000_000, 100_000, 70_000)
    await nextTick()
    const middle = view.viewport.start + (view.viewport.width * view.viewport.samplesPerPixel) / 2
    expect(middle).toBeCloseTo(70_000)

    usePreferencesStore().centerTrigger = false
    view.showRange(0, 1000)
    capture.summary = summary(3, 'device', 1_000_000, 100_000, 30_000)
    await nextTick()
    expect(view.viewport.start).toBe(0)
  })

  it('drops the segment and the edge measurement with a new capture', async () => {
    const capture = useCaptureStore()
    const view = useViewStore()
    capture.summary = summary(1, 'device', 1_000_000, 100_000)
    await nextTick()
    const edge = { sample: 10, rowKey: 'd0', edge: true }
    view.setSegment({ from: 10, to: 20 })
    view.setMeasure({ phase: 'done', from: edge, to: { ...edge, sample: 30 } })
    capture.summary = summary(2, 'device', 1_000_000, 100_000)
    await nextTick()
    expect(view.segment).toBeNull()
    expect(view.measure).toBeNull()
  })

  it('follows the newest samples while they stream in', async () => {
    const capture = useCaptureStore()
    const view = useViewStore()
    capture.summary = summary(1, 'device', 1_000_000, 0)
    capture.live = live(1, 0, 100_000)
    await nextTick()
    view.setWidth(1000)
    view.showRange(0, 10_000)
    capture.summary = { ...capture.summary, info: { ...capture.summary.info, sampleCount: 40_000 } }
    await nextTick()
    expect(view.end).toBeCloseTo(40_000)

    usePreferencesStore().followLatest = false
    capture.summary = { ...capture.summary, info: { ...capture.summary.info, sampleCount: 60_000 } }
    await nextTick()
    expect(view.end).toBeCloseTo(40_000)
  })
})

describe('view glides', () => {
  beforeEach(() => {
    localStorage.clear()
    setActivePinia(createPinia())
    vi.useFakeTimers({ toFake: ['requestAnimationFrame', 'cancelAnimationFrame', 'performance'] })
    motion.view = 100
  })
  afterEach(() => {
    vi.useRealTimers()
  })

  /** A million samples fitted to 1000 pixels. */
  async function fitted() {
    const capture = useCaptureStore()
    const view = useViewStore()
    capture.summary = summary(1, 'demo', 1_000_000, 1_000_000)
    await nextTick()
    view.setWidth(1000)
    return view
  }

  /** Runs `count` animation frames of 16 ms. */
  async function frames(count: number): Promise<void> {
    for (let i = 0; i < count; i++) {
      vi.advanceTimersToNextFrame()
      for (let j = 0; j < 10; j++) await Promise.resolve()
    }
  }

  it('glides a zoom over 100 ms, keeping the sample under the pointer still', async () => {
    const view = await fitted()
    const anchor = xToSample(view.viewport, 250)
    view.glideZoom(4, 250)
    expect(view.viewport.samplesPerPixel).toBe(1000)
    const steps: number[] = []
    for (let i = 0; i < 8; i++) {
      await frames(1)
      steps.push(view.viewport.samplesPerPixel)
      expect(xToSample(view.viewport, 250)).toBeCloseTo(anchor, 6)
    }
    // The first frame already moves, and the moves get smaller until the view rests at 4×.
    expect(steps[0]).toBeLessThan(1000)
    const moves = steps.map((step, i) => Math.log((i === 0 ? 1000 : (steps[i - 1] ?? 0)) / step))
    for (let i = 1; i < 6; i++) expect(moves[i]).toBeLessThan(moves[i - 1] ?? 0)
    expect(steps.at(-1)).toBe(250)
    expect(steps.indexOf(250)).toBeLessThanOrEqual(6)
  })

  it('adds a notch turned during a glide to where it is going', async () => {
    const view = await fitted()
    view.glideZoom(Math.SQRT2, 500)
    await frames(2)
    view.glideZoom(Math.SQRT2, 500)
    await frames(2)
    view.glideZoom(Math.SQRT2, 500)
    await frames(10)
    expect(view.viewport.samplesPerPixel).toBeCloseTo(1000 / Math.SQRT2 ** 3)
  })

  it('carries a glide on through a drag', async () => {
    const view = await fitted()
    const start = view.viewport
    view.glideZoom(2, 500)
    await frames(2)
    view.pan(100)
    await frames(10)
    const expected = panViewport(zoomViewport(start, 2, 500, 1_000_000), 100, 1_000_000)
    expect(view.viewport.start).toBeCloseTo(expected.start)
    expect(view.viewport.samplesPerPixel).toBeCloseTo(expected.samplesPerPixel)
  })

  it('pans by keys to where the glide is going', async () => {
    const view = await fitted()
    view.zoom(10)
    const start = view.viewport
    view.glidePan(100)
    await frames(1)
    view.glidePan(100)
    await frames(10)
    expect(view.viewport.start).toBeCloseTo(start.start + 200 * start.samplesPerPixel)
  })

  it('ends a glide on a jump', async () => {
    const view = await fitted()
    view.glideZoom(4)
    await frames(2)
    view.fit()
    await frames(10)
    expect(view.viewport.samplesPerPixel).toBe(1000)
  })

  it('jumps at once with reduced motion', async () => {
    motion.view = 0
    const view = await fitted()
    view.glideZoom(2)
    expect(view.viewport.samplesPerPixel).toBe(500)
  })
})
