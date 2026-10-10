import { defineStore } from 'pinia'
import { computed, shallowRef } from 'vue'

import { getBackend } from '@/ipc/backend'
import type { CaptureSummary } from '@/ipc/bindings/CaptureSummary'
import { toUiError, type UiError } from '@/ipc/errors'

export type CaptureStatus = 'idle' | 'loading' | 'ready' | 'error'

/** The open capture: what it is, and how to place its samples in time. */
export const useCaptureStore = defineStore('capture', () => {
  const summary = shallowRef<CaptureSummary | null>(null)
  const status = shallowRef<CaptureStatus>('idle')
  const error = shallowRef<UiError | null>(null)

  const samplerate = computed(() => summary.value?.info.samplerate ?? 0)
  const sampleCount = computed(() => summary.value?.info.sampleCount ?? 0)
  /** Time zero of the timeline: the trigger, or the first sample without one. */
  const triggerSample = computed(() => summary.value?.info.triggerSample ?? 0)
  const triggered = computed(() => summary.value?.info.triggerSample != null)
  const duration = computed(() => (samplerate.value > 0 ? sampleCount.value / samplerate.value : 0))
  const channelNames = computed(
    () => new Map(summary.value?.channels.map((channel) => [channel.index, channel.name]) ?? []),
  )

  /** Seconds from the trigger to `sample`. */
  function timeOf(sample: number): number {
    return samplerate.value > 0 ? (sample - triggerSample.value) / samplerate.value : 0
  }

  /** Sample at `seconds` from the trigger. */
  function sampleAt(seconds: number): number {
    return triggerSample.value + seconds * samplerate.value
  }

  async function run(load: () => Promise<CaptureSummary | null>): Promise<void> {
    status.value = 'loading'
    error.value = null
    try {
      const loaded = await load()
      summary.value = loaded
      status.value = loaded ? 'ready' : 'idle'
    } catch (reason) {
      error.value = toUiError(reason)
      status.value = 'error'
    }
  }

  /** Shows the capture the backend already holds, or opens the demo capture. */
  function init(): Promise<void> {
    return run(async () => {
      const backend = await getBackend()
      return (await backend.captureSummary()) ?? (await backend.openDemo())
    })
  }

  function openDemo(): Promise<void> {
    return run(async () => (await getBackend()).openDemo())
  }

  function openFile(path: string): Promise<void> {
    return run(async () => (await getBackend()).openFile(path))
  }

  return {
    summary,
    status,
    error,
    samplerate,
    sampleCount,
    triggerSample,
    triggered,
    duration,
    channelNames,
    timeOf,
    sampleAt,
    init,
    openDemo,
    openFile,
  }
})
