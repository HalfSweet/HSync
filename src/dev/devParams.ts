/**
 * Development-only URL parameters that put the UI into a given state, for comparing it with the
 * design's artboards in screenshots:
 *
 *   ?theme=paper|night  &density=comfortable|compact  &lang=zh-CN|en-US
 *   &view=1860-2600      µs after the trigger shown on the timeline
 *   &segment=1910-2450   µs after the trigger marked as A–B
 *   &trig=0:fall,1:high  trigger conditions by channel index
 */
import { nextTick, watch } from 'vue'

import type { TriggerCondition } from '@/ipc/bindings/TriggerCondition'
import { useCaptureStore } from '@/stores/capture'
import { useChannelsStore } from '@/stores/channels'
import { DENSITIES, LOCALE_PREFERENCES, THEMES, usePreferencesStore } from '@/stores/preferences'
import { useViewStore } from '@/stores/view'

const CONDITIONS: TriggerCondition[] = ['none', 'rise', 'fall', 'both', 'high', 'low']

function range(text: string | null): [number, number] | null {
  const match = /^(-?\d+(?:\.\d+)?)-(-?\d+(?:\.\d+)?)$/.exec(text ?? '')
  return match ? [Number(match[1]) * 1e-6, Number(match[2]) * 1e-6] : null
}

function oneOf<T extends string>(allowed: readonly T[], value: string | null): T | null {
  return value !== null && (allowed as readonly string[]).includes(value) ? (value as T) : null
}

export function applyDevParams(search = window.location.search): void {
  const params = new URLSearchParams(search)
  const preferences = usePreferencesStore()
  const capture = useCaptureStore()
  const view = useViewStore()
  const channels = useChannelsStore()

  const theme = oneOf(THEMES, params.get('theme'))
  if (theme) preferences.theme = theme
  const density = oneOf(DENSITIES, params.get('density'))
  if (density) preferences.density = density
  const locale = oneOf(LOCALE_PREFERENCES, params.get('lang'))
  if (locale) preferences.locale = locale

  for (const pair of (params.get('trig') ?? '').split(',').filter(Boolean)) {
    const [index, condition] = pair.split(':')
    const valid = oneOf(CONDITIONS, condition ?? null)
    if (valid && index !== undefined) channels.setTrigger(Number(index), valid)
  }

  const shown = range(params.get('view'))
  const marked = range(params.get('segment'))
  if (!shown && !marked) return
  let applied = false
  const stop = watch(
    [() => capture.summary?.id, () => view.viewport.width],
    ([id, width]) => {
      if (applied || id === undefined || width <= 0) return
      applied = true
      // After the view store has reacted to the new capture (it refits and clears the segment).
      void nextTick(() => {
        stop()
        if (shown) view.showRange(capture.sampleAt(shown[0]), capture.sampleAt(shown[1]))
        if (marked) {
          view.setSegment({
            from: Math.round(capture.sampleAt(marked[0])),
            to: Math.round(capture.sampleAt(marked[1])),
          })
        }
      })
    },
    { immediate: true },
  )
}
