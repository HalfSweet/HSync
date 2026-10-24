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

import { useControlHeld } from '@/composables/useControlHeld'
import { useDecoderCopy } from '@/composables/useDecoderCopy'
import { useRowMetrics } from '@/composables/useRowMetrics'
import { useShortcuts } from '@/composables/useShortcuts'
import { getBackend } from '@/ipc/backend'
import { binStart, type BinsResult } from '@/ipc/bins'
import type { PulseInfo } from '@/ipc/bindings/PulseInfo'
import type { StreamView } from '@/ipc/bindings/StreamView'
import type { TriggerCondition } from '@/ipc/bindings/TriggerCondition'
import { onFrameDrawn, onNextFrame } from '@/motion/frames'
import { Canvas2DRenderer } from '@/render/canvas2d'
import type { WaveformRenderer } from '@/render/types'
import { WebGpuRenderer } from '@/render/webgpu'
import { readRenderTokens } from '@/render/tokens'
import type { FrameDescriptor, RenderTokens, TraceDraw } from '@/render/types'
import { useCaptureStore } from '@/stores/capture'
import { useChannelsStore } from '@/stores/channels'
import { useDecodeStore } from '@/stores/decode'
import { entryFormat } from '@/decode/numberFormat'
import { useSearchStore } from '@/stores/search'
import { useStreamStore } from '@/stores/stream'
import { usePreferencesStore } from '@/stores/preferences'
import { useViewStore } from '@/stores/view'
import type { ChannelGroup, ChannelRef } from '@/timeline/groups'
import { FLING_MIN_SPEED, flingStep, releaseSpeed, type PointerSample } from '@/timeline/fling'
import type { FloatAnchor, FloatBounds } from '@/timeline/float'
import { denseGap } from '@/render/shape'
import { BUSY_WIDTH, blockWidth, traceBlocks } from '@/timeline/blocks'
import {
  clickMeasure,
  leftStart,
  measureGeometry,
  nearestEdge,
  type MeasureEnd,
} from '@/timeline/measure'
import { covers, overscan, type SampleWindow } from '@/timeline/window'
import {
  clampTraceHeight,
  collapsedByDefault,
  fitRows,
  layoutRows,
  type PinnedRow,
  type Row,
} from '@/timeline/layout'
import { pulseMode, readPulse } from '@/timeline/pulse'
import { segmentGeometry } from '@/timeline/segment'
import { sampleToX, viewportEnd, xToSample } from '@/timeline/viewport'
import { wheelAction } from '@/timeline/wheel'
import { formatCount, formatFrequency, formatSampled, formatTime } from '@/utils/units'

import ChannelLabel from './ChannelLabel.vue'
import DecodeLane from './DecodeLane.vue'
import FloatingReadout from './FloatingReadout.vue'
import GroupLabel from './GroupLabel.vue'
import MeasureOverlay from './MeasureOverlay.vue'
import TimelineRuler from './TimelineRuler.vue'
import ChannelMenu from './ChannelMenu.vue'
import TraceGrip from './TraceGrip.vue'
import TriggerMenu from './TriggerMenu.vue'

/** Rows are never narrower than this, label column included; a narrower timeline scrolls. It
 * is what the timeline keeps beside the data stream (`TIMELINE_MIN`). */
const MIN_ROWS_WIDTH = 640
/** Height of the trigger menu, for keeping it inside the timeline. */
const MENU_HEIGHT = 236

const { t, locale } = useI18n()
const copy = useDecoderCopy()
const capture = useCaptureStore()
const channels = useChannelsStore()
const decode = useDecodeStore()
const stream = useStreamStore()
const finder = useSearchStore()
const preferences = usePreferencesStore()
const view = useViewStore()
const metrics = useRowMetrics()
const { pixelRatio } = useDevicePixelRatio()

const scroller = useTemplateRef<HTMLElement>('scroller')
const rowsElement = useTemplateRef<HTMLElement>('rows')
const canvas = useTemplateRef<HTMLCanvasElement>('canvas')

// Size and scroll position of the visible area.
const client = shallowRef({ width: 0, height: 0 })
const scroll = shallowRef({ top: 0, left: 0 })

/** A channel row whose edge is being dragged while the rows fit the timeline. */
const pinned = shallowRef<PinnedRow | null>(null)

