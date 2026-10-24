import { defineStore } from 'pinia'
import { computed, shallowRef, watch } from 'vue'

import { getBackend } from '@/ipc/backend'
import type { EventRow } from '@/ipc/bindings/EventRow'
import { PANEL_WIDTH } from '@/stream/layout'

import { useCaptureStore } from './capture'
import { useDecodeStore } from './decode'
import { useViewStore } from './view'

/** Rows fetched at once. */
const PAGE = 200
/** Rows fetched at once for an export. */
const EXPORT_PAGE = 5000

export type StreamScope = 'all' | 'segment'

/** One stream of one decoder, as the panel lists it under a tab. */
export interface Listing {
  decoder: number
  stream: number
  /** The decoder's label. */
  label: string
  /** The node type whose output the stream is, e.g. `i2c`. */
  kind: string
  /** The name the entry exposed the stream under. */
  name: string
  /** How many streams the decoder has; with several, the tab names the stream too. */
  streams: number
}

/** The data stream panel beside the timeline (DESIGN.md §9). */
export const useStreamStore = defineStore('stream', () => {
  const capture = useCaptureStore()
  const decode = useDecodeStore()
  const view = useViewStore()

  const open = shallowRef(false)
  /** Position of the decoder listed. */
  const decoder = shallowRef<number | null>(null)
  /** Position of the stream listed among the decoder's. */
  const stream = shallowRef(0)
  const scope = shallowRef<StreamScope>('all')
  /** The width asked for the panel, in CSS pixels; the timeline keeps the rest. */
  const width = shallowRef(PANEL_WIDTH)
  /** The row under the pointer, marked on the overview and the timeline. */
  const hot = shallowRef<{ start: number; end: number } | null>(null)

  const total = shallowRef(0)
  /** Events of the whole stream listed. */
  const events = shallowRef(0)
  const inside = shallowRef<number | null>(null)
  const pages = shallowRef(new Map<number, EventRow[]>())

  /** Every stream of every decoder, in timeline order. */
  const listings = computed<Listing[]>(() =>
    decode.statuses.flatMap((status) =>
      status.streams.map((one) => ({
        decoder: status.index,
        stream: one.index,
        label: status.label,
        kind: one.kind,
        name: one.name,
        streams: status.streams.length,
      })),
    ),
  )
  const current = computed(
    () =>
      listings.value.find((one) => one.decoder === decoder.value && one.stream === stream.value) ??
      null,
  )

  /** The marked segment in ascending order, when it spans samples. */
  const segment = computed<[number, number] | null>(() => {
    const marked = view.segment
    if (!marked || marked.from === marked.to) return null
    return [Math.min(marked.from, marked.to), Math.max(marked.from, marked.to)]
  })
  const onlySegment = computed(() => scope.value === 'segment' && segment.value !== null)

  // A listing that disappears (another capture) gives way to the first one.
  watch(listings, (list) => {
    if (list.length > 0 && !current.value) {
      decoder.value = list[0]?.decoder ?? null
      stream.value = list[0]?.stream ?? 0
    }
  })

  const request = computed(() => {
    const id = capture.summary?.id
    if (!open.value || id === undefined || decoder.value === null) return null
    return {
      captureId: id,
      index: decoder.value,
      stream: stream.value,
      segment: segment.value,
      onlySegment: onlySegment.value,
      revision: decode.revision,
    }
  })

  let generation = 0
  const loading = new Set<number>()
  /** Pages of the whole stream fetched before the segment moved or more was decoded: they still
   * hold, so they stay listed until their page comes again, and the list keeps its place. */
  let held = new Map<number, EventRow[]>()

  async function load(page: number): Promise<void> {
    const query = request.value
    if (!query || pages.value.has(page) || loading.has(page)) return
    const mine = generation
    loading.add(page)
    try {
      const { revision: _, ...rest } = query
      const found = await (
        await getBackend()
      ).decodeStream({ ...rest, offset: page * PAGE, count: PAGE })
      if (mine !== generation || !found) return
      total.value = found.total
      events.value = found.events
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
    (next, previous) => {
      generation += 1
      loading.clear()
      const same =
        next !== null &&
        previous != null &&
        !next.onlySegment &&
        !previous.onlySegment &&
        next.captureId === previous.captureId &&
        next.index === previous.index &&
        next.stream === previous.stream
      held = same ? new Map([...held, ...pages.value]) : new Map()
      pages.value = new Map()
      if (!same) {
        total.value = 0
        events.value = 0
        inside.value = null
      }
      void load(0)
    },
    { immediate: true },
  )

  /** Row `index` of the scope, fetching its page when needed. */
  function row(index: number): EventRow | undefined {
    const page = Math.floor(index / PAGE)
    const rows = pages.value.get(page)
    if (!rows) {
      void load(page)
      return held.get(page)?.[index - page * PAGE]
    }
    return rows[index - page * PAGE]
  }

  /** Where the event at `index` of the stream is listed, when its page is at hand. */
  function positionOf(index: number): number | null {
    for (const [page, rows] of [...pages.value, ...held]) {
      const at = rows.findIndex((one) => one.index === index)
      if (at >= 0) return page * PAGE + at
    }
    return null
  }

  /** Every row of the scope, for copying. */
  async function allRows(): Promise<EventRow[]> {
    const pagesNeeded = Math.ceil(total.value / PAGE)
    await Promise.all(Array.from({ length: pagesNeeded }, (_, page) => load(page)))
    return Array.from({ length: total.value }, (_, index) => row(index)).filter(
      (one): one is EventRow => one !== undefined,
    )
  }

  /** Every row of the scope, fetched in large pages for exporting. */
  async function fetchAll(): Promise<EventRow[]> {
    const query = request.value
    if (!query) return []
    const { revision: _, ...rest } = query
    const backend = await getBackend()
    const rows: EventRow[] = []
    for (;;) {
      const page = await backend.decodeStream({ ...rest, offset: rows.length, count: EXPORT_PAGE })
      if (!page || page.rows.length === 0) break
      rows.push(...page.rows)
      if (rows.length >= page.total) break
    }
    return rows
  }

  /** Opens the panel on a decoder's stream, or closes it when it already shows that one. */
  function show(index: number, streamIndex = 0): void {
    if (open.value && decoder.value === index && stream.value === streamIndex) {
      open.value = false
      return
    }
    decoder.value = index
    stream.value = streamIndex
    open.value = true
  }

  function toggle(): void {
    open.value = !open.value
    if (open.value && decoder.value === null) {
      decoder.value = listings.value[0]?.decoder ?? null
      stream.value = listings.value[0]?.stream ?? 0
    }
  }

  /** Switches to a decoder's stream without opening the panel, as clicking a block does. */
  function select(index: number, streamIndex = 0): void {
    decoder.value = index
    stream.value = streamIndex
  }

  return {
    open,
    decoder,
    stream,
    scope,
    width,
    hot,
    total,
    events,
    inside,
    pages,
    listings,
    current,
    segment,
    onlySegment,
    row,
    positionOf,
    allRows,
    fetchAll,
    show,
    toggle,
    select,
  }
})
