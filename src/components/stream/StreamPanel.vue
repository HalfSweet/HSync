<script setup lang="ts">
import { onClickOutside, useClipboard, useElementSize, useWindowSize } from '@vueuse/core'
import { computed, nextTick, ref, shallowRef, useTemplateRef, watch } from 'vue'
import { useI18n } from 'vue-i18n'

import AppIcon from '@/components/common/AppIcon.vue'
import { defaultCaptureName, withExtension } from '@/composables/useFileActions'
import { useDecoderCopy } from '@/composables/useDecoderCopy'
import { activeCatalog, typeOf } from '@/decode/catalog'
import { entryFormat } from '@/decode/numberFormat'
import { fieldTexts, isCounter, type Wording } from '@/decode/present'
import { getBackend } from '@/ipc/backend'
import { pickSavePath } from '@/ipc/dialogs'
import type { EventRow } from '@/ipc/bindings/EventRow'
import { vIndicator } from '@/motion/indicator'
import { useCaptureStore } from '@/stores/capture'
import { useDecodeStore } from '@/stores/decode'
import { useSearchStore } from '@/stores/search'
import { useStreamStore, type Listing } from '@/stores/stream'
import { useViewStore } from '@/stores/view'
import { csvLines } from '@/stream/csv'
import { panelWidth, spans } from '@/stream/layout'
import { rowParts, type RowParts } from '@/stream/rows'
import { formatSpan } from '@/utils/units'

/**
 * The data stream beside the timeline (DESIGN.md §9): the events of one decoder's stream, one
 * row each, with the event's kind and its fields as the contract names them, and below them
 * every field of the event selected.
 */
const { t } = useI18n()
const capture = useCaptureStore()
const decode = useDecodeStore()
const stream = useStreamStore()
const view = useViewStore()
const finder = useSearchStore()
const { copy, copied } = useClipboard({ legacy: true })

const ROW = 20
const OVERSCAN = 12

const list = useTemplateRef<HTMLElement>('list')
const { height: listHeight } = useElementSize(list)
const scrollTop = shallowRef(0)

const labels = useDecoderCopy()
const typeName = labels.typeName

/** What decoder `index` is called: its label, else the name of its first node type. */
function decoderName(index: number): string {
  const label = decode.entries[index]?.label?.trim()
  if (label) return label
  const status = decode.statusOf(index)
  const first = status?.kinds[0]
  return first === undefined ? (status?.label ?? '') : typeName(first)
}

/** A listing's name: the decoder, and the stream too when the decoder has several. */
function listingName(listing: Listing): string {
  const decoder = decoderName(listing.decoder)
  if (listing.streams <= 1) return decoder
  // The bus itself goes by the decoder's name; the decoders stacked on it are named too.
  if (listing.stream === 0) return decoder
  return t('stream.name', { decoder, stream: typeName(listing.kind) })
}

const listings = computed(() =>
  stream.listings.map((listing) => ({
    listing,
    key: `${listing.decoder}:${listing.stream}`,
    name: listingName(listing),
  })),
)

function isCurrent(listing: Listing): boolean {
  const current = stream.current
  return (
    current !== null && current.decoder === listing.decoder && current.stream === listing.stream
  )
}

const currentName = computed(() => listings.value.find((one) => isCurrent(one.listing))?.name)

// The decoder menu: every stream of every decoder, the one listed checked.
const pickerOpen = ref(false)
const pickerRoot = useTemplateRef<HTMLElement>('pickerRoot')
const pickerButton = useTemplateRef<HTMLButtonElement>('pickerButton')
const pickerMenu = useTemplateRef<HTMLElement>('pickerMenu')
onClickOutside(pickerRoot, () => (pickerOpen.value = false))

function pickerItems(): HTMLElement[] {
  return [...(pickerMenu.value?.querySelectorAll<HTMLElement>('[role="menuitemradio"]') ?? [])]
}

// The keyboard starts on the stream listed.
watch(pickerOpen, async (open) => {
  if (!open) return
  await nextTick()
  const items = pickerItems()
  const checked = items.find((item) => item.getAttribute('aria-checked') === 'true')
  ;(checked ?? items[0])?.focus({ preventScroll: true })
})