const layout = computed(() => {
  const rows = layoutRows(
    capture.groups,
    metrics.value,
    (group) => channels.isCollapsed(group.key, collapsedByDefault(group)),
    (group) => Math.max(1, decode.statusOf(group.decoder)?.streams.length ?? 0),
    (channel) => channels.heights[channel.index],
    streamSchemaOf,
  )
  // The channel rows grow and shrink with the timeline's height, below the ruler (§6.1).
  if (!preferences.fitRows || client.value.height <= 0) return { ...rows, scale: 1 }
  return fitRows(rows, metrics.value, client.value.height - metrics.value.ruler, pinned.value)
})

const isTrace = (row: Row) => row.kind === 'trace' || row.kind === 'trace-narrow'

/** Channel rows, whose bottom edge in the label column can be dragged (DESIGN.md §6.1). */
const resizable = computed(() => layout.value.rows.filter((row) => isTrace(row) && row.channel))

function resizeRow(row: Row, height: number | null): void {
  if (!row.channel) return
  if (height === null || !preferences.fitRows) {
    channels.setHeight(
      row.channel.index,
      height === null ? null : clampTraceHeight(height, metrics.value),
    )
    return
  }
  // While the rows fit the timeline, the dragged row follows the pointer and the others make
  // room; the drag's end keeps the height it ends at.
  pinned.value = { index: row.channel.index, height: clampTraceHeight(height, metrics.value) }
}

/** The height a drag ended at, kept as the channel's own height before the rows were scaled. */
function endResize(): void {
  const drag = pinned.value
  if (!drag) return
  channels.setHeight(drag.index, clampTraceHeight(drag.height / layout.value.scale, metrics.value))
  pinned.value = null
}

/** The decoder's stream a row shows: the first on the group's title row, each further one (a
 * decoder stacked on the bus) on a lane of its own. */
function decodeStreamOf(row: Row): number | null {
  if (row.group.decoder === null) return null
  if (row.kind === 'header') return 0
  return row.kind === 'lane' ? (row.stream ?? null) : null
}

/** The node type whose events a stream carries, known before its viewport query returns. */
function streamKindOf(group: ChannelGroup, stream: number): string {
  return (
    decode.statusOf(group.decoder)?.streams[stream]?.kind ??
    (stream === 0 ? (group.type ?? '') : '')
  )
}

/** Shared contracts determine annotation lanes, including cJTAG and selected TAP streams. */
function streamSchemaOf(group: ChannelGroup, stream: number): string | null {
  return (
    decode.statusOf(group.decoder)?.streams[stream]?.schema ??
    decode.typeNamed(streamKindOf(group, stream))?.outputs[0]?.schema ??
    null
  )
}

/** The main events or the requested annotation lane, still belonging to the same stream. */
function decodeViewOf(row: Row): StreamView | undefined {
  const held = decode.streamView(row.group.decoder, row.stream ?? 0)
  return row.annotation
    ? held?.annotations?.find((annotation) => annotation.id === row.annotation)
    : held
}

/** Clicking a decoded block marks it as the segment and lists its stream in the data stream;
 * clicking it again lets go. */
function pickBlock(
  block: { id: string; start: number; end: number },
  decoder: number,
  streamIndex: number,
): void {
  if (view.segment?.blockId === block.id) {
    view.setSegment(null)
    return
  }
  view.setSegment({ from: block.start, to: block.end, blockId: block.id })
  stream.select(decoder, streamIndex)
}

/** `start..end` in track pixels, clipped to the view and at least 2 px wide. */
function bandOf(span: { start: number; end: number } | null) {
  if (!span) return null
  const { start, samplesPerPixel, width } = view.viewport
  const left = (span.start - start) / samplesPerPixel
  const right = (span.end - start) / samplesPerPixel
  if (right < 0 || left > width) return null
  return { left: Math.max(0, left), width: Math.max(2, Math.min(width, right) - Math.max(0, left)) }
}

/** The data stream row under the pointer, banded across the timeline. */
const hotBand = computed(() => bandOf(stream.hot))

/** The search hit shown, or where the level pattern was found. */
const findBand = computed(() => {
  const mark = finder.mark
  const band = bandOf(mark)
  if (!band || !mark) return null
  // A pattern marks one sample: a line centred on it.
  return mark.start === mark.end
    ? { ...band, left: band.left - 1, line: true }
    : { ...band, line: false }
})

