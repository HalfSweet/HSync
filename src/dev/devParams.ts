/**
 * Development-only URL parameters that put the UI into a given state, for comparing it with the
 * design's artboards in screenshots:
 *
 *   ?theme=light|dark  &density=comfortable|compact  &lang=zh-CN|en-US
 *   &view=1860-2600      µs after the trigger shown on the timeline, once a capture is complete
 *   &segment=1910-2450   µs after the trigger marked as A–B, likewise
 *   &trig=0:fall,1:high  trigger conditions by channel index
 *   &heights=0:60,9:90   row heights in CSS pixels by channel index
 *   &autocapture=1       start a capture once a device is open (the app starts without one)
 *   &stream=0            open the data stream on decoder 0
 *   &search=0x4F         search for this and show the first hit
 *   &frame=1440x900      draw a macOS window of this size around the app, for the README screenshots
 *   &update=0.3.0        the mock backend announces this release, found at startup
 *   &import=1            open the import dialog on the waveform the mock backend reads
 *   &import=all          import every signal of it at once, to see the capture it makes
 *   &settings=decoders   open the settings page on this section
 *   &modules=uart,mine   "load" decoder modules of these names (the mock backend fakes them)
 *   &carried=mine        a capture file "brought" decoder modules of these names, likewise
 */
import { nextTick, watch } from 'vue'

import { getBackend } from '@/ipc/backend'
import type { TriggerCondition } from '@/ipc/bindings/TriggerCondition'
import { useCaptureStore } from '@/stores/capture'
import { useChannelsStore } from '@/stores/channels'
import { useDecodeStore } from '@/stores/decode'
import { useDeviceStore } from '@/stores/device'
import { useSearchStore } from '@/stores/search'
import { SETTINGS_SECTIONS, useSettingsStore } from '@/stores/settings'
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

/**
 * Draws a macOS window around the app, `width` × `height`, on a white page: the README
 * screenshots are framed this way. Development only; the colours imitate macOS, not the design.
 */
function frameAsWindow(width: number, height: number, dark: boolean): void {
  const app = document.getElementById('app')
  if (!app?.parentNode) return
  const [bar, left, top, bottom] = [38, 44, 28, 53]
  const chrome = dark ? '#2b2b2e' : '#f2f2f4'
  const text = dark ? '#c9c9cc' : '#3c3c3f'
  const line = dark ? '#1b1b1d' : '#d9d9dc'
  const edge = dark ? 'rgba(0,0,0,0.55)' : 'rgba(0,0,0,0.14)'
  document.documentElement.style.cssText = 'height:auto;background:#fff'
  document.body.style.cssText = `margin:0;position:relative;overflow:hidden;background:#fff;width:${width + 2 * left}px;height:${height + bar + top + bottom}px`
  const window_ = document.createElement('div')
  window_.style.cssText = `position:absolute;left:${left}px;top:${top}px;width:${width}px;height:${height + bar}px;border-radius:10px;overflow:hidden;background:${chrome};box-shadow:0 0 0 1px ${edge},0 22px 48px rgba(0,0,0,0.28),0 4px 12px rgba(0,0,0,0.12)`
  const title = document.createElement('div')
  title.style.cssText = `position:relative;height:${bar}px;border-bottom:1px solid ${line};font:600 13px/${bar}px -apple-system,'SF Pro Text',Helvetica,Arial,sans-serif;color:${text};text-align:center`
  title.textContent = 'HSync'
  ;['#ff5f57', '#febc2e', '#28c840'].forEach((colour, at) => {
    const light = document.createElement('i')
    light.style.cssText = `position:absolute;top:13px;left:${14 + at * 20}px;width:12px;height:12px;border-radius:50%;background:${colour}`
    title.append(light)
  })
  app.parentNode.insertBefore(window_, app)
  window_.append(title, app)
  app.style.cssText = `position:relative;width:${width}px;height:${height}px`
}

export function applyDevParams(search = window.location.search): void {
  const params = new URLSearchParams(search)
  const preferences = usePreferencesStore()
  const frame = /^(\d+)x(\d+)$/.exec(params.get('frame') ?? '')
  if (frame) frameAsWindow(Number(frame[1]), Number(frame[2]), params.get('theme') === 'dark')
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
  const section = oneOf(SETTINGS_SECTIONS, params.get('settings'))
  if (section) {
    const settings = useSettingsStore()
    settings.section = section
    settings.open = true
  }
  for (const name of (params.get('modules') ?? '').split(',').filter(Boolean)) {
    void useDecodeStore().loadModule(`/Users/dev/decoders/${name}.json`, false)
  }
  const carried = (params.get('carried') ?? '').split(',').filter(Boolean)
  if (carried.length > 0) {
    void (async () => {
      // Loaded behind the store's back, as a capture file's modules are, then listed.
      const backend = await getBackend()
      for (const name of carried) {
        await backend.decodeLoadModule(`/Users/dev/captures/${name}.hsync`)
      }
      await useDecodeStore().modulesChanged()
    })()
  }
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
  for (const pair of (params.get('heights') ?? '').split(',').filter(Boolean)) {
    const [index, height] = pair.split(':').map(Number)
    if (Number.isInteger(index) && Number.isFinite(height)) channels.setHeight(index!, height!)
  }

  const streamDecoder = params.get('stream')
  if (streamDecoder !== null && /^\d+$/.test(streamDecoder)) {
    const stream = useStreamStore()
    stream.show(Number(streamDecoder))
  }

  const query = params.get('search')
  if (query) {
    const finder = useSearchStore()
    finder.query = query
    // Once the decoders answer, the first hit in view is shown, as pressing Enter would.
    const stop = watch(
      () => finder.result,
      (result) => {
        if (!result) return
        void nextTick(() => stop())
        void finder.step(true)
      },
    )
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
