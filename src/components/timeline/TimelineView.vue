<script setup lang="ts">
import { useDevicePixelRatio, useEventListener } from '@vueuse/core'
import {
  computed,
  nextTick,
  onBeforeUnmount,
  onMounted,
  shallowRef,
  useTemplateRef,
  watch,
} from 'vue'
import { useI18n } from 'vue-i18n'

import { useRowMetrics } from '@/composables/useRowMetrics'
import { getBackend } from '@/ipc/backend'
import type { BinsResult } from '@/ipc/bins'
import type { PulseInfo } from '@/ipc/bindings/PulseInfo'
import type { TriggerCondition } from '@/ipc/bindings/TriggerCondition'
import { Canvas2DRenderer } from '@/render/canvas2d'
import { readRenderTokens } from '@/render/tokens'
import type { FrameDescriptor, RenderTokens, TraceDraw } from '@/render/types'
import { useCaptureStore } from '@/stores/capture'
import { useChannelsStore } from '@/stores/channels'
import { usePreferencesStore } from '@/stores/preferences'
import { useViewStore } from '@/stores/view'
import type { ChannelRef } from '@/timeline/groups'
import { collapsedByDefault, layoutRows, type Row } from '@/timeline/layout'
import { pulseMode, readPulse } from '@/timeline/pulse'
import { segmentGeometry } from '@/timeline/segment'
import { viewportEnd, xToSample } from '@/timeline/viewport'
import { formatFrequency, formatTime } from '@/utils/units'

import ChannelLabel from './ChannelLabel.vue'
import GroupLabel from './GroupLabel.vue'
import PulseReadout from './PulseReadout.vue'
import TimelineRuler from './TimelineRuler.vue'
import TriggerMenu from './TriggerMenu.vue'

/** Rows are never narrower than this, label column included; narrower windows scroll. */
const MIN_ROWS_WIDTH = 860
/** Height of the trigger menu, for keeping it inside the timeline. */
const MENU_HEIGHT = 236

const { t } = useI18n()
const capture = useCaptureStore()
const channels = useChannelsStore()
const preferences = usePreferencesStore()
const view = useViewStore()
const metrics = useRowMetrics()
const { pixelRatio } = useDevicePixelRatio()

const scroller = useTemplateRef<HTMLElement>('scroller')
const rowsElement = useTemplateRef<HTMLElement>('rows')
const canvas = useTemplateRef<HTMLCanvasElement>('canvas')

const layout = computed(() =>
  layoutRows(capture.groups, metrics.value, (group) =>
    channels.isCollapsed(group.key, collapsedByDefault(group)),
  ),
)

const isTrace = (row: Row) => row.kind === 'trace' || row.kind === 'trace-narrow'

// Size and scroll position of the visible area.
const client = shallowRef({ width: 0, height: 0 })
const scroll = shallowRef({ top: 0, left: 0 })
let resizeObserver: ResizeObserver | undefined

function measure(): void {
  const element = scroller.value
  if (!element) return
  client.value = { width: element.clientWidth, height: element.clientHeight }
  const rowsWidth = Math.max(element.clientWidth, MIN_ROWS_WIDTH)
  view.setWidth(Math.max(0, rowsWidth - metrics.value.label))
}

function onScroll(): void {
  const element = scroller.value
  if (!element) return
  scroll.value = { top: element.scrollTop, left: element.scrollLeft }
  if (menu.value) closeMenu(false)
}

onMounted(() => {
  resizeObserver = new ResizeObserver(measure)
  if (scroller.value) resizeObserver.observe(scroller.value)
  measure()
})
onBeforeUnmount(() => resizeObserver?.disconnect())
watch(metrics, measure)

// Column summaries of every traced channel over the visible samples.
const traceChannels = computed(() => [
  ...new Set(
    layout.value.rows.filter(isTrace).flatMap((row) => (row.channel ? [row.channel.index] : [])),
  ),
])

const binsRequest = computed(() => {
  const summary = capture.summary
  const viewport = view.viewport
  if (!summary || viewport.width <= 0 || traceChannels.value.length === 0) return null
  const start = Math.max(0, Math.floor(viewport.start))
  const end = Math.min(capture.sampleCount, Math.ceil(viewportEnd(viewport)))
  if (end <= start) return null
  const bins = Math.max(1, Math.min(Math.round(viewport.width * pixelRatio.value), end - start))
  return { captureId: summary.id, channels: traceChannels.value, start, end, bins }
})

const bins = shallowRef<BinsResult | null>(null)
let binsSequence = 0
watch(
  binsRequest,
  async (request) => {
    const sequence = ++binsSequence
    if (!request) {
      bins.value = null
      return
    }
    try {
      const result = await (await getBackend()).bins(request)
      if (sequence === binsSequence) bins.value = result
    } catch (error) {
      console.error('bins request failed', error)
    }
  },
  { immediate: true },
)

// Canvas layer.
let renderer: Canvas2DRenderer | null = null
const tokens = shallowRef<RenderTokens | null>(null)
watch(
  () => preferences.resolvedTheme,
  async () => {
    await nextTick()
    tokens.value = readRenderTokens()
  },
  { immediate: true, flush: 'post' },
)

