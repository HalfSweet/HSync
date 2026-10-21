<script setup lang="ts">
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'

import { useDecoderCopy } from '@/composables/useDecoderCopy'
import type { NumberFormat } from '@/decode/numberFormat'
import { fieldTexts, primaryFields, valueText, type Wording } from '@/decode/present'
import type { EventRole } from '@/ipc/bindings/EventRole'
import type { EventRow } from '@/ipc/bindings/EventRow'
import type { LaneBlock } from '@/ipc/bindings/LaneBlock'
import type { StreamView } from '@/ipc/bindings/StreamView'
import type { Viewport } from '@/timeline/viewport'
import { formatTime } from '@/utils/units'

/**
 * One decode lane of the timeline (DESIGN.md §6.3): a block per event, whatever the protocol,
 * carrying the kind's name and the event's main value and coloured by the event's role;
 * events too close to tell apart at the scale come packed into a dense block, painted by the
 * role covering most of every two pixels.
 */
const props = defineProps<{
  /** The node type whose events the lane shows, e.g. `i2c`; it names the kinds and fields. */
  kind: string
  view: StreamView | undefined
  viewport: Viewport
  decoder: number
  /** Position of the stream among the decoder's. */
  stream: number
  /** A presentation lane of the same event stream, such as JTAG's state intervals. */
  annotation?: string
  /** The block the marked segment came from. */
  selected: string | null
  /** Converts a sample to seconds from the trigger, for tips. */
  timeOf: (sample: number) => number
  /** How numbers show. */
  format: NumberFormat
}>()
const emit = defineEmits<{
  pick: [block: { id: string; start: number; end: number }]
}>()

const { t } = useI18n()
const copy = useDecoderCopy()

/** State names show from this width on, abbreviated when they do not fit. */
const STATE_TEXT = 40

const kindLabel = (kind: string) => copy.eventLabel(props.kind, kind)
const fieldLabel = (field: string) => copy.eventFieldLabel(props.kind, field)

const wording = computed<Wording>(() => ({
  enumValue: (field, value) => copy.valueLabel(props.kind, field, value),
  yes: t('decode.yes'),
  no: t('decode.no'),
  none: t('decode.none'),
}))

const at = (sample: number) => formatTime(props.timeOf(sample), 4, 'ms')

interface Geometry {
  left: number
  width: number
  cutLeft: boolean
  cutRight: boolean
}

function geometry(start: number, end: number, gap: number): Geometry {
  const { start: origin, samplesPerPixel, width } = props.viewport
  const x0 = (start - origin) / samplesPerPixel
  const x1 = (end - origin) / samplesPerPixel
  const left = Math.max(0, x0)
  const right = Math.min(width, x1)
  return {
    left,
    // Leave a pixel between events, but keep point events visible at every scale.
    width: Math.max(right - left - gap, 2),
    cutLeft: x0 < 0,
    cutRight: x1 > width,
  }
}

const blockId = (block: LaneBlock) =>
  `${props.decoder}:${props.stream}:${props.annotation ? `${props.annotation}:` : ''}${block.type}:${block.start}:${block.end}`

/** An event's tip: its kind, every field as shown, and when it starts. */
function tipOf(event: EventRow): string {
  if (props.annotation === 'states') {
    return t('decode.intervalTip', {
      label: copy.valueLabel(props.kind, 'state', String(event.payload.state)),
      start: at(event.start),
      end: at(event.end),
      duration: formatTime(props.timeOf(event.end) - props.timeOf(event.start)),
    })
  }
  const fields = fieldTexts(event.payload, props.format, wording.value).map(({ name, text }) =>
    t('decode.field', { label: fieldLabel(name), value: text }),
  )
  return t('decode.blockTip', {
    label: [kindLabel(event.kind), ...fields].join(', '),
    time: at(event.start),
  })
}

/** A stretch of a dense block in one role, placed within the block. */
interface DensePart {
  key: number
  left: number
  width: number
  role: EventRole
}

interface DensePiece {
  type: 'dense'
  id: string
  g: Geometry
  count: number
  parts: DensePart[]
}

