import { createPinia, setActivePinia } from 'pinia'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { nextTick } from 'vue'

import type { CaptureSummary } from '@/ipc/bindings/CaptureSummary'
import type { LiveStatus } from '@/ipc/bindings/LiveStatus'

import { useCaptureStore } from './capture'
import { usePreferencesStore } from './preferences'
import { useViewStore } from './view'

vi.mock('@/ipc/backend', () => ({
  getBackend: () => Promise.resolve({}),
}))

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
    info: { samplerate, sampleCount, triggerSample, triggerTimeMs: null, channels: [0] },
    channels: [{ index: 0, name: '0' }],
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