const frame = computed<FrameDescriptor>(() => {
  const { width, height } = client.value
  const ruler = metrics.value.ruler
  const top = scroll.value.top
  const result = bins.value
  const traces: TraceDraw[] = []
  for (const row of layout.value.rows) {
    if (!isTrace(row) || !row.channel || !result) continue
    const y = ruler + row.top - top
    if (y + row.height <= ruler || y >= height) continue
    const cells = result.channels.get(row.channel.index)
    if (!cells) continue
    traces.push({
      top: y,
      height: row.height,
      wire: row.channel.wire,
      narrow: row.kind === 'trace-narrow',
      cells,
    })
  }
  return {
    width,
    height,
    dpr: pixelRatio.value,
    trackLeft: metrics.value.label - scroll.value.left,
    clipTop: ruler,
    view: view.viewport,
    grid: view.ruler.ticks.filter((tick) => tick.major).map((tick) => tick.x),
    bins: result,
    traces,
  }
})

let frameRequest = 0
function draw(): void {
  frameRequest = 0
  if (!canvas.value || !tokens.value) return
  renderer ??= new Canvas2DRenderer(canvas.value)
  renderer.draw(frame.value, tokens.value)
}
watch([frame, tokens], () => {
  frameRequest ||= requestAnimationFrame(draw)
})
onBeforeUnmount(() => {
  cancelAnimationFrame(frameRequest)
  renderer?.dispose()
})

// The segment over the rows.
const segment = computed(() =>
  view.segment ? segmentGeometry(view.segment.from, view.segment.to, view.viewport) : null,
)

// Trigger menu: one channel at a time, beside the label column (DESIGN.md §9.7).
interface OpenMenu {
  channel: ChannelRef
  top: number
  slot: HTMLElement
}
const menu = shallowRef<OpenMenu | null>(null)

function toggleMenu(channel: ChannelRef, slot: HTMLElement): void {
  if (menu.value?.slot === slot) {
    closeMenu(false)
    return
  }
  const rows = rowsElement.value?.getBoundingClientRect()
  const visible = scroller.value?.getBoundingClientRect()
  if (!rows || !visible) return
  const box = slot.getBoundingClientRect()
  const level = box.top - rows.top - 6
  const highest = visible.top - rows.top + 42
  const lowest = visible.bottom - rows.top - MENU_HEIGHT - 6
  menu.value = { channel, slot, top: Math.round(Math.max(highest, Math.min(level, lowest))) }
}

function closeMenu(returnFocus: boolean): void {
  const slot = menu.value?.slot
  menu.value = null
  if (returnFocus) slot?.focus({ preventScroll: true })
}

function pick(condition: TriggerCondition): void {
  if (menu.value) channels.setTrigger(menu.value.channel.index, condition)
  closeMenu(true)
}

useEventListener(document, 'pointerdown', (event) => {
  const target = event.target as Element | null
  if (menu.value && !target?.closest('[role="menu"], [aria-haspopup="menu"]')) closeMenu(false)
})

// Hover read-out and drag to pan on the tracks.
interface Hover {
  row: Row
  pulse: PulseInfo
}
const hover = shallowRef<Hover | null>(null)
let hoverTarget: { row: Row; sample: number } | null = null
let hoverRequest = 0
let drag: { x: number } | null = null

function trackSample(event: PointerEvent): number {
  const box = (event.currentTarget as HTMLElement).getBoundingClientRect()
  return Math.floor(xToSample(view.viewport, event.clientX - box.left))
}

async function resolveHover(): Promise<void> {
  hoverRequest = 0
  const target = hoverTarget
  const summary = capture.summary
  if (!target?.row.channel || !summary) return
  const pulse = await (
    await getBackend()
  ).pulse(summary.id, target.row.channel.index, target.sample)
  if (hoverTarget === target) hover.value = pulse ? { row: target.row, pulse } : null
}

function onTrackPointerMove(row: Row, event: PointerEvent): void {
  if (drag) {
    view.pan(drag.x - event.clientX)
    drag.x = event.clientX
    return
  }
  if (!isTrace(row) || !row.channel) return
  const sample = trackSample(event)
  if (sample < 0 || sample >= capture.sampleCount) {
    onTrackPointerLeave()
    return
  }
  const current = hover.value
  if (current?.row.key === row.key && sample >= current.pulse.start && sample < current.pulse.end) {
    return
  }
  hoverTarget = { row, sample }
  hoverRequest ||= requestAnimationFrame(() => void resolveHover())
}

function onTrackPointerLeave(): void {
  hoverTarget = null
  hover.value = null
}

function onTrackPointerDown(event: PointerEvent): void {
  if (event.button !== 0 || (event.target as Element).closest('button')) return
  drag = { x: event.clientX }
  ;(event.currentTarget as HTMLElement).setPointerCapture(event.pointerId)
  onTrackPointerLeave()
}

function onTrackPointerUp(): void {
  drag = null
}