/** A stacked decoder's lane is named after its node type. */
function laneLabel(row: Row): string | null {
  if (row.kind !== 'lane' || row.stream === undefined) return null
  if (row.annotation === 'states') return t('decoder.jtag.lane.states')
  const kind = decode.statusOf(row.group.decoder)?.streams[row.stream]?.kind
  if (kind === undefined) return null
  return copy.typeName(kind)
}

let resizeObserver: ResizeObserver | undefined

function measure(): void {
  const element = scroller.value
  if (!element) return
  const { clientWidth: width, clientHeight: height } = element
  const resized = width !== client.value.width || height !== client.value.height
  client.value = { width, height }
  const rowsWidth = Math.max(width, MIN_ROWS_WIDTH)
  view.setWidth(Math.max(0, rowsWidth - metrics.value.label))
  // The canvas takes its new size in the frame being painted, as the data stream comes or goes:
  // it is drawn before that, not a frame later, its old picture stretched or squeezed meanwhile.
  if (resized) void nextTick(drawQueuedFrame)
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

/** What the waveform needs: the visible samples at a column per device pixel. */
const binsWanted = computed(() => {
  const summary = capture.summary
  const viewport = view.viewport
  if (!summary || viewport.width <= 0 || traceChannels.value.length === 0) return null
  return {
    // Asks again as a running capture grows or, looping, moves on; columns past its samples
    // come back empty.
    key: `${summary.id}:${traceChannels.value.join(',')}:${capture.extent}:${capture.sampleCount + capture.dropped}`,
    captureId: summary.id,
    channels: traceChannels.value,
    extent: capture.extent,
    window: {
      start: viewport.start,
      end: viewportEnd(viewport),
      samplesPerPixel: viewport.samplesPerPixel / pixelRatio.value,
    },
  }
})

// Columns are fetched a view's width beyond each side, so panning draws from what is at hand;
// one request at a time, the latest wanted once it returns.
const bins = shallowRef<BinsResult | null>(null)
let binsFetched: { key: string; window: SampleWindow } | null = null
let binsLoading = false
let binsAgain = false

async function loadBins(): Promise<void> {
  const wanted = binsWanted.value
  if (!wanted) {
    bins.value = null
    binsFetched = null
    return
  }
  if (binsFetched?.key === wanted.key && covers(binsFetched.window, wanted.window, wanted.extent)) {
    return
  }
  if (binsLoading) {
    binsAgain = true
    return
  }
  binsLoading = true
  try {
    const window = overscan(wanted.window, wanted.extent)
    const count = Math.max(
      1,
      Math.min(
        Math.round((window.end - window.start) / window.samplesPerPixel),
        window.end - window.start,
      ),
    )
    if (window.end <= window.start) {
      bins.value = null
      binsFetched = null
      return
    }
    const result = await (
      await getBackend()
    ).bins({
      captureId: wanted.captureId,
      channels: wanted.channels,
      start: window.start,
      end: window.end,
      bins: count,
    })
    // A capture replaced meanwhile has other samples.
    if (binsWanted.value?.captureId !== wanted.captureId) return
    bins.value = result
    binsFetched = {
      key: wanted.key,
      window: { ...window, samplesPerPixel: (window.end - window.start) / count },
    }
  } catch (error) {
    console.error('bins request failed', error)
  } finally {
    binsLoading = false
    if (binsAgain) {
      binsAgain = false
      void loadBins()
    }
  }
}
watch(binsWanted, () => void loadBins(), { immediate: true })

// Canvas layer: WebGPU where it works and is wanted, else Canvas2D. A canvas holds one kind of
// context, so changing the preference or losing the GPU makes a new canvas.
let renderer: WaveformRenderer | null = null
let creating = false
const gpuLost = shallowRef(false)
const canvasKey = computed(() => `${preferences.graphics}:${gpuLost.value}`)

async function createRenderer(element: HTMLCanvasElement): Promise<void> {
  if (creating) return
  creating = true
  try {
    const gpu =
      preferences.graphics === 'auto' && !gpuLost.value
        ? await WebGpuRenderer.create(element, () => {
            gpuLost.value = true
          })
        : null
    const made = gpu ?? new Canvas2DRenderer(element)
    if (canvas.value !== element) {
      made.dispose()
      return
    }
    renderer = made
    preferences.activeRenderer = gpu ? 'webgpu' : 'canvas2d'
    requestDraw()
  } finally {
    creating = false
  }
}

watch(canvasKey, () => {
  renderer?.dispose()
  renderer = null
  requestDraw()
})
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
  const trackLeft = metrics.value.label - scroll.value.left
  const result = bins.value
  const traces: TraceDraw[] = []
  const cellsOf = (index: number) => result?.channels.get(index)
  for (const row of layout.value.rows) {
    if (!isTrace(row) || !row.channel || !result) continue
    const y = ruler + row.top - top
    if (y + row.height <= ruler || y >= height) continue
    const cells = cellsOf(row.channel.index)
    if (!cells) continue
    const blocks = traceBlocks(row.channel, row.group, cellsOf, denseGap(pixelRatio.value))
    // A line busy in view reads better as lines and blocks than with the wash between them.
    const columnX = (column: number) =>
      trackLeft + sampleToX(view.viewport, binStart(result, column))
    const busy = blockWidth(blocks, columnX, trackLeft, trackLeft + view.viewport.width)
    traces.push({
      top: y,
      height: row.height,
      wire: row.channel.wire,
      narrow: row.kind === 'trace-narrow',
      cells,
      blocks,
      wash: busy < BUSY_WIDTH,
    })
  }
  return {
    width,
    height,
    dpr: pixelRatio.value,
    trackLeft,
    clipTop: ruler,
    view: view.viewport,
    grid: view.ruler.ticks.filter((tick) => tick.major).map((tick) => tick.x),
    bins: result,
    traces,
  }
})

