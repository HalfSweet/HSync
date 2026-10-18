<script setup lang="ts">
import { onClickOutside, useClipboard, useElementSize } from '@vueuse/core'
import { computed, nextTick, ref, shallowRef, useTemplateRef, watch } from 'vue'
import { useI18n } from 'vue-i18n'

import AppIcon from '@/components/common/AppIcon.vue'
import { defaultCaptureName, withExtension } from '@/composables/useFileActions'
import { useDecoderCopy } from '@/composables/useDecoderCopy'
import { entryFormat } from '@/decode/numberFormat'
import { fieldTexts, type Wording } from '@/decode/present'
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
import { busKind, type BusKind } from '@/timeline/groups'

/**
 * The data stream under the timeline (DESIGN.md §9): the events of one decoder's stream, one
 * row each, with the event's kind and its fields as the contract names them.
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
const MIN_HEIGHT = 80
/** The timeline keeps at least this much. */
const TIMELINE_MIN = 180

const list = useTemplateRef<HTMLElement>('list')
const { height: listHeight } = useElementSize(list)
const scrollTop = shallowRef(0)

const SQUARES: Record<BusKind, string> = {
  i2c: 'bg-i2c',
  uart: 'bg-uart',
  spi: 'bg-spi',
  can: 'bg-can',
  other: 'bg-ink-2',
}

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

/** A tab's text: the decoder, and the stream too when the decoder has several. */
function tabName(listing: Listing): string {
  const decoder = decoderName(listing.decoder)
  if (listing.streams <= 1) return decoder
  // The bus itself goes by the decoder's name; the decoders stacked on it are named too.
  if (listing.stream === 0) return decoder
  return t('stream.tab', { decoder, stream: typeName(listing.kind) })
}

const tabs = computed(() =>
  stream.listings.map((listing) => ({
    listing,
    key: `${listing.decoder}:${listing.stream}`,
    name: tabName(listing),
    square: SQUARES[busKind(listing.kind)],
  })),
)

function isCurrent(listing: Listing): boolean {
  const current = stream.current
  return (
    current !== null && current.decoder === listing.decoder && current.stream === listing.stream
  )
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

/** The fields of an event as shown, "label: value" each, in the contract's order. */
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
  cells: string[]
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
    rows.push({
      index,
      row,
      find: row ? (marks.value.get(row.index) ?? null) : null,
      kind: row ? kindText(row) : '',
      cells: row ? cellsOf(row) : [],
    })
  }
  return rows
})

/** Whether an event lies wholly inside the marked segment. */
function inSegment(row: EventRow): boolean {
  const segment = stream.segment
  return segment !== null && row.start >= segment[0] && row.end <= segment[1]
}

function onScroll(): void {
  scrollTop.value = list.value?.scrollTop ?? 0
}

function goTo(row: EventRow): void {
  view.centerOn((row.start + row.end) / 2)
  view.setSegment({ from: row.start, to: row.end })
}

// The first row inside a new segment scrolls into view, one row below the top, once the
// segment stops changing.
let settle: ReturnType<typeof setTimeout> | undefined
watch(
  () => stream.segment,
  (segment) => {
    clearTimeout(settle)
    if (!segment || stream.onlySegment) return
    settle = setTimeout(() => void scrollToSample(segment[0]), 250)
  },
)

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

// Resizing by the top edge.
let grip: { y: number; height: number; max: number } | null = null
function onGripDown(event: PointerEvent): void {
  const work = (event.currentTarget as HTMLElement).closest('main')
  grip = {
    y: event.clientY,
    height: stream.height,
    max: Math.max(MIN_HEIGHT, (work?.clientHeight ?? 600) - TIMELINE_MIN),
  }
  ;(event.currentTarget as HTMLElement).setPointerCapture(event.pointerId)
}
function onGripMove(event: PointerEvent): void {
  if (!grip) return
  stream.height = Math.min(grip.max, Math.max(MIN_HEIGHT, grip.height + grip.y - event.clientY))
}
function onGripUp(): void {
  grip = null
}
</script>

