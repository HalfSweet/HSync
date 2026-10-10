<script setup lang="ts">
import { onClickOutside, useClipboard, useElementSize } from '@vueuse/core'
import { computed, nextTick, ref, shallowRef, useTemplateRef, watch } from 'vue'
import { useI18n } from 'vue-i18n'

import AppIcon from '@/components/common/AppIcon.vue'
import { defaultCaptureName, withExtension } from '@/composables/useFileActions'
import { entryFormat, shownTexts } from '@/decode/numberFormat'
import { escapePieces, hex, hexBytes, i2cMeaningText, type TextPiece } from '@/decode/text'
import { getBackend } from '@/ipc/backend'
import { pickSavePath } from '@/ipc/dialogs'
import type { StreamRow } from '@/ipc/bindings/StreamRow'
import { vIndicator } from '@/motion/indicator'
import { useCaptureStore } from '@/stores/capture'
import { useDecodeStore } from '@/stores/decode'
import { useSearchStore } from '@/stores/search'
import { useStreamStore } from '@/stores/stream'
import { useViewStore } from '@/stores/view'
import { csvLines } from '@/stream/csv'
import type { BusKind } from '@/timeline/groups'

/** The data stream under the timeline (DESIGN.md §9.9). */
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

const tabs = computed(() =>
  stream.decoders.map((status) => {
    const group = capture.groups.find((one) => one.decoder === status.index)
    return {
      index: status.index,
      name: group?.title ?? status.id,
      square: SQUARES[status.bus ?? 'other'],
    }
  }),
)

const bus = computed(() => stream.bus)
/** How the listed decoder's annotation values show. */
const format = computed(() =>
  entryFormat(stream.decoder === null ? undefined : decode.entries[stream.decoder]),
)
const address = computed(() => decode.lanesOf(stream.decoder)?.address ?? null)

/** "9 transfers, 4 bytes in the segment" and its variants. */
const summary = computed(() => {
  const counts = stream.counts
  if (!counts) return ''
  const bytes = bus.value === 'can' ? counts.frames : counts.bytes
  const unit = t(
    bus.value === 'can' || bus.value === 'uart'
      ? 'stream.summary.unitFrames'
      : 'stream.summary.unitBytes',
  )
  let total: string
  if (bus.value === 'spi' && counts.transfers === 1) {
    total = t('stream.summary.transferBytes', { count: counts.bytes })
  } else if (bus.value === 'i2c' || bus.value === 'spi') {
    total = t('stream.summary.transfers', { count: counts.transfers })
  } else {
    total = t('stream.summary.frames', { count: bus.value === 'can' ? counts.frames : bytes })
  }
  const inside = stream.inside
  if (inside === null || !stream.segment) return total
  if (stream.onlySegment) {
    const [from, to] = stream.segment
    return t('stream.summary.segmentOnly', {
      from: (capture.timeOf(from) * 1000).toFixed(3),
      to: (capture.timeOf(to) * 1000).toFixed(3),
      count: inside,
      unit,
    })
  }
  return t('stream.summary.inSegment', { total, count: inside, unit })
})

/** Width of the label column by bus, as the design sets it. */
const LABEL_WIDTHS: Partial<Record<BusKind, string>> = {
  i2c: '132px',
  spi: '36px',
  can: '64px',
  other: '120px',
}
const labelWidth = computed(() => (bus.value ? (LABEL_WIDTHS[bus.value] ?? null) : null))

const header = computed(() => {
  switch (bus.value) {
    case 'i2c':
      return {
        label: t('stream.column.meaning'),
        offset: false,
        content: t('stream.column.wireOrder'),
        chars: false,
      }
    case 'spi':
      return {
        label: t('stream.column.dataLine'),
        offset: true,
        content: t('stream.column.content'),
        chars: false,
      }
    case 'can':
      return {
        label: t('stream.column.identifier'),
        offset: false,
        content: t('stream.column.data'),
        chars: false,
      }
    case 'other':
      return {
        label: t('stream.column.row'),
        offset: false,
        content: t('stream.column.content'),
        chars: false,
      }
    default:
      return stream.effectiveMode === 'hex'
        ? { label: null, offset: true, content: t('stream.column.bytes'), chars: true }
        : { label: null, offset: false, content: t('stream.column.text'), chars: false }
  }
})

/** Search hits of the decoder listed, in time order. */
const decoderHits = computed(() =>
  (finder.result?.hits ?? []).filter((hit) => hit.decoder === stream.decoder),
)