interface EventPiece {
  type: 'event'
  id: string
  g: Geometry
  on: boolean
  event: EventRow
  /** An error among the events of the block makes it an error. */
  role: EventRole
  /** The kind's name, shown when the block has room for it beside the value. */
  label: string
  /** The main values the block has room for (an SPI word's two lines, or the first alone),
   * or nothing. */
  value: string
  /** Whether the kind's name fits before the value. */
  labelFits: boolean
  /** A TAP state that shifts a register: the scan's data itself. */
  emphasis: boolean
  tip: string
}

/** Pixels of 12 px monospace and 11 px text per character, and the block's padding. */
const VALUE_CHAR = 7.3
const LABEL_CHAR = 6
const PADDING = 16

/**
 * What a block `width` wide shows of `values`: all of them, else the first alone, else
 * nothing; and whether the kind's name fits before that.
 */
function fitting(
  label: string,
  values: readonly string[],
  width: number,
): { value: string; label: boolean } {
  const candidates = values.length > 1 ? [values.join(' · '), values[0] ?? ''] : [...values]
  const value = candidates.find((text) => PADDING + text.length * VALUE_CHAR <= width) ?? ''
  const valueWidth = value.length * VALUE_CHAR
  return {
    value,
    label: PADDING + label.length * LABEL_CHAR + (value ? 8 + valueWidth : 0) <= width,
  }
}

/** The parts of a dense block, placed within it. */
function denseParts(block: Extract<LaneBlock, { type: 'dense' }>, g: Geometry): DensePart[] {
  const { start: origin, samplesPerPixel } = props.viewport
  return block.parts.flatMap((part) => {
    const left = Math.max(0, (part.start - origin) / samplesPerPixel - g.left)
    const right = Math.min(g.width, (part.end - origin) / samplesPerPixel - g.left)
    return right > left
      ? [{ key: part.start, left, width: Math.max(right - left, 1), role: part.role }]
      : []
  })
}

/** What is drawn in view. Lanes reach beyond the view, so panning shows them at once; what
 * lies wholly outside is left out. */
const pieces = computed<(DensePiece | EventPiece)[]>(() => {
  const { start: origin, samplesPerPixel, width } = props.viewport
  const end = origin + width * samplesPerPixel
  // A state change and a scan boundary can occupy exactly the same samples. Give them
  // one marker with every event in its tip, rather than overlapping buttons and keys.
  const grouped = new Map<string, { block: LaneBlock; events: EventRow[] }>()
  for (const block of props.view?.blocks ?? []) {
    const id = blockId(block)
    const held = grouped.get(id)
    if (held && block.type === 'event') held.events.push(block)
    else grouped.set(id, { block, events: block.type === 'event' ? [block] : [] })
  }
  return [...grouped.values()]
    .filter(({ block }) => block.end >= origin && block.start <= end)
    .map(({ block, events }) => {
      const g = geometry(block.start, block.end, block.type === 'event' ? 1 : 0)
      const id = blockId(block)
      if (block.type === 'dense') {
        return { type: 'dense', id, g, count: block.count, parts: denseParts(block, g) }
      }
      const state = props.annotation === 'states'
      const label = state ? '' : events.map((event) => kindLabel(event.kind)).join(' · ')
      const values = events.flatMap((event) =>
        primaryFields(event.payload).map(([name, held]) =>
          valueText(name, held, props.format, wording.value),
        ),
      )
      // State names occupy their own row; show as much of the name as fits, with the
      // complete localized name and duration in the tip, even at a narrow clock cycle.
      const { value, label: labelFits } = state
        ? { value: g.width >= STATE_TEXT ? (values[0] ?? '') : '', label: false }
        : fitting(label, values, g.width)
      const role = events.some((event) => event.role === 'error') ? 'error' : block.role
      return {
        type: 'event',
        id,
        g,
        on: props.selected === id,
        event: block,
        role,
        label,
        value,
        labelFits,
        emphasis: state && String(block.payload.state).startsWith('shift-'),
        tip: events.map(tipOf).join('\n'),
      }
    })
})

