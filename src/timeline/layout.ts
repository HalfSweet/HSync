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
  /** Decoder lane on `lane` rows, e.g. `bytes`, `miso`. */
  lane?: string
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

/** A lane of annotations a decoder of the group puts out (`decoder id/row id`). */
export interface AnnotationLaneRef {
  lane: string
  desc: string
}

export function layoutRows(
  groups: ChannelGroup[],
  metrics: RowMetrics,
  isCollapsed: (group: ChannelGroup) => boolean,
  annotationLanes: (group: ChannelGroup) => readonly AnnotationLaneRef[] = () => [],
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
      if (group.kind === 'i2c') {
        push({
          key: `${group.key}:lane:bytes`,
          kind: 'lane',
          height: metrics.lane,
          group,
          lane: 'bytes',
        })
      }
      // Annotation rows: a decoder without a native presentation shows its first row on the
      // title row, the rest (and those of stacked decoders) on rows of their own.
      const annotations = annotationLanes(group)
      for (const [position, lane] of annotations.entries()) {
        if (group.kind === 'other' && position === 0) continue
        push({
          key: `${group.key}:lane:${lane.lane}`,
          kind: 'lane',
          height: metrics.lane,
          group,
          lane: lane.lane,
        })
      }
      for (const channel of group.channels) {
        const wide = group.kind === 'i2c' || group.kind === 'uart'
        const dataLine =
          group.kind === 'spi' && (channel.role === 'miso' || channel.role === 'mosi')
        if (dataLine) {
          push({
            key: `${group.key}:lane:${channel.index}`,
            kind: 'lane',
            height: metrics.lane,
            group,
            channel,
            labeled: true,
            lane: channel.role,
          })
          push({
            key: `${group.key}:raw:${channel.index}`,
            kind: 'trace-narrow',
            height: metrics.traceNarrow,
            group,
            channel,
          })
        } else {
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
    }
    push({ key: `${group.key}:after`, kind: 'gap', height: gap, group })
  })
  return { rows, height: top }
}

/** Rows that overlap `from..to` (offsets below the ruler). */
export function visibleRows(rows: readonly Row[], from: number, to: number): Row[] {
  return rows.filter((row) => row.top < to && row.top + row.height > from)
}
