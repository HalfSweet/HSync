<script setup lang="ts">
import { useElementSize } from '@vueuse/core'
import { computed, useTemplateRef } from 'vue'
import { useI18n } from 'vue-i18n'

import { useCaptureStore } from '@/stores/capture'
import { useViewStore } from '@/stores/view'
import { overviewTicks } from '@/timeline/ticks'
import { formatTimeShort } from '@/utils/units'

const { t } = useI18n()
const capture = useCaptureStore()
const view = useViewStore()
const strip = useTemplateRef<HTMLElement>('strip')
const { width } = useElementSize(strip)

const percent = (sample: number) => (capture.extent > 0 ? (sample / capture.extent) * 100 : 0)

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
      <span class="font-medium tabular-nums">{{ formatTimeShort(wholeDuration) }}</span>
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
        class="absolute top-[3px] bottom-[17px] rounded-4 border-[1.5px] border-ink bg-canvas"
        :title="viewportTip"
        :style="{ left: `${box.left}%`, width: `${box.width}%` }"
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