function closePicker(): void {
  pickerOpen.value = false
  pickerButton.value?.focus()
}

function choose(listing: Listing): void {
  stream.select(listing.decoder, listing.stream)
  closePicker()
}

function onPickerKeydown(event: KeyboardEvent): void {
  if (event.key === 'Escape') {
    event.preventDefault()
    closePicker()
    return
  }
  if (event.key !== 'ArrowDown' && event.key !== 'ArrowUp') return
  event.preventDefault()
  const items = pickerItems()
  const at = items.indexOf(document.activeElement as HTMLElement)
  const step = event.key === 'ArrowDown' ? 1 : -1
  items[(at < 0 ? 0 : at + step + items.length) % items.length]?.focus()
}

/** The node type whose events are listed, e.g. `i2c`; it names their kinds and fields. */
const type = computed(() => stream.current?.kind)

/** How the listed decoder's numbers show. */
const format = computed(() =>
  entryFormat(stream.decoder === null ? undefined : decode.entries[stream.decoder]),
)

const wording = computed<Wording>(() => ({
  enumValue: (field, value) =>
    type.value === undefined ? value : labels.valueLabel(type.value, field, value),
  yes: t('decode.yes'),
  no: t('decode.no'),
  none: t('decode.none'),
}))

/** A sample's time in ms with `decimals` decimals, relative to the trigger point. */
function ms(sample: number, decimals: number): string {
  return (capture.timeOf(sample) * 1000).toFixed(decimals)
}

function time(row: EventRow): string {
  return ms(row.start, 4)
}

// The time and kind columns are as wide as their widest text: the heading, the times of the
// capture's first and last samples, and the name of every kind the node type puts out; a kind
// column of long names gives way to the fields.
const timeProbe = useTemplateRef<HTMLElement>('timeProbe')
const kindProbe = useTemplateRef<HTMLElement>('kindProbe')
const { width: timeWidth } = useElementSize(timeProbe)
const { width: kindWidth } = useElementSize(kindProbe)
const extremes = computed(() => [ms(0, 4), ms(capture.extent, 4)])
const events = computed(() =>
  type.value === undefined ? [] : (typeOf(type.value, activeCatalog.value)?.events ?? []),
)
const kinds = computed(() => {
  const name = type.value
  return name === undefined ? [] : events.value.map((event) => labels.eventLabel(name, event.kind))
})

// The number of the transfer each event belongs to has a column of its own when the node type
// numbers its transfers, headed by "#" and named in full in its tip. The column widens to the
// longest number seen in the listing, and never narrows while it lasts, so rows do not shift.
const counter = computed(() => {
  for (const event of events.value) {
    const name = event.fields.find(isCounter)
    if (name !== undefined) return fieldLabel(name)
  }
  return null
})
const counterChars = shallowRef(1)
watch(
  () => `${stream.decoder}:${stream.stream}:${stream.scope}`,
  () => (counterChars.value = 1),
)

const columns = computed(() => ({
  '--time-w': `${Math.ceil(timeWidth.value)}px`,
  '--count-w': `${counterChars.value}ch`,
  '--kind-w': `min(${Math.ceil(kindWidth.value)}px, 36%)`,
}))

/** "9 events", "9 events, 4 in the segment" and the segment's own variant. */
const summary = computed(() => {
  if (!stream.current) return ''
  const total = t('stream.summary.events', { count: stream.events })
  const inside = stream.inside
  const segment = stream.segment
  if (inside === null || !segment) return total
  if (stream.onlySegment) {
    const [from, to] = segment
    return t('stream.summary.segmentOnly', { from: ms(from, 3), to: ms(to, 3), count: inside })
  }
  return t('stream.summary.inSegment', { total, count: inside })
})

/** The search hits in the listed stream by their row: `current` for the one shown. */
const marks = computed(() => {
  const found = new Map<number, 'current' | 'hit'>()
  ;(finder.result?.hits ?? []).forEach((hit, at) => {
    if (hit.decoder !== stream.decoder || hit.stream !== stream.stream) return
    found.set(hit.row, at === finder.current ? 'current' : 'hit')
  })
  return found
})

