<script setup lang="ts">
import { useElementSize } from '@vueuse/core'
import { computed, shallowRef, useTemplateRef, watch } from 'vue'
import { useI18n } from 'vue-i18n'

import { getBackend } from '@/ipc/backend'
import { useCaptureStore } from '@/stores/capture'
import { useDecodeStore } from '@/stores/decode'
import { useSearchStore } from '@/stores/search'
import { useStreamStore } from '@/stores/stream'
import { useViewStore } from '@/stores/view'
import { busKind, type BusKind } from '@/timeline/groups'
import { overviewTicks } from '@/timeline/ticks'
import { formatTimeShort } from '@/utils/units'

const { t } = useI18n()
const capture = useCaptureStore()
const view = useViewStore()
const strip = useTemplateRef<HTMLElement>('strip')
const { width } = useElementSize(strip)

const decode = useDecodeStore()
const stream = useStreamStore()
const finder = useSearchStore()

/** The data stream row under the pointer. */
const hot = computed(() => {
  const row = stream.hot
  if (!row) return null
  return { left: percent(row.start), width: percent(row.end) - percent(row.start) }
})

/** The search hit shown, or where the level pattern was found. */
const found = computed(() => {
  const mark = finder.mark
  if (!mark) return null
  return { left: percent(mark.start), width: percent(mark.end) - percent(mark.start) }
})

const percent = (sample: number) => (capture.extent > 0 ? (sample / capture.extent) * 100 : 0)

// Activity of each decoder over the whole capture: one 5 px row per decoder, in its bus colour,
// over the stretches where it has events at the strip's resolution.
interface Coverage {
  index: number
  bus: BusKind
  stretches: [number, number][]
}

/** The decoders with streams to mark, each in the colour of the bus it starts with. */
const marked = computed(() =>
  decode.statuses
    .filter((status) => status.streams.length > 0)
    .map((status) => ({ index: status.index, bus: busKind(status.kinds[0]) })),
)
/** Changes when the decoders to mark do, which their statuses may do after the revision. */
const markedKey = computed(() => marked.value.map(({ index, bus }) => `${index}:${bus}`).join())

const coverage = shallowRef<Coverage[]>([])
let coverageSequence = 0
watch(
  () =>
    [
      capture.summary?.id,
      capture.extent,
      Math.round(width.value),
      decode.revision,
      markedKey.value,
    ] as const,
  async ([id, extent, pixels]) => {
    const current = ++coverageSequence
    if (id === undefined || extent <= 0 || pixels <= 0) {
      coverage.value = []
      return
    }
    try {
      const backend = await getBackend()
      const found = await Promise.all(
        marked.value.map(async ({ index, bus }) => ({
          index,
          bus,
          stretches: (await backend.decodeCoverage(id, index, 0, extent, extent / pixels)) ?? [],
        })),
      )
      if (current === coverageSequence) coverage.value = found
    } catch (error) {
      console.error('overview decode coverage failed', error)
    }
  },
  { immediate: true },
)

const BAND_COLOR: Record<BusKind, string> = {
  i2c: 'bg-i2c',
  uart: 'bg-uart',
  spi: 'bg-spi',
  can: 'bg-can',
  other: 'bg-ink-3',
}

const bands = computed(() =>
  coverage.value.flatMap(({ index, bus, stretches }, row) =>
    stretches.map(([start, end]) => ({
      key: `${index}:${start}`,
      top: 8 + row * 8,
      left: percent(start),
      width: Math.max(percent(end) - percent(start), 0.15),
      color: BAND_COLOR[bus],
    })),
  ),
)

const box = computed(() => ({
  left: percent(view.viewport.start),
  width: Math.max(percent(view.end) - percent(view.viewport.start), 0.2),
}))

/** The whole capture, or all it will hold while it fills. */
const wholeDuration = computed(() =>
  capture.samplerate > 0 ? capture.extent / capture.samplerate : 0,
)

const viewportTip = computed(() =>
  t('overview.viewport', {
    from: formatTimeShort(capture.timeOf(view.viewport.start)),
    to: formatTimeShort(capture.timeOf(view.end)),
  }),
)

