import { computedAsync } from '@vueuse/core'
import type { Ref } from 'vue'

import { getBackend } from '@/ipc/backend'
import { useCaptureStore } from '@/stores/capture'

/**
 * Frequency of the clock a channel carries, from the median period between its rising edges.
 * Null when the channel never completes a period.
 */
export function useClockFrequency(channel: Ref<number | undefined>): Ref<number | null> {
  const capture = useCaptureStore()
  return computedAsync(async () => {
    const summary = capture.summary
    const index = channel.value
    if (!summary || index === undefined || summary.info.samplerate <= 0) return null
    const period = await (await getBackend()).clockPeriod(summary.id, index)
    return period ? summary.info.samplerate / period : null
  }, null)
}