/** The kind of an event as named, else the kind itself. */
function kindText(row: EventRow): string {
  return type.value === undefined ? row.kind : labels.eventLabel(type.value, row.kind)
}

/** A field's label, else the name itself. */
function fieldLabel(name: string): string {
  return type.value === undefined ? name : labels.eventFieldLabel(type.value, name)
}

/** The fields of an event as copied and exported, "label: value" each, in the contract's order. */
function cellsOf(row: EventRow): string[] {
  return fieldTexts(row.payload, format.value, wording.value).map(({ name, text }) =>
    t('decode.field', { label: fieldLabel(name), value: text }),
  )
}

interface Shown {
  index: number
  /** Not yet there while its page is fetched. */
  row: EventRow | undefined
  find: 'current' | 'hit' | null
  kind: string
  parts: RowParts
}

const NO_PARTS: RowParts = { counter: null, cells: [] }

/** An event's fields as its row shows them. */
function partsOf(row: EventRow, kind: string): RowParts {
  return rowParts(fieldTexts(row.payload, format.value, wording.value), kind, fieldLabel)
}

// Virtual list: only the rows in sight, and a few more.
const visible = computed<Shown[]>(() => {
  const first = Math.max(0, Math.floor(scrollTop.value / ROW) - OVERSCAN)
  const last = Math.min(
    stream.total,
    Math.ceil((scrollTop.value + listHeight.value) / ROW) + OVERSCAN,
  )
  const rows: Shown[] = []
  for (let index = first; index < last; index++) {
    const row = stream.row(index)
    const kind = row ? kindText(row) : ''
    rows.push({
      index,
      row,
      find: row ? (marks.value.get(row.index) ?? null) : null,
      kind,
      parts: row ? partsOf(row, kind) : NO_PARTS,
    })
  }
  return rows
})

watch(visible, (rows) => {
  const widest = Math.max(0, ...rows.map(({ parts }) => parts.counter?.length ?? 0))
  if (widest > counterChars.value) counterChars.value = widest
})

/** Whether an event lies wholly inside the marked segment. */
function inSegment(row: EventRow): boolean {
  const segment = stream.segment
  return segment !== null && row.start >= segment[0] && row.end <= segment[1]
}

function onScroll(): void {
  scrollTop.value = list.value?.scrollTop ?? 0
}

/** The event selected, in the list or as a block on the timeline; its fields show below. */
const picked = shallowRef<EventRow | null>(null)

function isPicked(row: EventRow): boolean {
  return picked.value?.index === row.index
}

/** Selects an event: the traces pan there and the segment spans it. */
function pick(row: EventRow): void {
  picked.value = row
  view.centerOn((row.start + row.end) / 2)
  view.setSegment({ from: row.start, to: row.end })
}

// Another listing, another capture or decoders set up anew let go of the event selected; the
// summary of a running capture changes as it fills, its decoders as they were.
watch(
  [
    () => stream.decoder,
    () => stream.stream,
    () => capture.summary?.id,
    () => JSON.stringify(decode.entries),
  ],
  () => (picked.value = null),
)

/** The marked segment in ascending order, a single sample too. */
function marked(): [number, number] | null {
  const segment = view.segment
  return segment ? [Math.min(segment.from, segment.to), Math.max(segment.from, segment.to)] : null
}

// A segment that no longer spans the event selected lets go of it. A segment marked on the
// timeline scrolls the first row inside it into view, one row below the top, once it stops
// changing; a block clicked there selects its event.
let settle: ReturnType<typeof setTimeout> | undefined
watch(
  () => view.segment,
  (segment) => {
    clearTimeout(settle)
    const range = marked()
    if (picked.value && !spans(picked.value, range)) picked.value = null
    // The list's own selection is in sight already.
    if (!range || picked.value) return
    const block = segment?.blockId !== undefined
    if (block) void pickBlock(range)
    // A segment being dragged out spans no samples until it moves.
    if (stream.onlySegment || (!block && !stream.segment)) return
    settle = setTimeout(() => void scrollToSample(range[0]), block ? 0 : 250)
  },
)

