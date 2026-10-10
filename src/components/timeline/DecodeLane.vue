<script setup lang="ts">
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'

import AppIcon from '@/components/common/AppIcon.vue'
import { shownTexts, type NumberFormat } from '@/decode/numberFormat'
import type { DecoderView } from '@/ipc/bindings/DecoderView'
import type { I2cMeaning } from '@/ipc/bindings/I2cMeaning'
import type { LaneItem } from '@/ipc/bindings/LaneItem'
import type { BusKind } from '@/timeline/groups'
import type { Viewport } from '@/timeline/viewport'
import { formatSpan, formatTime } from '@/utils/units'

/**
 * One decode lane of the timeline (DESIGN.md §6.3): blocks for transfers, bytes and frames,
 * marks for I²C conditions and acknowledges, SPI text drawn character by character.
 */
const props = defineProps<{
  bus: BusKind
  /** `header`, `bytes`, `miso` or `mosi`. */
  lane: string
  view: DecoderView | undefined
  viewport: Viewport
  decoder: number
  /** The block the marked segment came from. */
  selected: string | null
  /** Converts a sample to seconds from the trigger, for tips. */
  timeOf: (sample: number) => number
  samplerate: number
  /** How annotation values show. */
  format: NumberFormat
}>()
const emit = defineEmits<{
  pick: [block: { id: string; start: number; end: number }]
  go: [sample: number]
}>()

const { t } = useI18n()

/** Blocks narrower than this share of the viewport hide their text (`la-tiny`). */
const TINY_SHARE = 0.032

const items = computed(
  () => props.view?.lanes.find((lane) => lane.lane === props.lane)?.items ?? [],
)

const hex = (value: number, digits = 2) => value.toString(16).toUpperCase().padStart(digits, '0')
const at = (sample: number) => formatTime(props.timeOf(sample), 4, 'ms')
const span = (from: number, to: number) =>
  props.samplerate > 0 ? formatSpan((to - from) / props.samplerate) : ''

function meaningText(meaning: I2cMeaning): string {
  const data = 'data' in meaning ? meaning.data.map((value) => hex(value)).join(' ') : ''
  switch (meaning.type) {
    case 'readRegister':
      return t('decode.i2c.readRegister', { register: hex(meaning.register), data })
    case 'writeRegister':
      return t('decode.i2c.writeRegister', { register: hex(meaning.register), data })
    case 'read':
      return t('decode.i2c.readData', { data })
    case 'write':
      return t('decode.i2c.writeData', { data })
    default:
      return t('decode.i2c.probe', { address: hex(props.view?.address ?? 0) })
  }
}

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

const blockId = (item: LaneItem) => `${props.decoder}:${props.lane}:${item.start}:${item.end}`

/** Rendered pieces in view: blocks, marks and SPI characters. Lanes reach beyond the view, so
 * panning shows them at once; what lies wholly outside is left out. */
const pieces = computed(() => {
  const { start: origin, samplesPerPixel, width } = props.viewport
  const end = origin + width * samplesPerPixel
  return items.value
    .filter((item) => item.end >= origin && item.start <= end)
    .map((item) => {
      const g = geometry(item.start, item.end)
      const id = blockId(item)
      return { item, g, id, on: props.selected === id }
    })
})

/** SPI characters in view, with their x and whether their line is selected. */
const characters = computed(() =>
  pieces.value.flatMap(({ item, on }) => {
    if (item.type !== 'spiText') return []
    const { start: origin, samplesPerPixel, width } = props.viewport
    return [...item.text].flatMap((ch, index) => {
      const x = ((item.centers[index] ?? item.start) - origin) / samplesPerPixel
      return x < -8 || x > width + 8
        ? []
        : [{ x, ch, control: ch === '·', on, key: `${item.start}:${index}` }]
    })
  }),
)

