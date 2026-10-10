import { useWindowSize } from '@vueuse/core'
import { nextTick, shallowRef, watch, type ShallowRef } from 'vue'

import { usePreferencesStore } from '@/stores/preferences'
import { COMFORTABLE_METRICS, type RowMetrics } from '@/timeline/layout'

export interface TimelineMetrics extends RowMetrics {
  /** Width of the label column (`--lab`). */
  label: number
  /** Height of the ruler (`--h-ruler`). */
  ruler: number
}

const FALLBACK: TimelineMetrics = { ...COMFORTABLE_METRICS, label: 204, ruler: 38 }

function read(): TimelineMetrics {
  const style = getComputedStyle(document.documentElement)
  const px = (name: string, fallback: number) => {
    const value = Number.parseFloat(style.getPropertyValue(name))
    return Number.isFinite(value) ? value : fallback
  }
  return {
    lane: px('--h-lane', FALLBACK.lane),
    trace: px('--h-tr', FALLBACK.trace),
    traceNarrow: px('--h-trc', FALLBACK.traceNarrow),
    gap: px('--gap', FALLBACK.gap),
    label: px('--lab', FALLBACK.label),
    ruler: px('--h-ruler', FALLBACK.ruler),
  }
}

/**
 * Row heights and the label column width as the density tokens define them, so the canvas uses
 * exactly the sizes CSS gives the DOM rows. Re-read when the density or the window width changes.
 */
export function useRowMetrics(): ShallowRef<TimelineMetrics> {
  const preferences = usePreferencesStore()
  const { width } = useWindowSize()
  const metrics = shallowRef(read())
  watch(
    [() => preferences.density, width],
    async () => {
      await nextTick()
      metrics.value = read()
    },
    { flush: 'post' },
  )
  return metrics
}
