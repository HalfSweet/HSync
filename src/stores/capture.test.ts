import { createPinia, setActivePinia } from 'pinia'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { nextTick } from 'vue'

import type { CaptureRequest } from '@/ipc/bindings/CaptureRequest'
import type { CaptureSummary } from '@/ipc/bindings/CaptureSummary'
import type { DeviceState } from '@/ipc/bindings/DeviceState'
import type { LiveStatus } from '@/ipc/bindings/LiveStatus'

import { repeatInterval, useCaptureStore } from './capture'
import { useDeviceStore } from './device'
import { usePreferencesStore } from './preferences'
import type { TriggerRequest } from './trigger'

const summary: CaptureSummary = {
  id: 7,
  source: { kind: 'device', driver: 'virtual-demo', model: 'Demo' },
  info: {
    samplerate: 1_000_000,
    sampleCount: 0,
    triggerSample: null,
    triggerTimeMs: null,
    channels: [0, 1],
    analog: [],
  },
  channels: [
    { index: 0, name: '0' },
    { index: 1, name: '1' },
  ],
  analog: [],
  decoders: null,
  session: null,
}

const requests: CaptureRequest[] = []
let onLive: ((status: LiveStatus) => void) | null = null
/** What the backend shows, and the id of the next capture it starts. */
const backend = { shown: summary as CaptureSummary | null, next: summary.id }

vi.mock('@/ipc/backend', () => ({
  getBackend: () =>
    Promise.resolve({
      captureSummary: () => Promise.resolve(backend.shown),
      captureStart: (request: CaptureRequest) => {
        requests.push(request)
        const started = { ...summary, id: backend.next++ }
        if (!request.hold) backend.shown = started
        return Promise.resolve(started)
      },
      onLive: (handler: (status: LiveStatus) => void) => {
        onLive = handler
        return Promise.resolve(() => undefined)
      },
    }),
}))

const trigger: TriggerRequest = {
  triggers: [],
  triggerPosition: 1,
  instant: false,
  advanced: null,
}

function deviceState(canLoop: boolean): DeviceState {
  return {
    info: {
      id: 'virtual-demo',
      driver: 'virtual-demo',
      vendor: 'HSync',
      model: 'Demo',
      firmware: null,
      usbSpeed: null,
      slowPort: false,
    },
    options: [],
    channelCount: 16,
    enabledChannels: [0, 1],
    capturing: false,
    trigger: { maxPosition: 1, advanced: false, advancedOtherMode: true },
    canLoop,
    hardwareDepth: null,
    rateTiers: [],
    rateTier: null,
    analogInputs: [],
  }
}

function status(patch: Partial<LiveStatus>): LiveStatus {
  return {
    captureId: summary.id,
    phase: 'capturing',
    samples: 4096,
    limit: 4096,
    triggerSample: null,
    hardwareTriggered: false,
    hardwareSamples: null,
    overflow: false,
    dropped: 0,
    end: null,
    error: null,
    ...patch,
  }
}

const settle = () => new Promise((resolve) => setTimeout(resolve, 0))