/** Selects the event of the stream listed that spans a clicked block, if there is one. */
async function pickBlock(range: [number, number]): Promise<void> {
  const captureId = capture.summary?.id
  const index = stream.decoder
  if (captureId === undefined || index === null) return
  const backend = await getBackend()
  const at = await backend.decodeRowAt(captureId, index, stream.stream, range[0])
  if (at === null) return
  const page = await backend.decodeStream({
    captureId,
    index,
    stream: stream.stream,
    segment: stream.segment,
    onlySegment: false,
    offset: at,
    count: 8,
  })
  const row = page?.rows.find((one) => spans(one, range))
  if (row && index === stream.decoder && spans(row, marked())) picked.value = row
}

/** ↑ and ↓ select the event before or after, starting from the first in sight. */
function onListKeydown(event: KeyboardEvent): void {
  if (event.key !== 'ArrowDown' && event.key !== 'ArrowUp') return
  if (event.altKey || event.ctrlKey || event.metaKey || event.shiftKey) return
  event.preventDefault()
  const at = picked.value === null ? null : stream.positionOf(picked.value.index)
  const next =
    at === null ? Math.floor(scrollTop.value / ROW) : at + (event.key === 'ArrowDown' ? 1 : -1)
  if (next < 0 || next >= stream.total) return
  const row = stream.row(next)
  if (!row) return
  pick(row)
  reveal(next)
}

/** Scrolls row `at` into sight below the sticky heading. */
function reveal(at: number): void {
  const box = list.value
  if (!box) return
  const top = (at + 1) * ROW
  if (top < box.scrollTop + ROW) box.scrollTop = at * ROW
  else if (top + ROW > box.scrollTop + box.clientHeight)
    box.scrollTop = top + ROW - box.clientHeight
}

/** The event selected as the details show it: its kind, start and length, and every field. */
const details = computed(() => {
  const row = picked.value
  if (!row) return null
  const length = capture.timeOf(row.end) - capture.timeOf(row.start)
  return {
    key: `${row.index}:${row.start}`,
    kind: kindText(row),
    time: time(row),
    length: length > 0 ? formatSpan(length) : null,
    fields: fieldTexts(row.payload, format.value, wording.value).map(({ name, text }) => ({
      name,
      label: fieldLabel(name),
      text,
    })),
  }
})

// The search hit shown scrolls into view, one row below the top, once its stream is listed.
const pendingRow = shallowRef<number | null>(null)
watch(
  () => finder.hit,
  (hit) => (pendingRow.value = hit?.row ?? null),
  { immediate: true },
)
watch(
  () =>
    [pendingRow.value, stream.total, stream.decoder, stream.stream, stream.onlySegment] as const,
  async ([row, total, decoder, streamIndex, onlySegment]) => {
    const hit = finder.hit
    if (
      row === null ||
      !hit ||
      hit.decoder !== decoder ||
      hit.stream !== streamIndex ||
      onlySegment ||
      total <= row
    )
      return
    pendingRow.value = null
    await nextTick()
    const box = list.value
    if (!box) return
    // Rows start below the sticky header, which is one row high.
    const top = (row + 1) * ROW
    const shown = top >= box.scrollTop + ROW && top + ROW <= box.scrollTop + box.clientHeight
    if (!shown) box.scrollTo({ top: Math.max(0, (row - 1) * ROW) })
  },
)

async function scrollToSample(sample: number): Promise<void> {
  const id = capture.summary?.id
  if (id === undefined || stream.decoder === null) return
  const index = await (await getBackend()).decodeRowAt(id, stream.decoder, stream.stream, sample)
  if (index === null) return
  await nextTick()
  list.value?.scrollTo({ top: Math.max(0, (index - 1) * ROW) })
}

/** The decoder and stream names the copy and the exports write beside every row. */
const names = computed(() => {
  const current = stream.current
  if (!current) return { decoder: '', stream: '' }
  return { decoder: decoderName(current.decoder), stream: typeName(current.kind) }
})

/** A row's columns: time, decoder, stream, kind and the fields, two spaces apart. */
function columnsOf(row: EventRow): string[] {
  return [
    time(row),
    names.value.decoder,
    names.value.stream,
    kindText(row),
    cellsOf(row).join('  '),
  ]
}