/** Whether `row` lists a search hit: `current` for the one shown. */
function findOf(row: StreamRow): 'current' | 'hit' | null {
  if (row.type === 'gap' || row.type === 'note') return null
  const hits = decoderHits.value
  const line = 'line' in row ? row.line : null
  let lo = 0
  let hi = hits.length
  while (lo < hi) {
    const mid = (lo + hi) >>> 1
    if ((hits[mid]?.start ?? 0) < row.start) lo = mid + 1
    else hi = mid
  }
  let found: 'hit' | null = null
  for (let at = lo; at < hits.length; at++) {
    const hit = hits[at]
    if (!hit || hit.start >= row.end) break
    if (hit.line !== line) continue
    if (hit === finder.hit) return 'current'
    found = 'hit'
  }
  return found
}

// Virtual list: only the rows in sight, and a few more.
const visible = computed(() => {
  const first = Math.max(0, Math.floor(scrollTop.value / ROW) - OVERSCAN)
  const last = Math.min(
    stream.total,
    Math.ceil((scrollTop.value + listHeight.value) / ROW) + OVERSCAN,
  )
  const rows: { index: number; row: StreamRow | undefined; find: 'current' | 'hit' | null }[] = []
  // Reading `pages` makes the list update as pages arrive.
  void stream.pages
  for (let index = first; index < last; index++) {
    const row = stream.row(index)
    rows.push({ index, row, find: row ? findOf(row) : null })
  }
  return rows
})

function time(row: StreamRow): string {
  return (capture.timeOf(row.start) * 1000).toFixed(4)
}

/** Whether a row lies entirely inside the marked segment; I²C shades its bytes instead. */
function inSegment(row: StreamRow): boolean {
  return row.type !== 'gap' && row.type !== 'i2c' && spanInside(row.start, row.end)
}

interface Cell {
  text: string
  kind: 'plain' | 'muted' | 'nack' | 'escape' | 'note' | 'run'
  prefix?: string
  /** The cell lies inside the segment (I²C shades byte by byte). */
  inside?: boolean
}

function spanInside(start: number, end: number): boolean {
  const segment = stream.segment
  return !!segment && start >= segment[0] && end <= segment[1]
}

function noteText(row: Extract<StreamRow, { type: 'note' }>): string {
  return t(`stream.note.${row.note}`)
}

function textCells(pieces: TextPiece[]): Cell[] {
  return pieces.map((piece) => ({ text: piece.text, kind: piece.escape ? 'escape' : 'plain' }))
}

function cellsOf(row: StreamRow): Cell[] {
  switch (row.type) {
    case 'note':
      return [{ text: noteText(row), kind: 'note' }]
    case 'i2c': {
      const cells: Cell[] = row.tokens.map((token) => {
        const inside = spanInside(token.start, token.end)
        switch (token.type) {
          case 'start':
            return { text: 'S', kind: 'muted', inside }
          case 'repeatedStart':
            return { text: 'Sr', kind: 'muted', inside }
          case 'stop':
            return { text: 'P', kind: 'muted', inside }
          case 'nack':
            return { text: 'N', kind: 'nack', inside }
          case 'address':
            return {
              text: hex(token.address),
              kind: 'plain',
              prefix: t(token.read ? 'decode.i2c.read' : 'decode.i2c.write'),
              inside,
            }
          default:
            return { text: hex(token.value), kind: 'plain', inside }
        }
      })
      if (row.cut)
        cells.push(
          { text: '…', kind: 'muted' },
          { text: t('stream.note.captureEnds'), kind: 'note' },
        )
      return cells
    }
    case 'uartText':
    case 'spiText':
      return textCells(escapePieces(row.text))
    case 'hex':
      return [
        {
          text: row.values.map((value) => hex(value, Math.ceil(row.bits / 4))).join(' '),
          kind: 'plain',
        },
      ]
    case 'run':
      return [
        { text: t('decode.spi.run', { value: hex(row.value), count: row.count }), kind: 'run' },
      ]
    case 'can': {
      const parts = [
        t(row.fd ? 'stream.can.fd' : 'stream.can.classic'),
        t('stream.can.bytes', { count: row.data.length }),
        t(row.ok ? 'stream.can.crcOk' : 'stream.can.crcError'),
        t(row.acked ? 'stream.can.acked' : 'stream.can.noAck'),
      ]
      if (row.error) parts.push(t('stream.can.errorFrame'))
      return [
        { text: hexBytes(row.data), kind: 'plain' },
        { text: parts.join(t('stream.can.separator')), kind: 'note' },
      ]
    }
    case 'annotation':
      return [
        {
          text: shownTexts([row.text].filter(Boolean), row.value, format.value)[0] ?? '',
          kind: 'plain',
        },
      ]
    default:
      return []
  }
}

