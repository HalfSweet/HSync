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

/** The value of an integer option, if the device has it. */
function integer(option: OptionState | undefined): number | null {
  return option?.value.type === 'integer' ? option.value.value : null
}

/** Devices found, the open one and its settings (DESIGN.md §4.9, §9.4). */
export const useDeviceStore = defineStore('device', () => {
  const devices = shallowRef<DeviceSummary[]>([])
  const state = shallowRef<DeviceState | null>(null)
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

  /** Follows device changes and opens a device: the open one, else hardware, else the demo. */
  async function init(): Promise<void> {
    const backend = await getBackend()
    await backend.onDevices((found) => {
      devices.value = found
    })
    await backend.onDeviceState((changed) => {
      state.value = changed
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
    init,
  }
})
