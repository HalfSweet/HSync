<script setup lang="ts">
import { computed, useTemplateRef } from 'vue'
import { useI18n } from 'vue-i18n'

import AppIcon from '@/components/common/AppIcon.vue'
import { useCaptureStore } from '@/stores/capture'
import { useDecodeStore } from '@/stores/decode'
import { useViewStore } from '@/stores/view'
import { segmentGeometry } from '@/timeline/segment'
import { xToSample } from '@/timeline/viewport'
import { formatSpan } from '@/utils/units'

const { t } = useI18n()
const capture = useCaptureStore()
const decode = useDecodeStore()
const view = useViewStore()
const track = useTemplateRef<HTMLElement>('track')

const ticks = computed(() => view.ruler.ticks)
/** Labels too close to the left edge would run into the unit label there; their marks stay. */
const UNIT_LABEL_CLEARANCE = 32
const majors = computed(() =>
  ticks.value.filter((tick) => tick.major && tick.x >= UNIT_LABEL_CLEARANCE),
)

/** Tick marks as one SVG path: 4 px minor and 8 px major marks along the bottom edge. */
const marks = computed(() =>
  ticks.value
    .map((tick) => {
      const x = Math.round(tick.x) + 0.5
      return tick.major ? `M${x} 30v8` : `M${x} 34v4`
    })
    .join(''),
)

const segment = computed(() => {
  const current = view.segment
  if (!current) return null
  const geometry = segmentGeometry(current.from, current.to, view.viewport)
  const seconds = capture.samplerate > 0 ? (current.to - current.from) / capture.samplerate : 0
  const text = geometry.open ? t('ruler.outsideViewport') : formatSpan(seconds)
  return { ...geometry, text }
})

// Drag along the ruler to mark a segment, however short. As on the tracks, a press that moves no
// more than a few pixels is a click, which clears it; so does a drag that covers no samples.
const CLICK_SLOP = 3
let press: { x: number; sample: number; dragging: boolean } | null = null

function sampleAtPointer(event: PointerEvent): number {
  const box = track.value?.getBoundingClientRect()
  if (!box) return 0
  const x = Math.min(Math.max(event.clientX - box.left, 0), box.width)
  return Math.round(Math.min(Math.max(xToSample(view.viewport, x), 0), capture.extent))
}

function onPointerDown(event: PointerEvent): void {
  if (event.button !== 0) return
  press = { x: event.clientX, sample: sampleAtPointer(event), dragging: false }
  ;(event.currentTarget as HTMLElement).setPointerCapture(event.pointerId)
}

function onPointerMove(event: PointerEvent): void {
  if (!press) return
  if (!press.dragging && Math.abs(event.clientX - press.x) <= CLICK_SLOP) return
  press.dragging = true
  const at = sampleAtPointer(event)
  view.setSegment({ from: Math.min(press.sample, at), to: Math.max(press.sample, at) })
}

function onPointerUp(): void {
  if (!press) return
  const dragged = press.dragging
  press = null
  const current = view.segment
  if (!dragged || !current || current.to === current.from) view.setSegment(null)
}
</script>

<template>
  <div class="flex h-(--h-ruler) flex-none border-b border-line bg-canvas">
    <div
      class="sticky left-0 z-[3] flex w-(--lab) flex-none items-center justify-between gap-2 border-r border-line bg-chrome pr-3 pl-2 whitespace-nowrap"
    >
      <button
        class="btn-ghost max-[640px]:px-1"
        :disabled="!capture.summary"
        @click="decode.openEditor(null)"
      >
        <AppIcon name="plus" />
        <span class="max-[640px]:hidden">{{ t('ruler.addDecoder') }}</span>
      </button>
      <span
        class="flex w-5 flex-none justify-center text-caption whitespace-nowrap text-ink-3"
        :title="t('ruler.triggerColumnTip')"
      >
        {{ t('ruler.triggerColumn') }}
      </span>
    </div>
    <div
      ref="track"
      class="relative min-w-0 flex-1 cursor-crosshair touch-none select-none"
      :title="t('ruler.dragTip')"
      @pointerdown="onPointerDown"
      @pointermove="onPointerMove"
      @pointerup="onPointerUp"
      @pointercancel="onPointerUp"
    >
      <span
        v-if="capture.summary"
        class="absolute bottom-2.5 left-1.5 text-caption leading-none text-ink-3"
        :title="t('ruler.unitTip', { unit: t(`units.${view.ruler.unit}`) })"
      >
        {{ view.ruler.unit }}
      </span>
      <svg class="absolute inset-0 size-full overflow-visible" aria-hidden="true">
        <path :d="marks" class="fill-none stroke-line-2 [shape-rendering:crispEdges]" />
      </svg>

      <template v-if="segment">
        <template v-if="segment.a.visible">
          <i
            class="absolute top-[2px] bottom-0 border-l border-ink opacity-50"
            :style="{ left: `${segment.a.x}px` }"
          />
          <i
            class="absolute top-[2px] size-[14px] rounded-3 bg-ink text-center text-caption leading-[14px] font-semibold text-canvas not-italic"
            :style="{ left: `${segment.a.flag}px` }"
          >
            A
          </i>
        </template>
        <template v-if="segment.b.visible">
          <i
            class="absolute top-[2px] bottom-0 border-l border-ink opacity-50"
            :style="{ left: `${segment.b.x}px` }"
          />
          <i
            class="absolute top-[2px] size-[14px] rounded-3 bg-ink text-center text-caption leading-[14px] font-semibold text-canvas not-italic"
            :style="{ left: `${segment.b.flag}px` }"
          >
            B
          </i>
        </template>
      </template>

      <span
        v-for="tick in majors"
        :key="tick.time"
        class="absolute bottom-2.5 z-[1] -translate-x-1/2 bg-canvas px-[3px] text-caption leading-none text-ink-3 tabular-nums"
        :style="{ left: `${tick.x}px` }"
      >
        {{ tick.label }}
      </span>

      <template v-if="segment?.label">
        <div
          v-if="segment.label.kind === 'between'"
          class="absolute top-[2px] flex h-[14px] items-center justify-center before:absolute before:inset-x-0 before:top-1/2 before:border-t before:border-ink"
          :style="{ left: `${segment.label.left}px`, width: `${segment.label.width}px` }"
        >
          <i
            v-if="segment.label.arrowA"
            class="absolute top-1/2 left-0 -mt-[3px] border-[3.5px] border-r-[6px] border-l-0 border-transparent border-r-ink"
          />
          <i
            v-if="segment.label.arrowB"
            class="absolute top-1/2 right-0 -mt-[3px] border-[3.5px] border-r-0 border-l-[6px] border-transparent border-l-ink"
          />
          <span
            class="relative bg-canvas px-[5px] text-caption leading-[14px] font-medium whitespace-nowrap tabular-nums"
          >
            {{ segment.text }}
          </span>
        </div>
        <span
          v-else
          class="absolute top-[2px] z-[1] h-[14px] bg-canvas px-1 text-caption leading-[14px] font-medium whitespace-nowrap tabular-nums"
          :style="
            segment.label.kind === 'besideB'
              ? { left: `${segment.label.left}px` }
              : { right: `${segment.label.right}px` }
          "
        >
          {{ segment.text }}
        </span>
      </template>
    </div>
  </div>
</template>