async function copyRows(): Promise<void> {
  const rows = await stream.allRows()
  await copy(rows.map((row) => columnsOf(row).join('\t')).join('\n'))
}

// Exporting the listing: CSV with time, decoder, stream, event and content columns, or the
// same columns as text, separated by tabs.
const exportMenu = ref(false)
const exportRoot = useTemplateRef<HTMLElement>('exportRoot')
onClickOutside(exportRoot, () => (exportMenu.value = false))

async function exportRows(kind: 'csv' | 'text'): Promise<void> {
  exportMenu.value = false
  const current = stream.current
  const extension = kind === 'csv' ? 'csv' : 'txt'
  // The file is named after the decoder, and its stream when the decoder has several.
  const name = !current
    ? 'stream'
    : current.streams > 1
      ? `${names.value.decoder}-${names.value.stream}`
      : names.value.decoder
  const safeName = name.replace(/[\\/:*?"<>|\s]+/g, '_')
  const suggested = defaultCaptureName(new Date(), extension).replace(
    `.${extension}`,
    `-${safeName}.${extension}`,
  )
  const path = await pickSavePath(suggested, [
    { name: t(`stream.export.${kind}`), extensions: [extension] },
  ])
  if (!path) return
  const lines = (await stream.fetchAll()).map(columnsOf)
  const text =
    kind === 'csv'
      ? csvLines([
          [
            t('stream.export.timeColumn'),
            t('stream.export.decoderColumn'),
            t('stream.export.streamColumn'),
            t('stream.export.kindColumn'),
            t('stream.export.contentColumn'),
          ],
          ...lines,
        ])
      : lines.map((line) => line.join('\t')).join('\n') + '\n'
  await capture.saveText(withExtension(path, extension), text)
}

// Resizing by the left edge; the width asked for stays while the window is too narrow for it.
const { width: room } = useWindowSize()
const width = computed(() => panelWidth(stream.width, room.value))
let grip: { x: number; width: number } | null = null
function onGripDown(event: PointerEvent): void {
  grip = { x: event.clientX, width: width.value }
  ;(event.currentTarget as HTMLElement).setPointerCapture(event.pointerId)
}
function onGripMove(event: PointerEvent): void {
  if (!grip) return
  stream.width = panelWidth(grip.width + grip.x - event.clientX, room.value)
}
function onGripUp(): void {
  grip = null
}
</script>

<template>
  <aside
    class="relative flex-none"
    :style="{ width: `${width}px` }"
    :aria-label="t('stream.title')"
  >
    <div
      class="group absolute inset-y-0 -left-1 z-[6] w-[7px] cursor-col-resize touch-none"
      role="separator"
      aria-orientation="vertical"
      :aria-label="t('stream.resize')"
      :title="t('stream.resizeTip')"
      @pointerdown="onGripDown"
      @pointermove="onGripMove"
      @pointerup="onGripUp"
      @pointercancel="onGripUp"
    >
      <i class="absolute inset-y-0 left-[3px] hidden w-px bg-ink-3 group-hover:block" />
    </div>
    <!-- As wide as the panel, which opens and closes around it (src/motion/drawer.ts). -->
    <div
      class="flex h-full flex-col border-l border-line bg-canvas"
      :style="{ width: `${width}px` }"
    >
      <div
        class="flex h-(--h-overview) flex-none flex-col justify-center gap-1 border-b border-line bg-chrome px-2"
      >
        <div class="flex h-6 items-center gap-2">
          <div ref="pickerRoot" class="relative flex max-w-[65%] min-w-0 flex-none">
            <button
              v-if="listings.length > 1"
              ref="pickerButton"
              class="inline-flex h-6 min-w-0 items-center gap-1 rounded-4 pr-1 pl-1.5 text-small font-medium text-ink hover:bg-hover aria-expanded:bg-hover"
              aria-haspopup="menu"
              :aria-expanded="pickerOpen"
              :title="t('stream.pick')"
              @click="pickerOpen = !pickerOpen"
            >
              <span class="truncate">{{ currentName }}</span>
              <AppIcon name="chevron-down" class="text-ink-2" />
            </button>
            <span v-else class="truncate px-1.5 text-small font-medium">{{ currentName }}</span>
            <Transition name="pop">
              <div
                v-if="pickerOpen"
                ref="pickerMenu"
                role="menu"
                :aria-label="t('stream.decoders')"
                class="absolute top-full left-0 z-20 mt-1 flex w-max max-w-80 min-w-44 origin-top-left flex-col rounded-6 bg-menu p-1 text-small"
                @keydown="onPickerKeydown"
              >
                <button
                  v-for="one in listings"
                  :key="one.key"
                  role="menuitemradio"
                  :aria-checked="isCurrent(one.listing)"
                  class="flex h-[26px] w-full items-center gap-2 rounded-4 px-2 text-left whitespace-nowrap hover:bg-hover"
                  :class="isCurrent(one.listing) && 'font-medium'"
                  @click="choose(one.listing)"
                >
                  <span class="truncate">{{ one.name }}</span>
                  <AppIcon v-if="isCurrent(one.listing)" name="check" class="ml-auto" />
                </button>
              </div>
            </Transition>
          </div>
          <span class="min-w-0 flex-1 truncate text-small text-ink-2">{{ summary }}</span>
          <button
            class="btn-icon flex-none"
            :aria-label="t('stream.close')"
            :title="t('stream.close')"
            @click="stream.open = false"
          >
            <AppIcon name="close" />
          </button>
        </div>
        <div class="flex h-6 items-center gap-2">
          <div
            v-indicator
            role="group"
            :aria-label="t('stream.scope.label')"
            class="flex gap-0.5 rounded-6 bg-hover p-0.5"
          >
            <button
              class="seg"
              :aria-pressed="stream.scope === 'all'"
              :title="t('stream.scope.allTip')"
              @click="stream.scope = 'all'"
            >
              {{ t('stream.scope.all') }}
            </button>
            <button
              class="seg"
              :aria-pressed="stream.scope === 'segment'"
              :aria-disabled="!stream.segment"
              :title="
                stream.segment ? t('stream.scope.segmentTip') : t('stream.scope.segmentDisabledTip')
              "
              @click="stream.segment && (stream.scope = 'segment')"
            >
              {{ t('stream.scope.segment') }}
            </button>
          </div>
          <button class="ml-auto btn-ghost" @click="copyRows">
            <AppIcon :name="copied ? 'check' : 'copy'" />
            <span>{{ t('stream.copy') }}</span>
          </button>
          <div ref="exportRoot" class="relative">
            <button
              class="btn-ghost"
              aria-haspopup="menu"
              :aria-expanded="exportMenu"
              :disabled="capture.job !== null"
              @click="exportMenu = !exportMenu"
            >
              <AppIcon name="export" />
              <span>{{ t('stream.export.label') }}</span>
            </button>
            <Transition name="pop">
              <div
                v-if="exportMenu"
                role="menu"
                class="absolute top-full right-0 z-20 mt-1 flex w-44 origin-top-right flex-col rounded-6 bg-menu p-1 text-small"
              >
                <button
                  v-for="kind in ['csv', 'text'] as const"
                  :key="kind"
                  role="menuitem"
                  class="flex h-[30px] w-full items-center rounded-4 px-2 text-left hover:bg-hover"
                  @click="exportRows(kind)"
                >
                  {{ t(`stream.export.${kind}`) }}
                </button>
              </div>
            </Transition>
          </div>
        </div>
      </div>
      <!-- What the time and kind columns measure, unseen. -->
      <div
        aria-hidden="true"
        class="invisible absolute top-0 left-0 size-0 overflow-hidden text-small leading-5"
      >
        <div ref="timeProbe" class="w-max font-mono tabular-nums">
          <div class="font-sans">{{ t('stream.column.time') }}</div>
          <div v-for="(text, at) in extremes" :key="at">{{ text }}</div>
        </div>
        <div ref="kindProbe" class="w-max">
          <div>{{ t('stream.column.kind') }}</div>
          <div v-for="(kind, at) in kinds" :key="at">{{ kind }}</div>
        </div>
      </div>
      <div
        ref="list"
        tabindex="0"
        :aria-label="t('stream.list')"
        class="relative min-h-0 flex-1 overflow-auto pb-0.5 font-mono text-small leading-5 tabular-nums focus-visible:outline-offset-[-2px]"
        :style="columns"
        @scroll.passive="onScroll"
        @keydown="onListKeydown"
      >
        <div class="sticky top-0 z-[1] flex gap-3 bg-canvas px-3 whitespace-nowrap text-ink-3">
          <span class="w-(--time-w) flex-none text-right font-sans">{{
            t('stream.column.time')
          }}</span>
          <span v-if="counter" class="w-(--count-w) flex-none text-right" :title="counter">{{
            t('stream.column.counter')
          }}</span>
          <span class="w-(--kind-w) flex-none truncate font-sans">{{
            t('stream.column.kind')
          }}</span>
          <span class="font-sans">{{ t('stream.column.fields') }}</span>
        </div>
        <!-- Another stream or scope is another listing, which rises in. -->
        <div
          :key="`${stream.decoder}:${stream.stream}:${stream.scope}`"
          class="rise-in relative"
          :style="{ height: `${stream.total * ROW}px` }"
        >
          <template v-for="{ index, row, find, kind, parts } in visible" :key="index">
            <div
              v-if="row"
              class="absolute inset-x-0 flex h-5 cursor-pointer gap-3 px-3 hover:bg-hover"
              :class="
                find === 'current'
                  ? 'bg-find-strong'
                  : find === 'hit'
                    ? 'bg-find'
                    : inSegment(row) && 'bg-sel'
              "
              :style="{ top: `${index * ROW}px` }"
              :aria-current="isPicked(row) || undefined"
              @mouseenter="stream.hot = { start: row.start, end: row.end }"
              @mouseleave="stream.hot = null"
              @click="pick(row)"
            >
              <i v-if="isPicked(row)" class="absolute inset-y-0 left-0 w-0.5 bg-ink" />
              <span class="w-(--time-w) flex-none text-right text-ink-3">{{ time(row) }}</span>
              <span v-if="counter" class="w-(--count-w) flex-none text-right text-ink-3">{{
                parts.counter
              }}</span>
              <span class="w-(--kind-w) flex-none truncate font-sans text-ink-2">{{ kind }}</span>
              <span class="min-w-0 flex-1 overflow-hidden text-ellipsis whitespace-pre"
                ><span v-for="(cell, at) in parts.cells" :key="at" class="cell"
                  ><span v-if="cell.label" class="font-sans text-ink-3">{{ `${cell.label} ` }}</span
                  ><span class="text-ink">{{ cell.text }}</span></span
                ></span
              >
            </div>
          </template>
        </div>
      </div>
      <section
        v-if="details"
        class="flex max-h-[45%] min-h-0 flex-none flex-col border-t border-line bg-chrome"
        :aria-label="t('stream.details.label')"
      >
        <div class="flex h-[30px] flex-none items-center gap-2 pr-1.5 pl-3 text-small">
          <span class="truncate font-medium">{{ details.kind }}</span>
          <span class="flex-none font-mono text-ink-2 tabular-nums">{{
            t('stream.details.time', { time: details.time })
          }}</span>
          <span v-if="details.length" class="truncate text-ink-3 tabular-nums">{{
            t('stream.details.length', { length: details.length })
          }}</span>
          <button
            class="ml-auto btn-icon flex-none"
            :aria-label="t('stream.details.close')"
            :title="t('stream.details.close')"
            @click="picked = null"
          >
            <AppIcon name="close" />
          </button>
        </div>
        <dl
          :key="details.key"
          class="rise-in grid min-h-0 grid-cols-[fit-content(45%)_minmax(0,1fr)] content-start gap-x-4 overflow-auto px-3 pb-2 text-small leading-5"
        >
          <template v-for="field in details.fields" :key="field.name">
            <dt class="truncate text-ink-3">{{ field.label }}</dt>
            <dd class="font-mono break-all whitespace-pre-wrap tabular-nums">{{ field.text }}</dd>
          </template>
        </dl>
      </section>
    </div>
  </aside>
</template>

<style scoped>
.cell + .cell {
  margin-left: 1.5ch;
}
</style>
