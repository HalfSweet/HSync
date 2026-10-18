<script setup lang="ts">
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'

import { useDecoderCopy } from '@/composables/useDecoderCopy'
import type { NumberFormat } from '@/decode/numberFormat'
import { fieldTexts, primaryFields, valueText, type Wording } from '@/decode/present'
import type { EventRow } from '@/ipc/bindings/EventRow'
import type { LaneBlock } from '@/ipc/bindings/LaneBlock'
import type { StreamView } from '@/ipc/bindings/StreamView'
import type { BusKind } from '@/timeline/groups'
import type { Viewport } from '@/timeline/viewport'
import { formatTime } from '@/utils/units'

/**
 * One decode lane of the timeline (DESIGN.md §6.3): a block per event, whatever the protocol,
 * carrying the kind's name and the event's main value; events too close to tell apart at the
 * scale come packed into a dense block.
 */
const props = defineProps<{
  /** The bus the decoder sits on, for the tint. */
  bus: BusKind
  /** The node type whose events the lane shows, e.g. `i2c`; it names the kinds and fields. */
  kind: string
  view: StreamView | undefined
  viewport: Viewport
  decoder: number
  /** Position of the stream among the decoder's. */
  stream: number
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

/** Blocks narrower than this share of the viewport hide their text (`la-tiny`). */
const TINY_SHARE = 0.032

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
  tiny: boolean
}

function geometry(start: number, end: number): Geometry {
  const { start: origin, samplesPerPixel, width } = props.viewport
  const x0 = (start - origin) / samplesPerPixel
  const x1 = (end - origin) / samplesPerPixel
  const left = Math.max(0, x0)
  const right = Math.min(width, x1)
  return {
    left,
    width: Math.max(right - left, 1),
    cutLeft: x0 < 0,
    cutRight: x1 > width,
    tiny: right - left < width * TINY_SHARE,
  }
}

const blockId = (block: LaneBlock) => `${props.decoder}:${props.stream}:${block.start}:${block.end}`

/** An event's tip: its kind, every field as shown, and when it starts. */
function tipOf(event: EventRow): string {
  const fields = fieldTexts(event.payload, props.format, wording.value).map(({ name, text }) =>
    t('decode.field', { label: fieldLabel(name), value: text }),
  )
  return t('decode.blockTip', {
    label: [kindLabel(event.kind), ...fields].join(', '),
    time: at(event.start),
  })
}

interface DensePiece {
  type: 'dense'
  id: string
  g: Geometry
  count: number
}

interface EventPiece {
  type: 'event'
  id: string
  g: Geometry
  on: boolean
  event: EventRow
  /** The kind's name, shown when the block has room for it beside the value. */
  label: string
  /** The main values the block has room for (an SPI word's two lines, or the first alone),
   * or nothing. */
  value: string
  /** Whether the kind's name fits before the value. */
  labelFits: boolean
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

/** What is drawn in view. Lanes reach beyond the view, so panning shows them at once; what
 * lies wholly outside is left out. */
const pieces = computed<(DensePiece | EventPiece)[]>(() => {
  const { start: origin, samplesPerPixel, width } = props.viewport
  const end = origin + width * samplesPerPixel
  return (props.view?.blocks ?? [])
    .filter((block) => block.end >= origin && block.start <= end)
    .map((block) => {
      const g = geometry(block.start, block.end)
      const id = blockId(block)
      if (block.type === 'dense') return { type: 'dense', id, g, count: block.count }
      const label = kindLabel(block.kind)
      const values = primaryFields(block.payload).map(([name, held]) =>
        valueText(name, held, props.format, wording.value),
      )
      const { value, label: labelFits } = fitting(label, values, g.width)
      return {
        type: 'event',
        id,
        g,
        on: props.selected === id,
        event: block,
        label,
        value,
        labelFits,
        tip: tipOf(block),
      }
    })
})

const tint = computed(
  () =>
    ({
      i2c: 'bg-i2c-wash border-i2c-mid',
      uart: 'bg-uart-wash border-uart-mid',
      spi: 'bg-spi-wash border-spi-mid',
      can: 'bg-can-wash border-can-mid',
      other: 'bg-hover border-line-2',
    })[props.bus],
)

/** The kind's name takes the bus colour. */
const roleTint = computed(
  () =>
    ({
      i2c: 'text-i2c',
      uart: 'text-uart',
      spi: 'text-spi',
      can: 'text-can',
      other: 'text-ink-2',
    })[props.bus],
)

function blockClass(g: Geometry, on: boolean): string[] {
  return [
    'blk',
    on ? 'blk-on' : tint.value,
    g.cutLeft ? 'blk-cl' : '',
    g.cutRight ? 'blk-cr' : '',
    g.tiny ? 'blk-tiny' : '',
  ]
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
        class="blk blk-tiny"
        :class="tint"
        :style="{ left: `${piece.g.left}px`, width: `${piece.g.width}px` }"
        :title="t('decode.dense', { count: piece.count })"
      />
      <button
        v-else
        :class="blockClass(piece.g, piece.on)"
        :style="{ left: `${piece.g.left}px`, width: `${piece.g.width - 1}px` }"
        :aria-pressed="piece.on"
        :title="piece.tip"
        @click="pick(piece)"
      >
        <template v-if="!piece.g.tiny">
          <span v-if="piece.labelFits" class="role" :class="roleTint">{{ piece.label }}</span>
          <span v-if="piece.value" class="val">{{ piece.value }}</span>
        </template>
      </button>
    </template>
  </div>
</template>

<style scoped>
/* la-blk */
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
  border: 1px solid transparent;
  border-radius: 3px;
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

.blk-on {
  border-color: var(--ink);
  background: var(--ink);
  color: var(--canvas);
}

.blk-tiny {
  padding: 0;
}

.blk-tiny > * {
  display: none;
}

.role {
  font-size: 11px;
}

.val {
  font-family: var(--mono);
  font-weight: 500;
}

.dim {
  color: var(--ink-3);
}

.blk-on .role,
.blk-on .dim {
  color: inherit;
  opacity: 0.72;
}
</style>
