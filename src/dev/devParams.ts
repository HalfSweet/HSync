/**
 * Development-only URL parameters that put the UI into a given state, for comparing it with the
 * design's artboards in screenshots:
 *
 *   ?theme=light|dark  &density=comfortable|compact  &lang=zh-CN|en-US
 *   &view=1860-2600      µs after the trigger shown on the timeline, once a capture is complete
 *   &segment=1910-2450   µs after the trigger marked as A–B, likewise
 *   &trig=0:fall,1:high  trigger conditions by channel index
 *   &autocapture=1       start a capture once a device is open (the app starts without one)
 *   &stream=0            open the data stream on decoder 0
 *   &update=0.3.0        the mock backend announces this release, found at startup
 *   &import=1            open the import dialog on the waveform the mock backend reads
 *   &import=all          import every signal of it at once, to see the capture it makes
 */
import { nextTick, watch } from 'vue'

import type { TriggerCondition } from '@/ipc/bindings/TriggerCondition'
import { useCaptureStore } from '@/stores/capture'
import { useChannelsStore } from '@/stores/channels'
import { useDeviceStore } from '@/stores/device'
import { useStreamStore } from '@/stores/stream'
import { useTriggerStore } from '@/stores/trigger'
import { useUpdateStore } from '@/stores/update'
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
  // Development builds skip the check at startup; this one finds what the mock announces.
  if (params.has('update')) void useUpdateStore().check(true)
  const imported = params.get('import')
  if (imported === '1' || imported === 'all') {
    void capture.inspectWaveform('sim/counter.vcd').then(() => {
      const signals = capture.pendingImport?.summary.items.flatMap((item) =>
        item.signal === null ? [] : [item.signal],
      )
      if (imported === 'all' && signals) void capture.importWaveform({ signals, samplerate: null })
    })
  }

  for (const pair of (params.get('trig') ?? '').split(',').filter(Boolean)) {
    const [index, condition] = pair.split(':')
    const valid = oneOf(CONDITIONS, condition ?? null)
    if (valid && index !== undefined) channels.setTrigger(Number(index), valid)
  }

  const streamDecoder = params.get('stream')
  if (streamDecoder !== null && /^\d+$/.test(streamDecoder)) {
    const stream = useStreamStore()
    stream.show(Number(streamDecoder))
  }

  if (params.get('autocapture') === '1') {
    const device = useDeviceStore()
    const stop = watch(
      () => device.state,
      (state) => {
        if (!state) return
        void nextTick(() => stop())
        void capture.start(useTriggerStore().request(false))
      },
      { immediate: true },
    )
  }

  const shown = range(params.get('view'))
  const marked = range(params.get('segment'))
  if (!shown && !marked) return
  let applied = false
  // Once the capture is complete: while it fills, the view follows the newest samples.
  const stop = watch(
    [
      () => capture.summary?.id,
      () => capture.sampleCount > 0 && !capture.capturing,
      () => view.viewport.width,
    ],
    ([id, complete, width]) => {
      if (applied || id === undefined || !complete || width <= 0) return
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