<template>
  <section
    class="relative flex flex-none flex-col border-t border-line bg-canvas"
    :style="{ height: `${stream.height}px` }"
    :aria-label="t('stream.title')"
  >
    <div
      class="group absolute inset-x-0 -top-1 z-[6] h-[7px] cursor-row-resize touch-none"
      role="separator"
      aria-orientation="horizontal"
      :aria-label="t('stream.resize')"
      :title="t('stream.resizeTip')"
      @pointerdown="onGripDown"
      @pointermove="onGripMove"
      @pointerup="onGripUp"
      @pointercancel="onGripUp"
    >
      <i class="absolute inset-x-0 top-[3px] hidden h-px bg-ink-3 group-hover:block" />
    </div>
    <div
      class="flex h-[33px] flex-none items-stretch gap-3 border-b border-line bg-chrome pr-2 pl-1"
    >
      <div
        v-indicator:line
        role="tablist"
        :aria-label="t('stream.decoders')"
        class="flex items-stretch"
      >
        <button
          v-for="tab in tabs"
          :key="tab.key"
          role="tab"
          :aria-selected="isCurrent(tab.listing)"
          class="inline-flex items-center gap-1.5 px-2.5 text-small font-medium whitespace-nowrap text-ink-2 hover:text-ink aria-selected:text-ink"
          @click="stream.select(tab.listing.decoder, tab.listing.stream)"
        >
          <i class="size-2 rounded-2" :class="tab.square" />
          <span>{{ tab.name }}</span>
        </button>
      </div>
      <span class="self-center truncate text-small text-ink-2">{{ summary }}</span>
      <div class="ml-auto flex items-center gap-2">
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
        <button class="btn-ghost" @click="copyRows">
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
              class="absolute right-0 bottom-full z-20 mb-1 flex w-44 origin-bottom-right flex-col rounded-6 bg-menu p-1 text-small"
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
        <button
          class="btn-icon"
          :aria-label="t('stream.close')"
          :title="t('stream.close')"
          @click="stream.open = false"
        >
          <AppIcon name="close" />
        </button>
      </div>
    </div>
    <div
      ref="list"
      role="tabpanel"
      class="relative min-h-0 flex-1 overflow-auto pb-0.5 font-mono text-small leading-5 tabular-nums"
      @scroll.passive="onScroll"
    >
      <div class="sticky top-0 z-[1] flex gap-4 bg-canvas px-4 whitespace-nowrap text-ink-3">
        <span class="w-[6ch] flex-none text-right font-sans">{{ t('stream.column.time') }}</span>
        <span class="w-[120px] flex-none font-sans">{{ t('stream.column.kind') }}</span>
        <span class="font-sans">{{ t('stream.column.fields') }}</span>
      </div>
      <!-- Another stream or scope is another listing, which rises in. -->
      <div
        :key="`${stream.decoder}:${stream.stream}:${stream.scope}`"
        class="rise-in relative"
        :style="{ height: `${stream.total * ROW}px` }"
      >
        <template v-for="{ index, row, find, kind, cells } in visible" :key="index">
          <div
            v-if="row"
            class="absolute inset-x-0 flex h-5 cursor-pointer gap-4 px-4 hover:bg-hover"
            :class="
              find === 'current'
                ? 'bg-find-strong'
                : find === 'hit'
                  ? 'bg-find'
                  : inSegment(row) && 'bg-sel'
            "
            :style="{ top: `${index * ROW}px` }"
            @mouseenter="stream.hot = { start: row.start, end: row.end }"
            @mouseleave="stream.hot = null"
            @click="goTo(row)"
          >
            <span class="w-[6ch] flex-none text-right text-ink-3">{{ time(row) }}</span>
            <span class="w-[120px] flex-none truncate font-sans text-ink-2">{{ kind }}</span>
            <span class="min-w-0 whitespace-pre"
              ><span v-for="(cell, at) in cells" :key="at" class="cell">{{ cell }}</span></span
            >
          </div>
        </template>
      </div>
    </div>
  </section>
</template>

<style scoped>
.cell + .cell {
  margin-left: 1ch;
}
</style>
