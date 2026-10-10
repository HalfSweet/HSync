import { defineStore } from 'pinia'
import { computed, shallowRef, watch } from 'vue'

import { DECODERS, specFromInfo, type DecoderSpec } from '@/decode/catalog'
import { getBackend } from '@/ipc/backend'
import type { DecoderStatus } from '@/ipc/bindings/DecoderStatus'
import type { DecoderView } from '@/ipc/bindings/DecoderView'
import { viewportEnd } from '@/timeline/viewport'

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

  const request = computed(() => {
    const summary = capture.summary
    const viewport = view.viewport
    if (!summary || viewport.width <= 0) return null
    return {
      captureId: summary.id,
      start: Math.max(0, Math.floor(viewport.start)),
      end: Math.ceil(viewportEnd(viewport)),
      samplesPerPixel: viewport.samplesPerPixel,
      revision: revision.value,
    }
  })

  let sequence = 0
  watch(
    request,
    async (next) => {
      const current = ++sequence
      if (!next) {
        lanes.value = new Map()
        return
      }
      try {
        const found = await (
          await getBackend()
        ).decodeView(next.captureId, next.start, next.end, next.samplesPerPixel)
        if (current === sequence) lanes.value = new Map(found.map((one) => [one.index, one.view]))
      } catch (error) {
        console.error('decode lanes failed', error)
      }
    },
    { immediate: true },
  )

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