const ticks = computed(() => {
  const ruler = overviewTicks(width.value, capture.extent, {
    samplerate: capture.samplerate,
    triggerSample: capture.triggerSample,
  })
  const last = ruler.ticks.length - 1
  return ruler.ticks.map((tick, index) => ({
    left: percent(capture.sampleAt(tick.time)),
    label:
      tick.time === 0
        ? t(capture.triggered ? 'overview.trigger' : 'overview.start')
        : index === last
          ? `${tick.label} ${ruler.unit}`
          : tick.label,
  }))
})

// Press to centre the view there, drag to move it, wheel to zoom.
let dragging = false

function sampleAt(event: PointerEvent | WheelEvent): number {
  const element = strip.value
  if (!element) return 0
  const rect = element.getBoundingClientRect()
  const fraction = Math.min(Math.max((event.clientX - rect.left) / rect.width, 0), 1)
  return fraction * capture.extent
}

function onPointerDown(event: PointerEvent): void {
  if (event.button !== 0) return
  dragging = true
  ;(event.currentTarget as HTMLElement).setPointerCapture(event.pointerId)
  view.centerOn(sampleAt(event))
}

function onPointerMove(event: PointerEvent): void {
  if (dragging) view.centerOn(sampleAt(event))
}

function onWheel(event: WheelEvent): void {
  event.preventDefault()
  const anchor = (sampleAt(event) - view.viewport.start) / view.viewport.samplesPerPixel
  view.zoom(Math.exp(-event.deltaY * 0.002), anchor)
}
</script>

<template>
  <div class="flex flex-none border-b border-line">
    <div class="flex w-(--lab) flex-none flex-col justify-center gap-0.5 border-r border-line px-3">
      <span class="text-caption text-ink-3">{{ t('overview.whole') }}</span>
      <span v-if="capture.summary" class="font-medium tabular-nums">{{
        formatTimeShort(wholeDuration)
      }}</span>
    </div>
    <div
      ref="strip"
      class="relative h-[var(--h-overview)] min-w-0 flex-1 touch-none overflow-hidden select-none"
      @pointerdown="onPointerDown"
      @pointermove="onPointerMove"
      @pointerup="dragging = false"
      @pointercancel="dragging = false"
      @wheel="onWheel"
    >
      <div
        v-if="capture.summary"
        class="absolute top-[3px] bottom-[17px] rounded-4 border-[1.5px] border-ink bg-canvas"
        :title="viewportTip"
        :style="{ left: `${box.left}%`, width: `${box.width}%` }"
      />
      <i
        v-for="band in bands"
        :key="band.key"
        class="pointer-events-none absolute h-[5px] rounded-[1.5px] opacity-62"
        :class="band.color"
        :style="{ top: `${band.top}px`, left: `${band.left}%`, width: `${band.width}%` }"
        aria-hidden="true"
      />
      <i
        v-if="found"
        class="pointer-events-none absolute top-0 bottom-[15px] z-[1] -ml-px bg-find-strong shadow-[inset_1px_0_0_var(--ink),inset_-1px_0_0_var(--ink)]"
        :style="{ left: `${found.left}%`, width: `max(${found.width}%, 3px)` }"
        aria-hidden="true"
      />
      <i
        v-if="hot"
        class="pointer-events-none absolute top-0 bottom-[15px] z-[1] bg-[color-mix(in_srgb,var(--ink)_14%,transparent)] shadow-[inset_1px_0_0_var(--ink),inset_-1px_0_0_var(--ink)]"
        :style="{ left: `${hot.left}%`, width: `max(${hot.width}%, 2px)` }"
        aria-hidden="true"
      />
      <span
        v-for="tick in ticks"
        :key="tick.left"
        class="absolute bottom-1 border-l border-line-2 pl-1 text-caption leading-none whitespace-nowrap text-ink-3 tabular-nums"
        :style="{ left: `${tick.left}%` }"
      >
        {{ tick.label }}
      </span>
    </div>
  </div>
</template>
