import { defineStore } from 'pinia'
import { computed, shallowRef, watch } from 'vue'

import { DECODERS, specFromInfo, type DecoderSpec } from '@/decode/catalog'
import { getBackend } from '@/ipc/backend'
import type { DecoderStatus } from '@/ipc/bindings/DecoderStatus'
import type { DecoderView } from '@/ipc/bindings/DecoderView'
import { viewportEnd } from '@/timeline/viewport'
import { covers, overscan, type SampleWindow } from '@/timeline/window'

/** How long the view rests before what depends on it is worked out again. */
const REFOCUS_MS = 150

import { useCaptureStore } from './capture'
import { useViewStore } from './view'

/** The decoders of the open capture and what they show in the viewport (DESIGN.md §7, §9.6). */
export const useDecodeStore = defineStore('decode', () => {
  const capture = useCaptureStore()
  const view = useViewStore()
  const statuses = shallowRef<DecoderStatus[]>([])
  /** Grows whenever a decoder finished, so views that depend on results ask again. */
  const revision = shallowRef(0)
  /** Lanes in the viewport by decoder position. */
  const lanes = shallowRef(new Map<number, DecoderView>())

  async function refreshStatuses(captureId: number): Promise<void> {
    try {
      const found = await (await getBackend()).decodeList(captureId)
      if (capture.summary?.id === captureId) statuses.value = found
    } catch (error) {
      console.error('listing decoders failed', error)
    }
  }

  watch(
    () => capture.summary?.id,
    (id) => {
      statuses.value = []
      lanes.value = new Map()
      if (id !== undefined) void refreshStatuses(id)
    },
    { immediate: true },
  )

  /** The decoders the dialog offers: the host's catalog, or the native ones without it. */
  const catalog = shallowRef<readonly DecoderSpec[]>(DECODERS)

  async function loadCatalog(): Promise<void> {
    try {
      const found = await (await getBackend()).decodeCatalog()
      if (found.length > 0) catalog.value = found.map(specFromInfo)
    } catch (error) {
      console.warn('the decoder catalog is not available', error)
    }
  }

  let subscribed = false
  /** Follows decoders finishing in the background. */
  async function init(): Promise<void> {
    if (subscribed) return
    subscribed = true
    void loadCatalog()
    await (
      await getBackend()
    ).onDecoded(({ captureId }) => {
      if (capture.summary?.id !== captureId) return
      void refreshStatuses(captureId)
      revision.value += 1
    })
  }

  /** The visible samples, for the decode lanes. */
  const wanted = computed(() => {
    const summary = capture.summary
    const viewport = view.viewport
    if (!summary || viewport.width <= 0) return null
    return {
      key: `${summary.id}:${revision.value}:${capture.extent}`,
      captureId: summary.id,
      extent: capture.extent,
      window: {
        start: viewport.start,
        end: viewportEnd(viewport),
        samplesPerPixel: viewport.samplesPerPixel,
      },
    }
  })

  // Lanes are fetched a view's width beyond each side, so panning shows what is at hand; one
  // request at a time, the latest wanted once it returns. What depends on the view (SPI offsets,
  // the I²C address, the frames beside an empty view) is worked out for the samples in view, and
  // again once the view has rested after moving within the fetched window.
  let fetched: { key: string; window: SampleWindow; focus: [number, number] } | null = null
  let loading = false
  let again = false
  let refocus: ReturnType<typeof setTimeout> | null = null

  const focusOf = (window: SampleWindow): [number, number] => [
    Math.max(0, Math.floor(window.start)),
    Math.ceil(window.end),
  ]

  async function loadLanes(force = false): Promise<void> {
    const want = wanted.value
    if (!want) {
      lanes.value = new Map()
      fetched = null
      return
    }
    if (!force && fetched?.key === want.key && covers(fetched.window, want.window, want.extent)) {
      const [from, to] = focusOf(want.window)
      if (fetched.focus[0] !== from || fetched.focus[1] !== to) {
        if (refocus !== null) clearTimeout(refocus)
        refocus = setTimeout(() => {
          refocus = null
          void loadLanes(true)
        }, REFOCUS_MS)
      }
      return
    }
    if (loading) {
      again = true
      return
    }
    loading = true
    try {
      const window =
        force && fetched?.key === want.key ? fetched.window : overscan(want.window, want.extent)
      const focus = focusOf(want.window)
      const found = await (
        await getBackend()
      ).decodeView(want.captureId, window.start, window.end, window.samplesPerPixel, focus)
      if (wanted.value?.captureId !== want.captureId) return
      lanes.value = new Map(found.map((one) => [one.index, one.view]))
      fetched = { key: want.key, window, focus }
    } catch (error) {
      console.error('decode lanes failed', error)
    } finally {
      loading = false
      if (again) {
        again = false
        void loadLanes()
      }
    }
  }
  watch(wanted, () => void loadLanes(), { immediate: true })

  function statusOf(decoder: number | null): DecoderStatus | undefined {
    return decoder === null ? undefined : statuses.value.find((one) => one.index === decoder)
  }

  function lanesOf(decoder: number | null): DecoderView | undefined {
    return decoder === null ? undefined : lanes.value.get(decoder)
  }

  /** The decoder dialog: closed, adding (`null`) or editing decoder `index`. */
  const editor = shallowRef<{ index: number | null } | null>(null)

  function openEditor(index: number | null): void {
    editor.value = { index }
  }

  function closeEditor(): void {
    editor.value = null
  }

  /** The capture's decoder list as stored with it. */
  const entries = computed<unknown[]>(() => {
    const decoders = capture.summary?.decoders
    return Array.isArray(decoders) ? (decoders as unknown[]) : []
  })

  /** Replaces the decoder list; the capture decodes again. */
  async function setEntries(next: unknown[]): Promise<void> {
    const id = capture.summary?.id
    if (id === undefined) return
    capture.replaceSummary(await (await getBackend()).decodeSet(id, next))
  }

  return {
    statuses,
    revision,
    lanes,
    editor,
    entries,
    catalog,
    init,
    statusOf,
    lanesOf,
    openEditor,
    closeEditor,
    setEntries,
  }
})
