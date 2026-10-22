/**
 * Rows of the timeline and where they sit (DESIGN.md §6.1). The DOM label column and the
 * waveform canvas both use this layout, so neither has to measure the other.
 */
import type { ChannelGroup, ChannelRef } from './groups'
import { typeOf } from '@/decode/catalog'

export type RowKind = 'gap' | 'header' | 'lane' | 'trace' | 'trace-narrow'

export interface Row {
  key: string
  kind: RowKind
  /** Offset from the top of the rows, below the ruler, in CSS pixels. */
  top: number
  height: number
  group: ChannelGroup
  /** The channel the row decodes or draws. */
  channel?: ChannelRef
  /** Whether the label column shows the channel (raw rows under SPI data lanes do not). */
  labeled: boolean
  /** On `lane` rows, the position of the decoder's stream the row shows. */
  stream?: number
  /** A separate annotation lane within that stream, such as JTAG's TAP states. */
  annotation?: string
  /** A 1 px line above the row separates it from the previous group. */
  separator: boolean
}

/** Row heights from the density tokens (`--h-lane`, `--h-tr`, `--h-trc`, `--gap`). */
export interface RowMetrics {
  lane: number
  trace: number
  traceNarrow: number
  gap: number
}

export const COMFORTABLE_METRICS: RowMetrics = { lane: 26, trace: 30, traceNarrow: 18, gap: 8 }

/** The CAN group keeps a fixed 28 px header and 6 px gaps in the design. */
const CAN_HEADER = 28
const CAN_GAP = 6

/** The most a channel row grows to, however far its edge is dragged (DESIGN.md §6.1). */
export const MAX_TRACE_HEIGHT = 200

/** A height given to a channel row, in whole pixels from the narrow row's height to the most. */
export function clampTraceHeight(height: number, metrics: RowMetrics): number {
  return Math.min(MAX_TRACE_HEIGHT, Math.max(metrics.traceNarrow, Math.round(height)))
}

/** Groups collapsed until the user opens them: the design starts with CAN-FD closed. */
export function collapsedByDefault(group: ChannelGroup): boolean {
  return group.kind === 'can'
}

/**
 * Lays out the rows: for each group its title row, which carries the decoder's first stream,
 * a lane row for each further stream (`streamsOf` says how many a decoder has), with a state
 * annotation row after each JTAG-schema stream, then its channels, each as
 * high as `heightOf` says (the user dragged its edge) or else its kind's height.
 */
export function layoutRows(
  groups: ChannelGroup[],
  metrics: RowMetrics,
  isCollapsed: (group: ChannelGroup) => boolean,
  streamsOf: (group: ChannelGroup) => number = () => 1,
  heightOf: (channel: ChannelRef) => number | undefined = () => undefined,
  streamSchemaOf: (group: ChannelGroup, stream: number) => string | null = (group, stream) =>
    stream === 0 ? (typeOf(group.type ?? '')?.outputs[0]?.schema ?? null) : null,
): { rows: Row[]; height: number } {
  const rows: Row[] = []
  let top = 0
  const push = (row: Omit<Row, 'top' | 'separator' | 'labeled'> & Partial<Row>) => {
    rows.push({ separator: false, labeled: false, ...row, top })
    top += row.height
  }

  groups.forEach((group, position) => {
    const can = group.kind === 'can'
    const gap = can ? CAN_GAP : metrics.gap
    push({ key: `${group.key}:before`, kind: 'gap', height: gap, group, separator: position > 0 })
    push({
      key: `${group.key}:header`,
      kind: 'header',
      height: can ? CAN_HEADER : metrics.lane,
      group,
    })
    if (!isCollapsed(group)) {
      // The decoder's first stream sits on the title row; a decoder stacked on it (the
      // further streams) gets a lane of its own.
      const streams = group.decoder === null ? 0 : streamsOf(group)
      for (let stream = 0; stream < streams; stream++) {
        if (stream > 0) {
          push({
            key: `${group.key}:stream:${stream}`,
            kind: 'lane',
            height: metrics.lane,
            group,
            stream,
          })
        }
        if (streamSchemaOf(group, stream) === 'jtag.events') {
          push({
            key: `${group.key}:stream:${stream}:annotation:states`,
            kind: 'lane',
            height: metrics.lane,
            group,
            stream,
            annotation: 'states',
          })
        }
      }
      for (const channel of group.channels) {
        const wide = group.kind === 'i2c' || group.kind === 'uart'
        const own = heightOf(channel)
        push({
          key: `${group.key}:trace:${channel.index}`,
          kind: wide ? 'trace' : 'trace-narrow',
          height:
            own === undefined
              ? metrics[wide ? 'trace' : 'traceNarrow']
              : clampTraceHeight(own, metrics),
          group,
          channel,
          labeled: true,
        })
      }
    }
    push({ key: `${group.key}:after`, kind: 'gap', height: gap, group })
  })
  return { rows, height: top }
}

