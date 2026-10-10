import { createPinia, setActivePinia } from 'pinia'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { nextTick } from 'vue'

import type { CaptureRequest } from '@/ipc/bindings/CaptureRequest'
import type { CaptureSummary } from '@/ipc/bindings/CaptureSummary'
import type { DeviceState } from '@/ipc/bindings/DeviceState'
import type { LiveStatus } from '@/ipc/bindings/LiveStatus'

import { useCaptureStore } from './capture'
import { useDeviceStore } from './device'
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
  },
  channels: [
    { index: 0, name: '0' },
    { index: 1, name: '1' },
  ],
  decoders: null,
}

const requests: CaptureRequest[] = []
let onLive: ((status: LiveStatus) => void) | null = null

vi.mock('@/ipc/backend', () => ({
  getBackend: () =>
    Promise.resolve({
      captureSummary: () => Promise.resolve(summary),
      captureStart: (request: CaptureRequest) => {
        requests.push(request)
        return Promise.resolve(summary)
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
    },
    options: [],
    channelCount: 16,
    enabledChannels: [0, 1],
    capturing: false,
    trigger: { maxPosition: 1, advanced: false },
    canLoop,
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

describe('capture store', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
    requests.length = 0
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
})
