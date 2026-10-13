/**
 * Rows of the timeline and where they sit (DESIGN.md §6.1). The DOM label column and the
 * waveform canvas both use this layout, so neither has to measure the other.
 */
import type { ChannelGroup, ChannelRef } from './groups'

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

/** Groups collapsed until the user opens them: the design starts with CAN-FD closed. */
export function collapsedByDefault(group: ChannelGroup): boolean {
  return group.kind === 'can'
}

/**
 * Lays out the rows: for each group its title row, which carries the decoder's first stream,
 * a lane row for each further stream (`streamsOf` says how many a decoder has), then its
 * channels.
 */
export function layoutRows(
  groups: ChannelGroup[],
  metrics: RowMetrics,
  isCollapsed: (group: ChannelGroup) => boolean,
  streamsOf: (group: ChannelGroup) => number = () => 1,
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
      for (let stream = 1; stream < streams; stream++) {
        push({
          key: `${group.key}:stream:${stream}`,
          kind: 'lane',
          height: metrics.lane,
          group,
          stream,
        })
      }
      for (const channel of group.channels) {
        const wide = group.kind === 'i2c' || group.kind === 'uart'
        push({
          key: `${group.key}:trace:${channel.index}`,
          kind: wide ? 'trace' : 'trace-narrow',
          height: wide ? metrics.trace : metrics.traceNarrow,
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

/** Rows that overlap `from..to` (offsets below the ruler). */
export function visibleRows(rows: readonly Row[], from: number, to: number): Row[] {
  return rows.filter((row) => row.top < to && row.top + row.height > from)
}