/** The most the channel rows grow to fit the window: three times their own heights. */
export const MAX_FIT_SCALE = 3

/** A channel row a drag is resizing, which keeps the height the pointer gives it. */
export interface PinnedRow {
  index: number
  height: number
}

const isChannelRow = (row: Row) =>
  (row.kind === 'trace' || row.kind === 'trace-narrow') && row.channel !== undefined

/**
 * Fits the channel rows of `layout` into `room` pixels of height (DESIGN.md §6.1): every channel
 * row is scaled by one factor, from what makes a default row as low as a narrow one up to
 * MAX_FIT_SCALE, no row lower than a narrow one or higher than MAX_TRACE_HEIGHT. Within those
 * bounds the rows fill the room exactly, in whole pixels; beyond them the rows scroll, or leave
 * room below. Group titles, decode lanes and gaps keep their heights, and a `pinned` row the
 * height a drag gives it while the others fit around it. `scale` is the factor the others got.
 */
export function fitRows(
  layout: { rows: Row[]; height: number },
  metrics: RowMetrics,
  room: number,
  pinned: PinnedRow | null = null,
): { rows: Row[]; height: number; scale: number } {
  const isPinned = (row: Row) => isChannelRow(row) && row.channel?.index === pinned?.index
  /** What a row keeps when it is not fitted: the drag's height, or its own. */
  const kept = (row: Row) => (pinned && isPinned(row) ? pinned.height : row.height)
  const fitted = layout.rows.filter((row) => isChannelRow(row) && !isPinned(row))
  const fixed = layout.rows.reduce(
    (sum, row) => (isChannelRow(row) && !isPinned(row) ? sum : sum + kept(row)),
    0,
  )
  const space = room - fixed
  const heightAt = (base: number, scale: number) =>
    Math.min(MAX_TRACE_HEIGHT, Math.max(metrics.traceNarrow, base * scale))
  const total = (scale: number) => fitted.reduce((sum, row) => sum + heightAt(row.height, scale), 0)

  const low = metrics.traceNarrow / metrics.trace
  let scale = 1
  let exact = false
  if (fitted.length > 0) {
    if (total(low) >= space) scale = low
    else if (total(MAX_FIT_SCALE) <= space) scale = MAX_FIT_SCALE
    else {
      // The total grows with the factor: halve the interval until it is well under a pixel.
      let [from, to] = [low, MAX_FIT_SCALE]
      for (let step = 0; step < 40; step++) {
        const middle = (from + to) / 2
        if (total(middle) < space) from = middle
        else to = middle
      }
      scale = (from + to) / 2
      exact = true
    }
  }

  // Whole pixels; filling the room exactly, the pixels left over go to the rows that lost most.
  const heights = new Map<Row, number>()
  const wanted = fitted.map((row) => heightAt(row.height, scale))
  if (exact) {
    const floors = wanted.map(Math.floor)
    let left = space - floors.reduce((sum, height) => sum + height, 0)
    const order = wanted
      .map((height, at) => ({ at, part: height - Math.floor(height) }))
      .filter(({ at }) => (floors[at] as number) < MAX_TRACE_HEIGHT)
      .sort((a, b) => b.part - a.part)
    for (const { at } of order) {
      if (left <= 0) break
      floors[at] = (floors[at] as number) + 1
      left -= 1
    }
    fitted.forEach((row, at) => heights.set(row, floors[at] as number))
  } else {
    fitted.forEach((row, at) => heights.set(row, Math.round(wanted[at] as number)))
  }

  let top = 0
  const rows = layout.rows.map((row) => {
    const height = heights.get(row) ?? kept(row)
    const placed = { ...row, top, height }
    top += height
    return placed
  })
  return { rows, height: top, scale }
}

/** Rows that overlap `from..to` (offsets below the ruler). */
export function visibleRows(rows: readonly Row[], from: number, to: number): Row[] {
  return rows.filter((row) => row.top < to && row.top + row.height > from)
}