function labelOf(row: StreamRow): string {
  switch (row.type) {
    case 'i2c':
      return i2cMeaningText(row.meaning, address.value, t) + (row.cut ? ' …' : '')
    case 'spiText':
    case 'hex':
    case 'run':
      return 'line' in row && row.line ? row.line.toUpperCase() : ''
    case 'can':
      return `0x${hex(row.id, row.extended ? 8 : 3)}`
    case 'annotation':
      return decode.statusOf(stream.decoder)?.lanes[row.row]?.desc ?? ''
    default:
      return ''
  }
}

function offsetOf(row: StreamRow): string {
  return 'offset' in row ? String(row.offset) : ''
}

function charsOf(row: StreamRow): string {
  if (row.type !== 'hex') return ''
  return row.values
    .map((value) => (value >= 0x20 && value < 0x7f ? String.fromCharCode(value) : '·'))
    .join('')
}

/** Only the first of a run of rows on one data line names the line. */
function showLabel(index: number, row: StreamRow): boolean {
  if (bus.value !== 'spi') return true
  const previous = stream.row(index - 1)
  return !previous || !('line' in previous) || previous.line !== ('line' in row ? row.line : null)
}

function onScroll(): void {
  scrollTop.value = list.value?.scrollTop ?? 0
}

function goTo(row: StreamRow): void {
  if (row.type === 'gap') return
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

// The search hit shown scrolls into view, one row below the top, once its decoder is listed.
const pendingRow = shallowRef<number | null>(null)
watch(
  () => finder.hit,
  (hit) => (pendingRow.value = hit?.row ?? null),
  { immediate: true },
)
watch(
  () => [pendingRow.value, stream.total, stream.decoder, stream.onlySegment] as const,
  async ([row, total, decoder, onlySegment]) => {
    const hit = finder.hit
    if (row === null || !hit || hit.decoder !== decoder || onlySegment || total <= row) return
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
  const index = await (
    await getBackend()
  ).decodeRowAt(id, stream.decoder, stream.effectiveMode, sample)
  if (index === null) return
  await nextTick()
  list.value?.scrollTo({ top: Math.max(0, (index - 1) * ROW) })
}

function contentOf(row: StreamRow): string {
  return cellsOf(row)
    .map((cell) => (cell.prefix ?? '') + cell.text)
    .join(' ')
}

/** A row as the panel shows it, its columns separated by tabs. */
function textLine(row: StreamRow): string {
  if (row.type === 'gap') return ''
  return [time(row), labelOf(row), offsetOf(row), contentOf(row), charsOf(row)]
    .filter((part) => part !== '')
    .join('\t')
}

async function copyRows(): Promise<void> {
  const rows = await stream.allRows()
  await copy(rows.map(textLine).join('\n'))
}

// Exporting the listing (DESIGN.md §8.2): CSV with time, decoder, label, offset and content
// columns, or text as the panel shows it.
const exportMenu = ref(false)
const exportRoot = useTemplateRef<HTMLElement>('exportRoot')
onClickOutside(exportRoot, () => (exportMenu.value = false))

async function exportRows(kind: 'csv' | 'text'): Promise<void> {
  exportMenu.value = false
  const decoderName = tabs.value.find((tab) => tab.index === stream.decoder)?.name ?? 'stream'
  const extension = kind === 'csv' ? 'csv' : 'txt'
  const safeName = decoderName.replace(/[\\/:*?"<>|\s]+/g, '_')
  const suggested = defaultCaptureName(new Date(), extension).replace(
    `.${extension}`,
    `-${safeName}.${extension}`,
  )
  const path = await pickSavePath(suggested, [
    { name: t(`stream.export.${kind}`), extensions: [extension] },
  ])
  if (!path) return
  const rows = (await stream.fetchAll()).filter((row) => row.type !== 'gap')
  const text =
    kind === 'csv'
      ? csvLines([
          [
            t('stream.export.timeColumn'),
            t('stream.export.decoderColumn'),
            t('stream.export.labelColumn'),
            t('stream.column.offset'),
            t('stream.export.contentColumn'),
          ],
          ...rows.map((row) => [
            time(row),
            decoderName,
            labelOf(row),
            offsetOf(row),
            [contentOf(row), charsOf(row)].filter(Boolean).join(' '),
          ]),
        ])
      : rows.map(textLine).join('\n') + '\n'
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
          :key="tab.index"
          role="tab"
          :aria-selected="tab.index === stream.decoder"
          class="inline-flex items-center gap-1.5 px-2.5 text-small font-medium whitespace-nowrap text-ink-2 hover:text-ink"
          :class="tab.index === stream.decoder && 'text-ink'"
          @click="stream.decoder = tab.index"
        >
          <i class="size-2 rounded-2" :class="tab.square" />
          <span>{{ tab.name }}</span>
        </button>
      </div>
      <span class="self-center truncate text-small text-ink-2">{{ summary }}</span>
      <div class="ml-auto flex items-center gap-2">
        <div
          v-if="stream.hasModes"
          v-indicator
          role="group"
          :aria-label="t('stream.mode.label')"
          class="flex gap-0.5 rounded-6 bg-hover p-0.5"
        >
          <button class="seg" :aria-pressed="stream.mode === 'text'" @click="stream.mode = 'text'">
            {{ t('stream.mode.text') }}
          </button>
          <button class="seg" :aria-pressed="stream.mode === 'hex'" @click="stream.mode = 'hex'">
            {{ t('stream.mode.hex') }}
          </button>
        </div>
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
              class="absolute right-0 bottom-full z-20 mb-1 flex w-44 origin-bottom-right flex-col rounded-6 border border-ink bg-raise p-1 text-small"
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
        <span v-if="labelWidth" class="flex-none font-sans" :style="{ width: labelWidth }">{{
          header.label
        }}</span>
        <span v-if="header.offset" class="w-[4ch] flex-none text-right font-sans">{{
          t('stream.column.offset')
        }}</span>
        <span class="font-sans" :class="header.chars && 'w-[48ch] flex-none'">{{
          header.content
        }}</span>
        <span v-if="header.chars" class="font-sans">{{ t('stream.column.characters') }}</span>
      </div>
      <div class="relative" :style="{ height: `${stream.total * ROW}px` }">
        <template v-for="{ index, row, find } in visible" :key="index">
          <div
            v-if="row"
            class="absolute inset-x-0 flex h-5 gap-4 px-4"
            :class="[
              find === 'current'
                ? 'bg-find-strong'
                : find === 'hit'
                  ? 'bg-find'
                  : inSegment(row) && 'bg-sel',
              row.type !== 'gap' && row.type !== 'note' && 'cursor-pointer hover:bg-hover',
            ]"
            :style="{ top: `${index * ROW}px` }"
            @mouseenter="row.type !== 'gap' && (stream.hot = { start: row.start, end: row.end })"
            @mouseleave="stream.hot = null"
            @click="goTo(row)"
          >
            <template v-if="row.type !== 'gap'">
              <span class="w-[6ch] flex-none text-right text-ink-3">{{ time(row) }}</span>
              <span
                v-if="labelWidth"
                class="flex-none truncate font-sans text-ink-2"
                :style="{ width: labelWidth }"
                >{{ showLabel(index, row) ? labelOf(row) : '' }}</span
              >
              <span v-if="header.offset" class="w-[4ch] flex-none text-right text-ink-3">{{
                offsetOf(row)
              }}</span>
              <span class="min-w-0 whitespace-pre" :class="header.chars && 'w-[48ch] flex-none'"
                ><span
                  v-for="(cell, at) in cellsOf(row)"
                  :key="at"
                  class="cell"
                  :class="[`cell-${cell.kind}`, cell.inside && 'bg-sel']"
                  ><i v-if="cell.prefix" class="cell-prefix">{{ cell.prefix }}</i
                  >{{ cell.text }}</span
                ></span
              >
              <span v-if="header.chars" class="whitespace-pre text-ink-2">{{ charsOf(row) }}</span>
            </template>
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

.cell-muted,
.cell-escape {
  color: var(--ink-3);
}

.cell-escape + .cell-escape,
.cell-plain + .cell-escape,
.cell-escape + .cell-plain {
  margin-left: 0;
}

.cell-nack {
  font-weight: 600;
}

.cell-run {
  color: var(--ink-2);
}

.cell-note {
  color: var(--ink-3);
  font-family: var(--sans);
}

.cell-prefix {
  margin-right: 3px;
  color: var(--i2c);
  font: normal 11px/1 var(--sans);
}
</style>