/** Character columns closer than this overlap; the lane then shows the line blocks only. */
const crowded = computed(() => {
  const xs = characters.value.map((c) => c.x)
  for (let i = 1; i < xs.length; i++) if ((xs[i] ?? 0) - (xs[i - 1] ?? 0) < 7) return true
  return false
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

const empty = computed(() => props.lane === 'header' && pieces.value.length === 0)

function blockClass(g: Geometry, on: boolean, extra = ''): string[] {
  return [
    'blk',
    on ? 'blk-on' : extra || tint.value,
    g.cutLeft ? 'blk-cl' : '',
    g.cutRight ? 'blk-cr' : '',
    g.tiny ? 'blk-tiny' : '',
  ]
}

/** Characters of 12 px text that fit a block `width` wide, past its padding. */
const fits = (text: string, width: number) => text.length * 7 + 16 <= width

/** The longest of an annotation's texts that fits its block, or nothing. */
function fitting(texts: readonly string[], width: number): string {
  return texts.find((text) => fits(text, width)) ?? ''
}

function pick(item: LaneItem, id: string): void {
  emit('pick', { id, start: item.start, end: item.end })
}
</script>

<template>
  <div class="absolute inset-0 overflow-hidden">
    <template v-for="{ item, g, id, on } in pieces" :key="id">
      <span
        v-if="item.type === 'dense'"
        class="blk blk-tiny"
        :class="tint"
        :style="{ left: `${g.left}px`, width: `${g.width}px` }"
        aria-hidden="true"
      />
      <button
        v-else-if="item.type === 'i2cTransfer'"
        :class="blockClass(g, on)"
        :style="{ left: `${g.left}px`, width: `${g.width - 1}px` }"
        :aria-pressed="on"
        :title="
          t('decode.i2c.transferTip', {
            text: meaningText(item.meaning),
            time: at(item.start),
            span: span(item.start, item.end),
          })
        "
        @click="pick(item, id)"
      >
        <span class="min-w-0 truncate"
          >{{ meaningText(item.meaning) }}{{ item.cut ? ' …' : '' }}</span
        >
      </button>
      <button
        v-else-if="item.type === 'i2cAddress'"
        :class="blockClass(g, on, 'border-i2c-mid bg-i2c-mid')"
        :style="{ left: `${g.left}px`, width: `${g.width - 1}px` }"
        :aria-pressed="on"
        :title="
          t('decode.i2c.addressTip', {
            address: hex(item.address),
            direction: t(item.read ? 'decode.i2c.read' : 'decode.i2c.write'),
            time: at(item.start),
          })
        "
        @click="pick(item, id)"
      >
        <span class="role">{{ t(item.read ? 'decode.i2c.read' : 'decode.i2c.write') }}</span>
        <span class="val">{{ hex(item.address) }}</span>
      </button>
      <button
        v-else-if="item.type === 'i2cData'"
        :class="blockClass(g, on)"
        :style="{ left: `${g.left}px`, width: `${g.width - 1}px` }"
        :aria-pressed="on"
        :title="t('decode.i2c.dataTip', { value: hex(item.value), time: at(item.start) })"
        @click="pick(item, id)"
      >
        <span class="val">{{ hex(item.value) }}</span>
      </button>
      <span
        v-else-if="item.type === 'i2cAck'"
        class="ack"
        :class="!item.ack && 'ack-n'"
        :style="{ left: `${g.left}px`, width: `${g.width - 1}px` }"
        :title="t(item.ack ? 'decode.i2c.ack' : 'decode.i2c.nack')"
      >
        {{ g.width >= 9 ? (item.ack ? 'A' : 'N') : '' }}
      </span>
      <span
        v-else-if="item.type === 'i2cCondition'"
        class="mk"
        :style="{ left: `${g.left}px` }"
        :title="
          t(
            item.condition === 'start'
              ? 'decode.i2c.start'
              : item.condition === 'stop'
                ? 'decode.i2c.stop'
                : 'decode.i2c.repeatedStart',
          )
        "
      >
        {{ item.condition === 'start' ? 'S' : item.condition === 'stop' ? 'P' : 'Sr' }}
      </span>
      <button
        v-else-if="item.type === 'uartFrame'"
        :class="blockClass(g, on)"
        :style="{ left: `${g.left}px`, width: `${g.width - 1}px` }"
        :aria-pressed="on"
        :title="t('decode.spi.byteTip', { value: hex(item.value), time: at(item.start) })"
        @click="pick(item, id)"
      >
        <span v-if="item.value === 0x20" class="uc-sp">{{ t('decode.uart.space') }}</span>
        <span v-else class="uc">{{
          item.value > 0x20 && item.value < 0x7f ? String.fromCharCode(item.value) : '·'
        }}</span>
        <span class="val dim">{{ hex(item.value) }}</span>
      </button>
      <button
        v-else-if="item.type === 'spiTransfer'"
        :class="blockClass(g, on, 'bg-hover text-ink-2')"
        :style="{ left: `${g.left}px`, width: `${g.width - 1}px` }"
        :aria-pressed="on"
        :title="t('decode.spi.transferTip', { time: at(item.start) })"
        @click="pick(item, id)"
      >
        <span>{{ t('decode.spi.transferring') }}</span>
        <span class="dim tabular-nums">{{
          t('decode.spi.offsets', { from: item.first, to: item.last })
        }}</span>
      </button>
      <button
        v-else-if="item.type === 'spiText'"
        :class="blockClass(g, on)"
        :style="{ left: `${g.left}px`, width: `${g.width - 1}px` }"
        :aria-pressed="on"
        :aria-label="item.text"
        :title="item.text"
        @click="pick(item, id)"
      >
        <span v-if="crowded" class="min-w-0 truncate font-mono">{{ item.text }}</span>
      </button>
      <button
        v-else-if="item.type === 'spiRun'"
        :class="blockClass(g, on, 'bg-hover text-ink-2')"
        :style="{ left: `${g.left}px`, width: `${g.width - 1}px` }"
        :aria-pressed="on"
        @click="pick(item, id)"
      >
        <span class="val text-ink">{{
          t('decode.spi.run', { value: hex(item.value), count: item.count })
        }}</span>
        <span class="dim">{{ t('decode.spi.unchangedSince', { offset: item.since }) }}</span>
      </button>
      <button
        v-else-if="item.type === 'spiWord'"
        :class="blockClass(g, on)"
        :style="{ left: `${g.left}px`, width: `${g.width - 1}px` }"
        :aria-pressed="on"
        @click="pick(item, id)"
      >
        <span class="val">{{ hex(item.value) }}</span>
      </button>
      <button
        v-else-if="item.type === 'canFrame'"
        :class="blockClass(g, on)"
        :style="{ left: `${g.left}px`, width: `${g.width - 1}px` }"
        :aria-pressed="on"
        :title="
          t('decode.can.frameTip', {
            id: hex(item.id, item.extended ? 8 : 3),
            time: at(item.start),
          })
        "
        @click="pick(item, id)"
      >
        <span class="val">0x{{ hex(item.id, item.extended ? 8 : 3) }}</span>
        <span class="dim"
          >{{ item.fd ? 'FD · ' : '' }}{{ t('decode.can.length', { count: item.length }) }}</span
        >
        <span v-if="!item.ok" class="text-alert">{{ t('decode.can.crcError') }}</span>
      </button>
      <button
        v-else-if="item.type === 'annotation'"
        :class="blockClass(g, on)"
        :style="{ left: `${g.left}px`, width: `${g.width - 1}px` }"
        :aria-pressed="on"
        :title="`${shownTexts(item.texts, item.value, format)[0] ?? ''} · ${at(item.start)}`"
        @click="pick(item, id)"
      >
        <span class="min-w-0 truncate">{{
          fitting(shownTexts(item.texts, item.value, format), g.width)
        }}</span>
      </button>
    </template>
    <span
      v-for="c in crowded ? [] : characters"
      :key="c.key"
      class="chr"
      :class="[c.control && 'text-spi', c.on && 'text-canvas']"
      :style="{ left: `${c.x}px` }"
      aria-hidden="true"
    >
      {{ c.ch }}
    </span>
    <div
      v-if="empty && bus === 'can' && view"
      class="flex h-full items-center gap-2 px-3 text-small text-ink-3"
    >
      <span>{{ t('decode.can.noFramesInView') }}</span>
      <button v-if="view.previous !== null" class="btn-ghost" @click="emit('go', view.previous)">
        <AppIcon name="chevron-left" />
        <span>{{ t('decode.can.previous') }}</span>
      </button>
      <button v-if="view.next !== null" class="btn-ghost" @click="emit('go', view.next)">
        <span>{{ t('decode.can.next') }}</span>
        <AppIcon name="chevron-right" />
      </button>
    </div>
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
  color: var(--i2c);
  font-size: 11px;
}

.val {
  font-family: var(--mono);
  font-weight: 500;
}

.uc {
  font-family: var(--mono);
  font-size: 13px;
  font-weight: 500;
}

.uc-sp {
  color: var(--ink-3);
  font-size: 11px;
}

.dim {
  color: var(--ink-3);
}

.blk-on .role,
.blk-on .dim,
.blk-on .uc-sp {
  color: inherit;
  opacity: 0.72;
}

/* la-ack */
.ack {
  position: absolute;
  top: 2px;
  bottom: 2px;
  display: flex;
  align-items: center;
  justify-content: center;
  border-radius: 2px;
  color: var(--i2c);
  font-size: 11px;
  font-weight: 500;
  line-height: 1;
}

.ack-n {
  box-shadow: inset 0 0 0 1px var(--line-2);
  color: var(--ink);
}

/* la-mk */
.mk {
  position: absolute;
  top: 5px;
  bottom: 5px;
  display: flex;
  min-width: 14px;
  align-items: center;
  justify-content: center;
  padding: 0 1px;
  transform: translateX(-50%);
  border: 1px solid var(--line-2);
  border-radius: 3px;
  background: var(--canvas);
  color: var(--ink-2);
  font-size: 11px;
  font-weight: 600;
  line-height: 1;
}

/* la-chr */
.chr {
  position: absolute;
  top: 0;
  bottom: 0;
  display: flex;
  align-items: center;
  transform: translateX(-50%);
  font: 500 13px/1 var(--mono);
  white-space: pre;
  pointer-events: none;
}
</style>