// Drawn once a frame, in the frame that moved the view when a glide, a fling or the wheel did.
let drawQueued = false
function requestDraw(): void {
  if (drawQueued) return
  drawQueued = true
  onFrameDrawn(drawQueuedFrame)
}

/** Draws what was asked for, unless it has been drawn already. */
function drawQueuedFrame(): void {
  if (drawQueued) draw()
}

function draw(): void {
  drawQueued = false
  const element = canvas.value
  if (!element || !tokens.value) return
  if (!renderer) {
    void createRenderer(element)
    return
  }
  renderer.draw(frame.value, tokens.value)
}
watch([frame, tokens], requestDraw)
onBeforeUnmount(() => {
  renderer?.dispose()
})

// The segment over the rows.
const segment = computed(() =>
  view.segment ? segmentGeometry(view.segment.from, view.segment.to, view.viewport) : null,
)

// Trigger menu: one channel at a time, beside the label column (DESIGN.md §7.1).
interface OpenMenu {
  channel: ChannelRef
  top: number
  slot: HTMLElement
}
const menu = shallowRef<OpenMenu | null>(null)
/** The channel whose context menu is open, and where. */
const channelMenu = shallowRef<{ channel: ChannelRef; x: number; y: number } | null>(null)

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

// Hover read-out, drag to pan and click to measure on the tracks.
interface Hover {
  row: Row
  pulse: PulseInfo
}
const hover = shallowRef<Hover | null>(null)
let hoverTarget: { row: Row; sample: number } | null = null
let hoverRequest = 0
/** Where the pointer is over the tracks, in track pixels. */
const pointer = shallowRef<{ row: Row; x: number } | null>(null)
let drag: { x: number; moves: PointerSample[] } | null = null
/** Where the button went down: released within a few pixels, it is a click. */
let press: { x: number; y: number; row: Row } | null = null
const CLICK_SLOP = 3
/** The fling under way; starting another or stopping replaces it. */
let fling: object | null = null

function stopFling(): void {
  fling = null
}

/** Keeps the waveform moving after a quick drag (DSView's quick scroll) until it slows down
 * or reaches an end. */
function startFling(speed: number): void {
  const current = {}
  fling = current
  let last = performance.now()
  const frame = (now: number) => {
    if (fling !== current) return
    const step = flingStep(speed, Math.max(0, now - last))
    last = now
    const before = view.viewport.start
    if (step) view.pan(-step.dx)
    if (!step || view.viewport.start === before) {
      fling = null
      return
    }
    speed = step.speed
    onNextFrame(frame)
  }
  onNextFrame(frame)
}
onBeforeUnmount(stopFling)

function trackX(event: PointerEvent): number {
  return event.clientX - (event.currentTarget as HTMLElement).getBoundingClientRect().left
}

