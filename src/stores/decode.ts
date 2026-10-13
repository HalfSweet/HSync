import { defineStore } from 'pinia'
import { computed, shallowRef, watch } from 'vue'

import { DEFAULT_CATALOG, isEntry, typeOf, type DecoderEntry } from '@/decode/catalog'
import { getBackend } from '@/ipc/backend'
import type { CatalogEntry } from '@/ipc/bindings/CatalogEntry'
import type { DecoderLanes } from '@/ipc/bindings/DecoderLanes'
import type { DecoderStatus } from '@/ipc/bindings/DecoderStatus'
import type { StreamView } from '@/ipc/bindings/StreamView'
import { viewportEnd } from '@/timeline/viewport'
import { covers, overscan, type SampleWindow } from '@/timeline/window'

import { useCaptureStore } from './capture'
import { useViewStore } from './view'

/** A decoder's progress refreshes what shows this often at most. */
const PROGRESS_MS = 120

/** The decoders of the open capture and what they show in the viewport (DESIGN.md §6.3). */
export const useDecodeStore = defineStore('decode', () => {
  const capture = useCaptureStore()
  const view = useViewStore()
  const statuses = shallowRef<DecoderStatus[]>([])
  /** Grows whenever a decoder has more to show, so views that depend on results ask again. */
  const revision = shallowRef(0)
  /** Lanes in the viewport by decoder position. */
  const lanes = shallowRef(new Map<number, DecoderLanes>())
  /** The node types the dialog offers: built in, until the backend's catalog is read. */
  const catalog = shallowRef<readonly CatalogEntry[]>(DEFAULT_CATALOG)

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

  async function loadCatalog(): Promise<void> {
    try {
      const found = await (await getBackend()).decodeCatalog()
      if (found.length > 0) catalog.value = found
    } catch (error) {
      console.warn('the decoder catalog is not available', error)
    }
  }

  /** Decoders decode as the capture fills: their progress refreshes the views, throttled. */
  let progressTimer: ReturnType<typeof setTimeout> | null = null
  function changed(captureId: number, now: boolean): void {
    if (capture.summary?.id !== captureId) return
    if (now) {
      if (progressTimer !== null) clearTimeout(progressTimer)
      progressTimer = null
      void refreshStatuses(captureId)
      revision.value += 1
      return
    }
    if (progressTimer !== null) return
    progressTimer = setTimeout(() => {
      progressTimer = null
      if (capture.summary?.id !== captureId) return
      void refreshStatuses(captureId)
      revision.value += 1
    }, PROGRESS_MS)
  }

  let subscribed = false
  /** Follows decoders working in the background. */
  async function init(): Promise<void> {
    if (subscribed) return
    subscribed = true
    void loadCatalog()
    const backend = await getBackend()
    await backend.onDecoded(({ captureId }) => changed(captureId, true))
    await backend.onDecodeProgress(({ captureId }) => changed(captureId, false))
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
  // request at a time, the latest wanted once it returns.
  let fetched: { key: string; window: SampleWindow } | null = null
  let loading = false
  let again = false

  async function loadLanes(): Promise<void> {
    const want = wanted.value
    if (!want) {
      lanes.value = new Map()
      fetched = null
      return
    }
    if (fetched?.key === want.key && covers(fetched.window, want.window, want.extent)) return
    if (loading) {
      again = true
      return
    }
    loading = true
    try {
      const window = overscan(want.window, want.extent)
      const found = await (
        await getBackend()
      ).decodeView(want.captureId, window.start, window.end, window.samplesPerPixel)
      if (wanted.value?.captureId !== want.captureId) return
      lanes.value = new Map(found.map((one) => [one.index, one]))
      fetched = { key: want.key, window }
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

  function lanesOf(decoder: number | null): DecoderLanes | undefined {
    return decoder === null ? undefined : lanes.value.get(decoder)
  }

  /** The lane of stream `stream` of decoder `decoder` in the viewport. */
  function streamView(decoder: number | null, stream: number): StreamView | undefined {
    return lanesOf(decoder)?.streams.find((one) => one.stream === stream)?.view
  }

  /** The node type `name` as the catalog describes it. */
  function typeNamed(name: string): CatalogEntry | undefined {
    return typeOf(name, catalog.value)
  }

  /** The decoder dialog: closed, adding (`null`) or editing decoder `index`. */
  const editor = shallowRef<{ index: number | null } | null>(null)

  function openEditor(index: number | null): void {
    editor.value = { index }
  }

  function closeEditor(): void {
    editor.value = null
  }

  /** The capture's decoder entries as stored with it. */
  const entries = computed<DecoderEntry[]>(() => {
    const decoders = capture.summary?.decoders
    return Array.isArray(decoders) ? decoders.filter(isEntry) : []
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
    streamView,
    typeNamed,
    openEditor,
    closeEditor,
    setEntries,
  }
})