function blockClass(piece: EventPiece): (string | false)[] {
  // A block without text shows its role in the stronger mark colour, so that narrow blocks
  // stay distinct when zoomed out; an error mark spans the lane.
  const bare = !piece.labelFits && !piece.value
  return [
    'blk',
    `role-${piece.role}`,
    props.annotation === 'states' && 'blk-state',
    piece.on && 'blk-on',
    bare && 'blk-mark',
    bare && piece.role === 'error' && 'blk-alarm',
    !bare && piece.emphasis && 'blk-deep',
    piece.g.cutLeft && 'blk-cl',
    piece.g.cutRight && 'blk-cr',
  ]
}

function blockStyle(piece: EventPiece): Record<string, string> {
  const alarm = !piece.labelFits && !piece.value && piece.role === 'error'
  return {
    left: `${piece.g.left}px`,
    width: `${alarm ? Math.max(piece.g.width, 3) : piece.g.width}px`,
  }
}

function pick(piece: EventPiece): void {
  emit('pick', { id: piece.id, start: piece.event.start, end: piece.event.end })
}
</script>

<template>
  <div class="absolute inset-0 overflow-hidden">
    <template v-for="piece in pieces" :key="piece.id">
      <span
        v-if="piece.type === 'dense'"
        class="dense"
        :style="{ left: `${piece.g.left}px`, width: `${piece.g.width}px` }"
        :title="t('decode.dense', { count: piece.count })"
      >
        <i
          v-for="part in piece.parts"
          :key="part.key"
          class="part"
          :class="[`role-${part.role}`, part.role === 'error' && 'part-alarm']"
          :style="{ left: `${part.left}px`, width: `${part.width}px` }"
        />
      </span>
      <button
        v-else
        :class="blockClass(piece)"
        :style="blockStyle(piece)"
        :aria-pressed="piece.on"
        :aria-label="piece.tip"
        :title="piece.tip"
        @click="pick(piece)"
      >
        <span v-if="piece.labelFits" class="kind">{{ piece.label }}</span>
        <span v-if="piece.value" class="val">{{ piece.value }}</span>
      </button>
    </template>
  </div>
</template>

<style scoped>
/* la-blk: a block with text sits on its role's fill, inside a border of the deeper fill. */
.blk {
  position: absolute;
  top: 2px;
  bottom: 2px;
  display: flex;
  align-items: center;
  gap: 8px;
  min-width: 0;
  padding: 0 8px;
  overflow: hidden;
  border: 1px solid var(--r-deep);
  border-radius: 3px;
  background-color: var(--r-fill);
  background-image: var(--r-fill-hatch, none);
  color: var(--ink);
  font-size: 12px;
  line-height: 1;
  white-space: nowrap;
}

.blk:hover {
  border-color: var(--ink-3);
}

.blk-cl {
  border-top-left-radius: 0;
  border-bottom-left-radius: 0;
  border-left-color: transparent;
}

.blk-cr {
  border-top-right-radius: 0;
  border-bottom-right-radius: 0;
  border-right-color: transparent;
}

/* A block without room for text: the role's mark colour, edge to edge. */
.blk-mark {
  padding: 0;
  border-color: transparent;
  background-color: var(--r);
  background-image: var(--r-hatch, none);
}

/* An error too narrow for text spans the lane, so that it stands out at every scale. */
.blk-alarm {
  top: 0;
  bottom: 0;
  border-radius: 1px;
}

.blk-deep {
  background-color: var(--r-deep);
}

.blk-on {
  border-color: var(--ink);
  background: var(--ink);
  color: var(--canvas);
}

.kind {
  font-size: 11px;
  color: var(--ink-2);
}

.val {
  font-family: var(--mono);
  font-weight: 500;
}

.role-error .kind,
.role-error .val {
  color: var(--alert);
}

.blk-state:not(.blk-mark) {
  padding: 0 4px;
}

.blk-state .val {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
}

.blk-on .kind,
.blk-on .val {
  color: inherit;
}

.blk-on .kind {
  opacity: 0.72;
}

/* A dense block: stretches in the role covering most of every two pixels. */
.dense {
  position: absolute;
  top: 2px;
  bottom: 2px;
}

.part {
  position: absolute;
  top: 0;
  bottom: 0;
  background-color: var(--r);
  background-image: var(--r-hatch, none);
}

.part-alarm {
  top: -2px;
  bottom: -2px;
  min-width: 3px;
}
</style>