function trackSample(event: PointerEvent): number {
  return Math.floor(xToSample(view.viewport, trackX(event)))
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
    drag.moves.push({ x: event.clientX, t: event.timeStamp })
    if (drag.moves.length > 32) drag.moves.splice(0, drag.moves.length - 32)
    return
  }
  pointer.value = { row, x: trackX(event) }
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
  pointer.value = null
}

function onTrackPointerDown(row: Row, event: PointerEvent): void {
  stopFling()
  if (event.button !== 0 || (event.target as Element).closest('button')) return
  drag = { x: event.clientX, moves: [{ x: event.clientX, t: event.timeStamp }] }
  press = { x: event.clientX, y: event.clientY, row }
  ;(event.currentTarget as HTMLElement).setPointerCapture(event.pointerId)
  onTrackPointerLeave()
}

function onTrackPointerUp(event: PointerEvent): void {
  const clicked =
    event.type === 'pointerup' &&
    press !== null &&
    Math.abs(event.clientX - press.x) <= CLICK_SLOP &&
    Math.abs(event.clientY - press.y) <= CLICK_SLOP
  if (drag && preferences.quickScroll && !clicked) {
    const speed = releaseSpeed(drag.moves, event.timeStamp)
    if (Math.abs(speed) >= FLING_MIN_SPEED) startFling(speed)
  }
  drag = null
  if (clicked && press) void onTrackClick(press.row, trackX(event))
  press = null
}

/** The first edge of a measurement under way on `row`, which the other end skips. */
function startOn(row: Row): number | null {
  const measure = view.measure
  return measure?.phase === 'measuring' && measure.from.rowKey === row.key
    ? measure.from.sample
    : null
}

/** The edge of the row's channel within reach of track x `x`, as a click picks it. */
async function edgeAt(row: Row, x: number): Promise<MeasureEnd | null> {
  const summary = capture.summary
  const viewport = view.viewport
  const sample = Math.floor(xToSample(viewport, x))
  if (!isTrace(row) || !row.channel || !summary || sample < 0 || sample >= capture.sampleCount) {
    return null
  }
  const pulse = await (await getBackend()).pulse(summary.id, row.channel.index, sample)
  const found = pulse && nearestEdge(pulse, x, viewport, startOn(row))
  return found == null ? null : { sample: found, rowKey: row.key, edge: true }
}

/** A click on the tracks starts, completes or lets go of the edge measurement (§9.6). */
async function onTrackClick(row: Row, x: number): Promise<void> {
  const before = view.measure
  const edge = await edgeAt(row, x)
  if (view.measure === before) view.setMeasure(clickMeasure(before, edge))
}

/** The edge under the pointer within reach, other than a measurement's first. */
const pointed = computed<MeasureEnd | null>(() => {
  const at = pointer.value
  const current = hover.value
  if (!at || !current || current.row.key !== at.row.key) return null
  const found = nearestEdge(current.pulse, at.x, view.viewport, startOn(at.row))
  return found === null ? null : { sample: found, rowKey: at.row.key, edge: true }
})

/** Rows by key, in the rows' coordinates (below the ruler). */
const rowBoxes = computed(
  () =>
    new Map(
      layout.value.rows.map((row) => [
        row.key,
        { top: metrics.value.ruler + row.top, height: row.height },
      ]),
    ),
)
const rowBox = (key: string) => rowBoxes.value.get(key) ?? null

/** The measurement as drawn: a finished one, or one whose second end follows the pointer. */
const measured = computed(() => {
  const measure = view.measure
  if (!measure) return null
  let to: MeasureEnd | null = measure.phase === 'done' ? measure.to : null
  if (measure.phase === 'measuring' && pointer.value) {
    const live = pointed.value ?? {
      sample: Math.round(xToSample(view.viewport, pointer.value.x)),
      rowKey: pointer.value.row.key,
      edge: false,
    }
    if (leftStart(measure.from, live, view.viewport)) to = live
  }
  const geometry = measureGeometry(measure.from, to ?? measure.from, view.viewport, rowBox)
  const hint = t('edgeMeasure.hint')
  if (!to) return { geometry, primary: null, secondary: hint, done: false }
  const samples = Math.abs(to.sample - measure.from.sample)
  const seconds = capture.samplerate > 0 ? samples / capture.samplerate : 0
  const primary = t('edgeMeasure.delta', { time: formatSampled(seconds, capture.samplerate) })
  if (!to.edge) return { geometry, primary, secondary: hint, done: false }
  const secondary =
    seconds > 0
      ? t('edgeMeasure.detail', {
          samples: formatCount(samples, locale.value),
          frequency: formatFrequency(1 / seconds, 1),
        })
      : t('edgeMeasure.sameSample')
  return { geometry, primary, secondary, done: measure.phase === 'done' }
})

