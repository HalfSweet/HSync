import { defineStore } from 'pinia'
import { computed, shallowRef, watch } from 'vue'

import { getBackend } from '@/ipc/backend'
import type { StreamCounts } from '@/ipc/bindings/StreamCounts'
import type { StreamMode } from '@/ipc/bindings/StreamMode'
import type { StreamRow } from '@/ipc/bindings/StreamRow'

import { useCaptureStore } from './capture'
import { useDecodeStore } from './decode'
import { useViewStore } from './view'

/** Rows fetched at once. */
const PAGE = 200
/** Rows fetched at once for an export. */
const EXPORT_PAGE = 5000

export type StreamScope = 'all' | 'segment'

/** The data stream panel under the timeline (DESIGN.md §9). */
export const useStreamStore = defineStore('stream', () => {
  const capture = useCaptureStore()
  const decode = useDecodeStore()
  const view = useViewStore()

  const open = shallowRef(false)
  /** Position of the decoder listed. */
  const decoder = shallowRef<number | null>(null)
  const mode = shallowRef<StreamMode>('text')
  const scope = shallowRef<StreamScope>('all')
  /** Panel height in CSS pixels; the timeline keeps the rest. */
  const height = shallowRef(260)
  /** The row under the pointer, marked on the overview and the timeline. */
  const hot = shallowRef<{ start: number; end: number } | null>(null)

  const total = shallowRef(0)
  const counts = shallowRef<StreamCounts | null>(null)
  const inside = shallowRef<number | null>(null)
  const pages = shallowRef(new Map<number, StreamRow[]>())

  /** Decoders with a native presentation, in timeline order. */
  const decoders = computed(() => decode.statuses.filter((status) => status.bus !== null))
  const bus = computed(() => decode.statusOf(decoder.value)?.bus ?? null)
  /** Text and hex only differ for UART; SPI lists hex only. */
  const hasModes = computed(() => bus.value === 'uart')
  const effectiveMode = computed<StreamMode>(() => (hasModes.value ? mode.value : 'text'))

  /** The marked segment in ascending order, when it spans samples. */
  const segment = computed<[number, number] | null>(() => {
    const marked = view.segment
    if (!marked || marked.from === marked.to) return null
    return [Math.min(marked.from, marked.to), Math.max(marked.from, marked.to)]
  })
  const onlySegment = computed(() => scope.value === 'segment' && segment.value !== null)

  // A decoder that disappears (another capture) gives way to the first one.
  watch(decoders, (list) => {
    if (list.length > 0 && !list.some((status) => status.index === decoder.value)) {
      decoder.value = list[0]?.index ?? null
    }
  })

  const request = computed(() => {
    const id = capture.summary?.id
    if (!open.value || id === undefined || decoder.value === null) return null
    return {
      captureId: id,
      index: decoder.value,
      mode: effectiveMode.value,
      segment: segment.value,
      onlySegment: onlySegment.value,
      revision: decode.revision,
    }
  })

  let generation = 0
  const loading = new Set<number>()

  async function load(page: number): Promise<void> {
    const current = request.value
    if (!current || pages.value.has(page) || loading.has(page)) return
    const mine = generation
    loading.add(page)
    try {
      const { revision: _, ...query } = current
      const found = await (
        await getBackend()
      ).decodeStream({ ...query, offset: page * PAGE, count: PAGE })
      if (mine !== generation || !found) return
      total.value = found.total
      counts.value = found.counts
      inside.value = found.inside
      pages.value = new Map(pages.value).set(page, found.rows)
    } catch (error) {
      console.error('data stream failed', error)
    } finally {
      loading.delete(page)
    }
  }

  watch(
    request,
    () => {
      generation += 1
      loading.clear()
      pages.value = new Map()
      total.value = 0
      counts.value = null
      inside.value = null
      void load(0)
    },
    { immediate: true },
  )

  /** Row `index` of the scope, fetching its page when needed. */
  function row(index: number): StreamRow | undefined {
    const page = Math.floor(index / PAGE)
    const rows = pages.value.get(page)
    if (!rows) {
      void load(page)
      return undefined
    }
    return rows[index - page * PAGE]
  }

  /** Every row of the scope, for copying. */
  async function allRows(): Promise<StreamRow[]> {
    const pagesNeeded = Math.ceil(total.value / PAGE)
    await Promise.all(Array.from({ length: pagesNeeded }, (_, page) => load(page)))
    return Array.from({ length: total.value }, (_, index) => row(index)).filter(
      (one): one is StreamRow => one !== undefined,
    )
  }

  /** Every row of the scope, fetched in large pages for exporting. */
  async function fetchAll(): Promise<StreamRow[]> {
    const current = request.value
    if (!current) return []
    const { revision: _, ...query } = current
    const backend = await getBackend()
    const rows: StreamRow[] = []
    for (;;) {
      const page = await backend.decodeStream({
        ...query,
        offset: rows.length,
        count: EXPORT_PAGE,
      })
      if (!page || page.rows.length === 0) break
      rows.push(...page.rows)
      if (rows.length >= page.total) break
    }
    return rows
  }

  /** Opens the panel on `index`, or closes it when it already shows that decoder. */
  function show(index: number): void {
    if (open.value && decoder.value === index) {
      open.value = false
      return
    }
    decoder.value = index
    open.value = true
  }

  function toggle(): void {
    open.value = !open.value
    if (open.value && decoder.value === null) decoder.value = decoders.value[0]?.index ?? null
  }

  /** Switches to `index` without opening the panel, as clicking a decoded block does. */
  function select(index: number): void {
    decoder.value = index
  }

  return {
    open,
    decoder,
    mode,
    scope,
    height,
    hot,
    total,
    counts,
    inside,
    pages,
    decoders,
    bus,
    hasModes,
    effectiveMode,
    segment,
    onlySegment,
    row,
    allRows,
    fetchAll,
    show,
    toggle,
    select,
  }
})