const readout = computed(() => {
  const current = hover.value
  const channel = current?.row.channel
  if (!current || !channel) return null
  const baud = current.row.group.options.baudrate
  const reading = readPulse(
    current.pulse,
    view.viewport,
    capture.samplerate,
    pulseMode(channel),
    typeof baud === 'number' && baud > 0 ? 1 / baud : null,
  )
  const level = t(reading.high ? 'hover.high' : 'hover.low')
  const width = formatTime(reading.seconds, 1)
  let secondary: string | null = null
  if (!reading.complete) secondary = t('hover.outsideViewport')
  else if (reading.period !== null) {
    secondary = t('hover.period', {
      period: formatTime(reading.period, 1),
      frequency: formatFrequency(1 / reading.period, 1),
    })
  } else if (reading.bits !== null) secondary = t('hover.bits', { count: reading.bits })
  return {
    rowKey: current.row.key,
    left: reading.left,
    width: reading.width,
    center: reading.center,
    primary: t(reading.complete ? 'hover.pulse' : 'hover.pulseOpen', { level, width }),
    secondary,
  }
})

// Ctrl/⌘ + wheel zooms around the pointer; shift + wheel and horizontal swipes pan.
function onWheel(event: WheelEvent): void {
  const rows = rowsElement.value?.getBoundingClientRect()
  if (!rows) return
  const x = Math.min(
    Math.max(event.clientX - rows.left - metrics.value.label, 0),
    view.viewport.width,
  )
  const scale = event.deltaMode === 1 ? 16 : 1
  if (event.ctrlKey || event.metaKey) {
    event.preventDefault()
    view.zoom(Math.exp(-event.deltaY * scale * 0.002), x)
  } else if (event.shiftKey || Math.abs(event.deltaX) > Math.abs(event.deltaY)) {
    event.preventDefault()
    view.pan((event.shiftKey && event.deltaX === 0 ? event.deltaY : event.deltaX) * scale)
  }
}
</script>

<template>
  <div class="relative min-h-0 flex-1 bg-canvas">
    <canvas
      ref="canvas"
      class="pointer-events-none absolute top-0 left-0"
      :style="{ width: `${client.width}px`, height: `${client.height}px` }"
      aria-hidden="true"
    />
    <div
      ref="scroller"
      class="absolute inset-0 overflow-auto"
      @scroll.passive="onScroll"
      @wheel="onWheel"
    >
      <div ref="rows" class="relative flex min-h-full min-w-[860px] flex-col">
        <TimelineRuler class="sticky top-0 z-[5]" />

        <div
          v-for="row in layout.rows"
          :key="row.key"
          class="flex flex-none"
          :class="row.separator && 'border-t border-line'"
          :style="{ height: `${row.height}px` }"
        >
          <div
            class="sticky left-0 z-[3] flex w-(--lab) min-w-0 flex-none items-center border-r border-line bg-chrome pr-3 pl-2 whitespace-nowrap"
          >
            <GroupLabel
              v-if="row.kind === 'header'"
              :group="row.group"
              :collapsed="channels.isCollapsed(row.group.key, collapsedByDefault(row.group))"
              @toggle="channels.toggleGroup(row.group.key, collapsedByDefault(row.group))"
            />
            <ChannelLabel
              v-else-if="row.labeled && row.channel"
              :channel="row.channel"
              :condition="channels.triggerOf(row.channel.index)"
              :menu-open="menu?.channel.index === row.channel.index"
              @trigger="(slot) => row.channel && toggleMenu(row.channel, slot)"
            />
          </div>
          <div
            class="relative min-w-0 flex-1"
            @pointerdown="onTrackPointerDown"
            @pointermove="onTrackPointerMove(row, $event)"
            @pointerup="onTrackPointerUp"
            @pointercancel="onTrackPointerUp"
            @pointerleave="onTrackPointerLeave"
          >
            <PulseReadout
              v-if="readout && readout.rowKey === row.key"
              :left="readout.left"
              :width="readout.width"
              :center="readout.center"
              :primary="readout.primary"
              :secondary="readout.secondary"
            />
          </div>
        </div>

        <div class="flex flex-[1_0_0]">
          <div class="sticky left-0 z-[3] w-(--lab) flex-none border-r border-line bg-chrome" />
        </div>

        <div
          v-if="segment"
          class="pointer-events-none absolute inset-y-0 right-0 left-(--lab) z-[2]"
          aria-hidden="true"
        >
          <div
            v-if="segment.band"
            class="absolute inset-y-0 bg-veil"
            :style="{ left: `${segment.band.left}px`, width: `${segment.band.width}px` }"
          />
          <i
            v-if="segment.a.visible"
            class="absolute inset-y-0 border-l border-ink opacity-50"
            :style="{ left: `${segment.a.x}px` }"
          />
          <i
            v-if="segment.b.visible"
            class="absolute inset-y-0 border-l border-ink opacity-50"
            :style="{ left: `${segment.b.x}px` }"
          />
        </div>

        <TriggerMenu
          v-if="menu"
          :channel="menu.channel.label"
          :wire="menu.channel.wire"
          :condition="channels.triggerOf(menu.channel.index)"
          :top="menu.top"
          @pick="pick"
          @close="closeMenu(true)"
        />
      </div>
    </div>
  </div>
</template>