/** The edge a click would start a measurement from: a ring on it. */
const target = computed(() => {
  const edge = pointed.value
  const box = edge && rowBox(edge.rowKey)
  if (!edge || !box || view.measure?.phase === 'measuring') return null
  return { x: sampleToX(view.viewport, edge.sample), y: box.top + box.height / 2 }
})

const readout = computed(() => {
  const current = hover.value
  const channel = current?.row.channel
  // While measuring, the measurement's read-out takes its place.
  if (!current || !channel || view.measure?.phase === 'measuring') return null
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
      duty: `${((reading.duty ?? 0) * 100).toFixed(1)}%`,
    })
  } else if (reading.bits !== null) secondary = t('hover.bits', { count: reading.bits })
  return {
    rowKey: current.row.key,
    channel,
    left: reading.left,
    width: reading.width,
    primary: t(reading.complete ? 'hover.pulse' : 'hover.pulseOpen', { level, width }),
    secondary,
  }
})

/** The settings float the read-outs over the rows instead of the status bar (DESIGN.md §11). */
const floating = computed(() => preferences.readoutPlace === 'float')

// The read-outs go to the status bar, where they cover no waveform (DESIGN.md §10), unless they
// float.
watch([readout, measured, floating], ([pulse, measure, float]) =>
  view.setReadouts(
    float
      ? { pulse: null, measure: null }
      : {
          pulse: pulse && {
            channel: pulse.channel,
            primary: pulse.primary,
            secondary: pulse.secondary,
          },
          measure: measure && {
            primary: measure.primary,
            secondary: measure.secondary,
            done: measure.done,
          },
        },
  ),
)
onBeforeUnmount(() => view.setReadouts({ pulse: null, measure: null }))

/** The visible tracks, which floating read-outs stay inside: right of the label column and
 * under the ruler. */
const floatBounds = computed<FloatBounds>(() => ({
  left: scroll.value.left,
  right: Math.min(
    view.viewport.width,
    scroll.value.left + client.value.width - metrics.value.label,
  ),
  top: scroll.value.top + metrics.value.ruler,
  bottom: scroll.value.top + client.value.height,
}))

/** A floating measurement read-out: centred on its line, over the row it ends on. */
const measureAnchor = computed<FloatAnchor | null>(() => {
  const geometry = measured.value?.geometry
  const line = geometry && (geometry.to ?? geometry.from)
  if (!floating.value || !geometry || !line) return null
  const span = geometry.span
  const x = span ? span.left + span.width / 2 : line.visible ? line.x : null
  if (x === null) return null
  return { x, align: 'centre', top: line.top, bottom: line.top + line.height, side: 'above' }
})

/** A floating pulse read-out: beside the pointer, over the row, or under it when the
 * measurement's read-out is over it. */
const pulseAnchor = computed<FloatAnchor | null>(() => {
  const current = readout.value
  const box = current && rowBox(current.rowKey)
  if (!floating.value || !current || !box) return null
  const at = pointer.value
  const x = at?.row.key === current.rowKey ? at.x : current.left + current.width / 2
  const side = measureAnchor.value?.top === box.top ? 'below' : 'above'
  return { x, align: 'beside', top: box.top, bottom: box.top + box.height, side }
})

// Keyboard navigation, unless a text field, menu or dialog has the keys (DESIGN.md §10): by
// default `+` and `-` zoom, `←` and `→` pan a tenth of the view (half with Shift), `Home` shows
// the whole capture and `Esc` lets go of the edge measurement, or else of the segment.
const pan = (share: number) => () => {
  stopFling()
  view.glidePan(view.viewport.width * share)
}
useShortcuts({
  zoomIn: () => {
    stopFling()
    view.glideZoom(2)
  },
  zoomOut: () => {
    stopFling()
    view.glideZoom(0.5)
  },
  fit: () => {
    stopFling()
    view.fit()
  },
  panLeft: pan(-0.1),
  panRight: pan(0.1),
  panLeftFar: pan(-0.5),
  panRightFar: pan(0.5),
  clear: () => {
    if (view.measure) view.setMeasure(null)
    else if (view.segment) view.setSegment(null)
    else return false
  },
})

