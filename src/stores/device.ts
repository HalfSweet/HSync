import { defineStore } from 'pinia'
import { computed, shallowRef } from 'vue'

import { getBackend } from '@/ipc/backend'
import type { DeviceState } from '@/ipc/bindings/DeviceState'
import type { DeviceSummary } from '@/ipc/bindings/DeviceSummary'
import type { OptionKey } from '@/ipc/bindings/OptionKey'
import type { OptionState } from '@/ipc/bindings/OptionState'
import type { OptionValue } from '@/ipc/bindings/OptionValue'
import { toUiError, type UiError } from '@/ipc/errors'

/** Driver of the demo device. */
export const DEMO_DRIVER = 'virtual-demo'

/**
 * How long a device that went away may take to come back as itself before HSync falls back to
 * the demo device (DESIGN.md §4.2; DSView waits 500 ms, macOS re-enumerates more slowly).
 */
export const RECONNECT_MS = 2000
const RECONNECT_POLL_MS = 200

/** The value of an integer option, if the device has it. */
function integer(option: OptionState | undefined): number | null {
  return option?.value.type === 'integer' ? option.value.value : null
}

/** Devices found, the open one and its settings (DESIGN.md §4.9, §9.4). */
export const useDeviceStore = defineStore('device', () => {
  const devices = shallowRef<DeviceSummary[]>([])
  const state = shallowRef<DeviceState | null>(null)
  /** Hardware plugged in while the demo device was open, offered as a switch. */
  const arrival = shallowRef<DeviceSummary | null>(null)
  /** Id of the device being opened; firmware uploads make this take seconds. */
  const opening = shallowRef<string | null>(null)
  const error = shallowRef<UiError | null>(null)

  const option = (key: OptionKey) => state.value?.options.find((candidate) => candidate.key === key)
  const samplerate = computed(() => integer(option('samplerate')) ?? 0)
  const sampleLimit = computed(() => integer(option('sampleLimit')) ?? 0)
  const enabledChannels = computed(() => state.value?.enabledChannels ?? [])
  const isDemo = computed(() => state.value?.info.driver === DEMO_DRIVER)

  async function attempt(action: () => Promise<DeviceState>): Promise<boolean> {
    error.value = null
    try {
      state.value = await action()
      return true
    } catch (reason) {
      error.value = toUiError(reason)
      return false
    }
  }

  async function refresh(): Promise<void> {
    try {
      devices.value = await (await getBackend()).deviceList()
    } catch (reason) {
      error.value = toUiError(reason)
    }
  }

  async function open(id: string): Promise<boolean> {
    opening.value = id
    try {
      return await attempt(async () => (await getBackend()).deviceOpen(id))
    } finally {
      opening.value = null
    }
  }

  function setOption(key: OptionKey, value: OptionValue): Promise<boolean> {
    return attempt(async () => (await getBackend()).deviceSetOption(key, value))
  }

  function setChannel(channel: number, enabled: boolean): Promise<boolean> {
    return attempt(async () => (await getBackend()).deviceSetChannel(channel, enabled))
  }

  /**
   * Reopens a device that went away when one of the same model shows up again within
   * {@link RECONNECT_MS} (a USB hiccup, a re-enumeration); its remembered settings come back
   * with it. Otherwise the demo device opens.
   */
  async function reconnect(previous: DeviceState['info'] | null): Promise<void> {
    if (previous && previous.driver !== DEMO_DRIVER) {
      const deadline = Date.now() + RECONNECT_MS
      while (Date.now() < deadline) {
        await new Promise((resolve) => setTimeout(resolve, RECONNECT_POLL_MS))
        if (state.value || opening.value) return
        await refresh()
        const same = devices.value.find(
          (device) =>
            device.driver === previous.driver &&
            device.model === previous.model &&
            (device.status === 'ready' || device.status === 'needsFirmware'),
        )
        if (same && (await open(same.id))) return
      }
    }
    if (!state.value && !opening.value) await openDemo()
  }

  /** Opens the demo device, so there is always something to capture with. */
  async function openDemo(): Promise<void> {
    const demo = devices.value.find(
      (device) => device.driver === DEMO_DRIVER && device.status === 'ready',
    )
    if (demo) await open(demo.id)
  }

  /** A new device list: hardware that just appeared is opened, or offered over the demo. */
  async function receiveDevices(found: DeviceSummary[]): Promise<void> {
    const known = new Set(devices.value.map((device) => device.id))
    devices.value = found
    if (arrival.value && !found.some((device) => device.id === arrival.value?.id))
      arrival.value = null
    const newcomer = found.find(
      (device) =>
        !known.has(device.id) && device.status !== 'unsupported' && device.driver !== DEMO_DRIVER,
    )
    if (!newcomer || newcomer.status === 'speedTooLow') return
    if (!state.value) await open(newcomer.id)
    else if (isDemo.value) arrival.value = newcomer
  }

  /** Takes up the offered hardware. */
  async function acceptArrival(): Promise<void> {
    const offered = arrival.value
    arrival.value = null
    if (offered) await open(offered.id)
  }

  function dismissArrival(): void {
    arrival.value = null
  }

  /** Follows device changes and opens a device: the open one, else hardware, else the demo. */
  async function init(): Promise<void> {
    const backend = await getBackend()
    await backend.onDevices((found) => {
      void receiveDevices(found)
    })
    // The open device went away: wait for it to come back, else fall back to the demo device.
    await backend.onDeviceState((changed) => {
      const previous = state.value?.info ?? null
      state.value = changed
      if (!changed) void reconnect(previous)
    })
    await refresh()
    state.value = await backend.deviceState()
    if (state.value) return
    const ready = devices.value.filter((device) => device.status === 'ready')
    const preferred = ready.find((device) => device.driver !== DEMO_DRIVER) ?? ready[0]
    if (preferred) await open(preferred.id)
  }

  return {
    devices,
    state,
    arrival,
    opening,
    error,
    samplerate,
    sampleLimit,
    enabledChannels,
    isDemo,
    option,
    refresh,
    open,
    setOption,
    setChannel,
    acceptArrival,
    dismissArrival,
    init,
  }
})