describe('capture store', () => {
  beforeEach(() => {
    localStorage.clear()
    setActivePinia(createPinia())
    requests.length = 0
    backend.shown = summary
    backend.next = summary.id
  })

  it('starts without a capture until one is taken', async () => {
    backend.shown = null
    const capture = useCaptureStore()
    await capture.init()
    expect(capture.summary).toBeNull()
    expect(capture.status).toBe('idle')
    expect(capture.error).toBeNull()

    useDeviceStore().state = deviceState(true)
    await capture.start(trigger)
    expect(capture.summary?.id).toBe(summary.id)
  })

  it('asks the backend to start a first capture with the preset', async () => {
    const capture = useCaptureStore()
    useDeviceStore().state = deviceState(true)
    await capture.start(trigger)
    expect(requests.at(-1)?.preset).toBeNull()

    const preset = { channelNames: { 0: 'SDA' }, decoders: [] }
    capture.preset = preset
    await capture.start(trigger)
    expect(requests.at(-1)?.preset).toEqual(preset)
  })

  it('shows the capture the backend holds after a reload', async () => {
    const capture = useCaptureStore()
    await capture.init()
    expect(capture.summary?.id).toBe(summary.id)
    expect(capture.status).toBe('ready')
  })

  it('loops only on a device that can', async () => {
    const device = useDeviceStore()
    const capture = useCaptureStore()
    device.state = deviceState(true)
    capture.mode = 'loop'
    await capture.start(trigger)
    expect(requests.at(-1)?.looping).toBe(true)

    capture.mode = 'single'
    await capture.start(trigger)
    expect(requests.at(-1)?.looping).toBe(false)
  })

  it('goes back to single captures when the device can no longer loop', async () => {
    const device = useDeviceStore()
    const capture = useCaptureStore()
    device.state = deviceState(true)
    capture.mode = 'loop'
    await nextTick()
    device.state = deviceState(false)
    await nextTick()
    expect(capture.mode).toBe('single')
    expect(capture.canLoop).toBe(false)
  })

  it('follows a loop capture as it drops samples and its trigger', async () => {
    const capture = useCaptureStore()
    useDeviceStore().state = deviceState(true)
    await capture.init()
    await capture.start(trigger)
    onLive?.(status({ triggerSample: 100 }))
    expect(capture.triggerSample).toBe(100)
    onLive?.(status({ dropped: 1024, triggerSample: null }))
    expect(capture.dropped).toBe(1024)
    expect(capture.triggered).toBe(false)
    expect(capture.extent).toBe(4096)
  })

  it('fills repeats behind the shown capture until they complete', async () => {
    const capture = useCaptureStore()
    useDeviceStore().state = deviceState(true)
    await capture.init()
    capture.mode = 'repeat'
    await capture.start(trigger)
    const first = capture.summary?.id
    expect(requests.at(-1)?.hold).toBe(false)

    await capture.start(trigger, true)
    const second = capture.pending?.id
    expect(requests.at(-1)?.hold).toBe(true)
    expect(capture.summary?.id).toBe(first)
    onLive?.(status({ captureId: second, samples: 1000 }))
    expect(capture.running).toBe(true)
    expect(capture.capturing).toBe(false)
    expect(capture.busy).toBe(true)
    expect(capture.shownLive?.captureId).toBe(second)
    expect(capture.summary?.info.sampleCount).toBe(0)

    // Complete: the backend shows it now, and so does the store.
    backend.shown = { ...summary, id: second ?? -1 }
    onLive?.(status({ captureId: second, phase: 'ended', end: 'completed' }))
    await settle()
    expect(capture.summary?.id).toBe(second)
    expect(capture.pending).toBeNull()
    capture.mode = 'single'
  })

  it('keeps the shown capture when a repeat stopped early is dropped', async () => {
    const capture = useCaptureStore()
    useDeviceStore().state = deviceState(true)
    await capture.init()
    await capture.start(trigger)
    const shown = capture.summary?.id
    usePreferencesStore().keepStopped = false
    await capture.start(trigger, true)
    expect(requests.at(-1)?.keepStopped).toBe(false)
    const dropped = capture.pending?.id
    onLive?.(status({ captureId: dropped, phase: 'ended', end: 'stopped' }))
    await settle()
    expect(capture.summary?.id).toBe(shown)
    expect(capture.pending).toBeNull()
    // The status bar still tells it was stopped.
    expect(capture.shownLive?.end).toBe('stopped')
  })

  it("keeps the repeat interval within DSView's range", () => {
    expect(repeatInterval(0)).toBe(0.1)
    expect(repeatInterval(2.345)).toBe(2.3)
    expect(repeatInterval(60)).toBe(10)
    expect(repeatInterval(Number.NaN)).toBe(1)
    const capture = useCaptureStore()
    expect(capture.interval).toBe(1)
    capture.interval = 0.05
    expect(capture.interval).toBe(0.1)
  })
})