// Trackpads send several wheel events a frame: gather them and move the view once per frame.
const wheel = { pan: 0, zoom: 1, anchor: 0, queued: false }

function applyWheel(): void {
  wheel.queued = false
  if (wheel.zoom !== 1) view.zoom(wheel.zoom, wheel.anchor)
  if (wheel.pan !== 0) view.pan(wheel.pan)
  wheel.zoom = 1
  wheel.pan = 0
}

function queueWheel(): void {
  if (wheel.queued) return
  wheel.queued = true
  onNextFrame(applyWheel)
}

/** Track x of the pointer, within the track. */
function anchorOf(clientX: number): number | null {
  const rows = rowsElement.value?.getBoundingClientRect()
  if (!rows) return null
  return Math.min(Math.max(clientX - rows.left - metrics.value.label, 0), view.viewport.width)
}

const controlHeld = useControlHeld()

// The wheel zooms around the pointer or pans (DESIGN.md §10); what it leaves alone scrolls the
// rows. A wheel's notches glide (§13); a trackpad follows the fingers.
function onWheel(event: WheelEvent): void {
  stopFling()
  const visible = scroller.value?.getBoundingClientRect()
  const anchor = anchorOf(event.clientX)
  if (!visible || anchor === null) return
  const action = wheelAction(event, {
    wheelZoom: preferences.wheelZoom,
    overLabels: event.clientX - visible.left < metrics.value.label,
    controlHeld: controlHeld(),
  })
  if (!action) return
  event.preventDefault()
  if (action.kind === 'zoom' && action.notched) {
    view.glideZoom(action.factor, anchor)
    return
  }
  if (action.kind === 'zoom') {
    wheel.zoom *= action.factor
    wheel.anchor = anchor
  } else {
    wheel.pan += action.dx
  }
  queueWheel()
}

// WebKit on macOS sends a trackpad pinch as gesture events rather than as Ctrl + wheel: each
// reports the scale since the pinch began.
interface GestureEvent extends UIEvent {
  scale: number
  clientX: number
}
let gestureScale = 1

useEventListener(scroller, 'gesturestart', (event: Event) => {
  event.preventDefault()
  stopFling()
  gestureScale = 1
})
useEventListener(scroller, 'gesturechange', (event: Event) => {
  event.preventDefault()
  const gesture = event as GestureEvent
  const anchor = anchorOf(gesture.clientX)
  if (anchor === null || !(gesture.scale > 0)) return
  wheel.zoom *= gesture.scale / gestureScale
  wheel.anchor = anchor
  gestureScale = gesture.scale
  queueWheel()
})
</script>

