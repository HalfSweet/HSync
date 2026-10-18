import { defineStore } from 'pinia'
import { computed, shallowRef, watch } from 'vue'

import { activeCatalog, isEntry, typeOf, type DecoderEntry } from '@/decode/catalog'
import { getBackend } from '@/ipc/backend'
import type { CatalogEntry } from '@/ipc/bindings/CatalogEntry'
import type { DecoderLanes } from '@/ipc/bindings/DecoderLanes'
import type { DecoderStatus } from '@/ipc/bindings/DecoderStatus'
import type { ModuleInfo } from '@/ipc/bindings/ModuleInfo'
import type { StreamView } from '@/ipc/bindings/StreamView'
import { toUiError, type UiError } from '@/ipc/errors'
import { viewportEnd } from '@/timeline/viewport'
import { covers, overscan, type SampleWindow } from '@/timeline/window'

import { useCaptureStore } from './capture'
import { usePreferencesStore } from './preferences'
import { useViewStore } from './view'

/** A decoder module as the settings page lists it: its file, and what loading it gave. */
export interface ModuleEntry {
  path: string
  /** What the backend says of it once loaded. */
  info: ModuleInfo | null
  /** Why it did not load. */
  error: UiError | null
  loading: boolean
}

/** A decoder's progress refreshes what shows this often at most. */
const PROGRESS_MS = 120

/** The decoders of the open capture and what they show in the viewport (DESIGN.md §6.3). */
export const useDecodeStore = defineStore('decode', () => {
  const capture = useCaptureStore()
  const view = useViewStore()
  const preferences = usePreferencesStore()
  const statuses = shallowRef<DecoderStatus[]>([])
  /** Grows whenever a decoder has more to show, so views that depend on results ask again. */
  const revision = shallowRef(0)
  /** Lanes in the viewport by decoder position. */
  const lanes = shallowRef(new Map<number, DecoderLanes>())
  /** The node types the dialog offers: built in, until the backend's catalog is read. */
  const catalog = activeCatalog

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

  /** The decoder modules loaded here, as the settings page lists them (DESIGN.md §11). */
  const modules = shallowRef<ModuleEntry[]>([])
  /** The modules capture files brought: those the backend holds that were not loaded here. */
  const carried = shallowRef<ModuleInfo[]>([])

  function setModule(path: string, patch: Partial<ModuleEntry>): void {
    const at = modules.value.findIndex((entry) => entry.path === path)
    const entry: ModuleEntry = {
      ...(modules.value[at] ?? { path, info: null, error: null, loading: false }),
      ...patch,
    }
    const next = [...modules.value]
    if (at < 0) next.push(entry)
    else next[at] = entry
    modules.value = next
  }

  /** Lists the modules the backend holds; those not loaded here came with a capture file. */
  async function refreshModules(): Promise<void> {
    try {
      const all = await (await getBackend()).decodeModules()
      const own = new Set(modules.value.flatMap((entry) => (entry.info ? [entry.info.name] : [])))
      carried.value = all.filter((info) => !own.has(info.name))
    } catch (error) {
      console.warn('the decoder modules are not available', error)
    }
  }

  /** The modules changed: the catalog follows, as do the lists. */
  function modulesChanged(): Promise<void> {
    return Promise.all([loadCatalog(), refreshModules()]).then(() => undefined)
  }

  /** The capture decodes again with the node types as they are now; its states follow. */
  async function catalogChanged(): Promise<void> {
    await modulesChanged()
    const id = capture.summary?.id
    if (id !== undefined) changed(id, true)
  }

  /**
   * Loads the decoder module at `path` and, when `remember` is set, loads it again at every
   * startup; a module that does not load stays listed with why.
   */
  async function loadModule(path: string, remember = true): Promise<void> {
    setModule(path, { loading: true, error: null })
    try {
      const info = await (await getBackend()).decodeLoadModule(path)
      setModule(path, { info, loading: false })
      if (remember && !preferences.decoderModules.includes(path))
        preferences.decoderModules = [...preferences.decoderModules, path]
      await catalogChanged()
    } catch (error) {
      setModule(path, { error: toUiError(error), loading: false })
    }
  }

  /** Forgets the decoder module at `path`: the decoder built in takes over. */
  async function unloadModule(path: string): Promise<void> {
    const entry = modules.value.find((one) => one.path === path)
    modules.value = modules.value.filter((one) => one.path !== path)
    preferences.decoderModules = preferences.decoderModules.filter((one) => one !== path)
    if (!entry?.info) return
    await unloadNamed(entry.info.name)
  }

  /** Lets go of the module `name` a capture file brought: the decoder built in takes over. */
  function unloadCarried(name: string): Promise<void> {
    return unloadNamed(name)
  }

  async function unloadNamed(name: string): Promise<void> {
    try {
      await (await getBackend()).decodeUnloadModule(name)
      await catalogChanged()
    } catch (error) {
      console.error('unloading a decoder module failed', error)
    }
  }

  let subscribed = false
  /** Follows decoders working in the background, and loads the modules remembered. */
  async function init(): Promise<void> {
    if (subscribed) return
    subscribed = true
    void loadCatalog()
    const backend = await getBackend()
    await backend.onDecoded(({ captureId }) => changed(captureId, true))
    await backend.onDecodeProgress(({ captureId }) => changed(captureId, false))
    await backend.onModulesChanged(() => void modulesChanged())
    // In order, as they were loaded: a later module stands in for an earlier one of its name.
    for (const path of [...preferences.decoderModules]) await loadModule(path, false)
    await refreshModules()
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
    modules,
    carried,
    init,
    loadModule,
    unloadModule,
    unloadCarried,
    modulesChanged,
    statusOf,
    lanesOf,
    streamView,
    typeNamed,
    openEditor,
    closeEditor,
    setEntries,
  }
})
