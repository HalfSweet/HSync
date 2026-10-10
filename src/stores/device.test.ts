import { createPinia, setActivePinia } from 'pinia'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import type { DeviceState } from '@/ipc/bindings/DeviceState'
import type { DeviceSummary } from '@/ipc/bindings/DeviceSummary'

import { RECONNECT_MS, useDeviceStore } from './device'

const demo: DeviceSummary = {
  id: 'virtual-demo',
  driver: 'virtual-demo',
  vendor: 'HSync',
  model: 'Demo',
  status: 'ready',
}
const dslogic: DeviceSummary = {
  id: 'DSLogic:1-2',
  driver: 'DSLogic',
  vendor: 'DreamSourceLab',
  model: 'DSLogic Plus',
  status: 'ready',
}

function stateOf(device: DeviceSummary): DeviceState {
  return {
    info: { ...device, firmware: null, usbSpeed: null, slowPort: false },
    options: [],
    channelCount: 16,
    enabledChannels: [],
    capturing: false,
    trigger: { maxPosition: 90, advanced: false, advancedOtherMode: false },
    canLoop: false,
    hardwareDepth: null,
  }
}

const backend = {
  list: [demo] as DeviceSummary[],
  onDevices: null as ((devices: DeviceSummary[]) => void) | null,
  onDeviceState: null as ((state: DeviceState | null) => void) | null,
  opened: [] as string[],
}

vi.mock('@/ipc/backend', () => ({
  getBackend: () =>
    Promise.resolve({
      deviceList: () => Promise.resolve(backend.list),
      deviceState: () => Promise.resolve(null),
      deviceOpen: (id: string) => {
        backend.opened.push(id)
        const device = backend.list.find((candidate) => candidate.id === id)
        return device ? Promise.resolve(stateOf(device)) : Promise.reject(new Error(id))
      },
      onDevices: (handler: (devices: DeviceSummary[]) => void) => {
        backend.onDevices = handler
        return Promise.resolve(() => undefined)
      },
      onDeviceState: (handler: (state: DeviceState | null) => void) => {
        backend.onDeviceState = handler
        return Promise.resolve(() => undefined)
      },
    }),
}))

const settle = () => new Promise((resolve) => setTimeout(resolve, 0))

describe('device store', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
    backend.list = [demo]
    backend.opened = []
  })

  it('opens hardware before the demo device', async () => {
    backend.list = [demo, dslogic]
    const store = useDeviceStore()
    await store.init()
    expect(store.state?.info.id).toBe(dslogic.id)
  })

  it('offers hardware plugged in while the demo device is open', async () => {
    const store = useDeviceStore()
    await store.init()
    expect(store.state?.info.id).toBe(demo.id)
    backend.list = [demo, dslogic]
    backend.onDevices?.(backend.list)
    await settle()
    expect(store.arrival?.id).toBe(dslogic.id)
    expect(store.state?.info.id).toBe(demo.id)
    await store.acceptArrival()
    expect(store.state?.info.id).toBe(dslogic.id)
    expect(store.arrival).toBeNull()
  })

  it('reopens a device that comes back soon', async () => {
    vi.useFakeTimers()
    backend.list = [demo, dslogic]
    const store = useDeviceStore()
    await store.init()
    backend.list = [demo]
    backend.onDeviceState?.(null)
    await vi.advanceTimersByTimeAsync(RECONNECT_MS / 4)
    expect(store.state).toBeNull()
    backend.list = [demo, { ...dslogic, id: 'DSLogic:1-3' }]
    await vi.advanceTimersByTimeAsync(RECONNECT_MS / 4)
    expect(store.state?.info.id).toBe('DSLogic:1-3')
    vi.useRealTimers()
  })

  it('falls back to the demo device when the open device stays away', async () => {
    vi.useFakeTimers()
    backend.list = [demo, dslogic]
    const store = useDeviceStore()
    await store.init()
    backend.list = [demo]
    backend.onDevices?.(backend.list)
    backend.onDeviceState?.(null)
    await vi.advanceTimersByTimeAsync(RECONNECT_MS + 500)
    vi.useRealTimers()
    expect(store.state?.info.id).toBe(demo.id)
    // Plugged back in with nothing but the demo open: offered again.
    backend.list = [demo, dslogic]
    backend.onDevices?.(backend.list)
    await settle()
    expect(store.arrival?.id).toBe(dslogic.id)
  })

  it('tells once that a USB 3.0 device is on a USB 2.0 port', async () => {
    backend.list = [dslogic]
    const store = useDeviceStore()
    await store.init()
    expect(store.slowPort).toBe(false)
    const state = store.state
    if (!state) throw new Error('no device')
    store.state = { ...state, info: { ...state.info, slowPort: true } }
    expect(store.slowPort).toBe(true)
    store.dismissSlowPort()
    expect(store.slowPort).toBe(false)
  })
})