<template>
  <div class="relative min-h-0 flex-1 bg-canvas">
    <canvas
      :key="canvasKey"
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
      <div ref="rows" class="relative flex min-h-full min-w-[640px] flex-col overflow-x-clip">
        <TimelineRuler class="sticky top-0 z-[5]" />

        <!-- The channel rows' bottom edges, across the boundary with the row below: over the
             labels, under the ruler. -->
        <div class="sticky left-0 z-[4] h-0 w-(--lab) flex-none">
          <TraceGrip
            v-for="row in resizable"
            :key="row.key"
            :channel="row.channel?.label ?? ''"
            :height="row.height"
            :style="{ top: `${row.top + row.height - 3}px` }"
            @resize="(height) => resizeRow(row, height)"
            @end="endResize"
            @reset="resizeRow(row, null)"
          />
        </div>

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
            <span v-else-if="laneLabel(row)" class="ml-6 truncate text-small text-ink-3">{{
              laneLabel(row)
            }}</span>
            <ChannelLabel
              v-else-if="row.labeled && row.channel"
              :channel="row.channel"
              :condition="channels.triggerOf(row.channel.index)"
              :menu-open="menu?.channel.index === row.channel.index"
              @trigger="(slot) => row.channel && toggleMenu(row.channel, slot)"
              @menu="(at) => row.channel && (channelMenu = { channel: row.channel, ...at })"
            />
          </div>
          <div
            class="relative min-w-0 flex-1"
            :class="view.measure?.phase === 'measuring' && 'cursor-crosshair'"
            @pointerdown="onTrackPointerDown(row, $event)"
            @pointermove="onTrackPointerMove(row, $event)"
            @pointerup="onTrackPointerUp"
            @pointercancel="onTrackPointerUp"
            @pointerleave="onTrackPointerLeave"
          >
            <!-- The title row shows the decoder's first stream, a lane row the one it is for. -->
            <DecodeLane
              v-if="row.group.decoder !== null && decodeStreamOf(row) !== null"
              :kind="streamKindOf(row.group, row.stream ?? 0)"
              :view="decodeViewOf(row)"
              :viewport="view.viewport"
              :decoder="row.group.decoder"
              :stream="row.stream ?? 0"
              :annotation="row.annotation"
              :selected="view.segment?.blockId ?? null"
              :time-of="capture.timeOf"
              :format="entryFormat(decode.entries[row.group.decoder])"
              @pick="
                (block) =>
                  row.group.decoder !== null && pickBlock(block, row.group.decoder, row.stream ?? 0)
              "
            />
            <i
              v-if="readout && readout.rowKey === row.key"
              class="pointer-events-none absolute top-[3px] bottom-[3px] rounded-2 bg-[color-mix(in_srgb,var(--ink)_13%,transparent)]"
              :style="{ left: `${readout.left}px`, width: `${Math.max(readout.width, 1)}px` }"
              aria-hidden="true"
            />
          </div>
        </div>

        <div class="flex flex-[1_0_0]">
          <div class="sticky left-0 z-[3] w-(--lab) flex-none border-r border-line bg-chrome" />
        </div>

        <div
          v-if="findBand"
          class="pointer-events-none absolute inset-y-0 right-0 left-(--lab) z-[2] overflow-hidden"
          aria-hidden="true"
        >
          <div
            class="absolute inset-y-0"
            :class="
              findBand.line
                ? 'bg-ink'
                : 'bg-[color-mix(in_srgb,var(--find-strong)_45%,transparent)] shadow-[inset_1px_0_0_var(--ink-3),inset_-1px_0_0_var(--ink-3)]'
            "
            :style="{ left: `${findBand.left}px`, width: `${findBand.width}px` }"
          />
        </div>
        <div
          v-if="hotBand"
          class="pointer-events-none absolute inset-y-0 right-0 left-(--lab) z-[2]"
          aria-hidden="true"
        >
          <div
            class="absolute inset-y-0 bg-[color-mix(in_srgb,var(--ink)_8%,transparent)] shadow-[inset_1px_0_0_var(--ink-3),inset_-1px_0_0_var(--ink-3)]"
            :style="{ left: `${hotBand.left}px`, width: `${hotBand.width}px` }"
          />
        </div>
        <div
          v-if="segment"
          class="pointer-events-none absolute inset-y-0 right-0 left-(--lab) z-[2]"
          aria-hidden="true"
        >
          <div
            v-if="segment && segment.band"
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

        <MeasureOverlay
          v-if="measured || target"
          :geometry="measured?.geometry ?? null"
          :target="target"
        />

        <div
          v-if="measureAnchor || pulseAnchor"
          class="pointer-events-none absolute inset-y-0 right-0 left-(--lab) z-[4] overflow-hidden"
        >
          <FloatingReadout
            v-if="measured && measureAnchor"
            data-part="measure"
            :anchor="measureAnchor"
            :bounds="floatBounds"
            :primary="measured.primary"
            :secondary="measured.secondary"
            :announce="measured.done"
          />
          <FloatingReadout
            v-if="readout && pulseAnchor"
            data-part="pulse"
            :anchor="pulseAnchor"
            :bounds="floatBounds"
            :primary="readout.primary"
            :secondary="readout.secondary"
          />
        </div>

        <Transition name="pop">
          <ChannelMenu
            v-if="channelMenu"
            :key="channelMenu.channel.index"
            :channel="channelMenu.channel"
            :x="channelMenu.x"
            :y="channelMenu.y"
            @close="channelMenu = null"
          />
        </Transition>
        <!-- Keyed by channel: moving to another row's menu closes one and opens the other. -->
        <Transition name="pop">
          <TriggerMenu
            v-if="menu"
            :key="menu.channel.index"
            :channel="menu.channel.label"
            :wire="menu.channel.wire"
            :condition="channels.triggerOf(menu.channel.index)"
            :top="menu.top"
            @pick="pick"
            @close="closeMenu(true)"
          />
        </Transition>
      </div>
    </div>
  </div>
</template>
