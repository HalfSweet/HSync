/**
 * Which stretches of a trace are drawn as blocks (DESIGN.md §6.3): where it changes too often to
 * draw edge by edge, as an SPI clock does within a byte; an SPI data line takes the bytes of its
 * clock in which it changes, so both show the same byte blocks.
 */
import { busyRuns, changingIn, type ColumnRun } from '@/render/shape'

import type { ChannelGroup, ChannelRef } from './groups'

/**
 * The column runs of `channel` to draw as blocks, given the cells of each channel and the quiet
 * columns a run may span ({@link denseGap}).
 */
export function traceBlocks(
  channel: ChannelRef,
  group: ChannelGroup,
  cellsOf: (index: number) => Uint8Array | undefined,
  gap = 1,
): ColumnRun[] {
  const cells = cellsOf(channel.index)
  if (!cells) return []
  if (group.kind === 'spi' && (channel.role === 'miso' || channel.role === 'mosi')) {
    const clock = group.channels.find((other) => other.role === 'clk')
    const clockCells = clock ? cellsOf(clock.index) : undefined
    if (clockCells) return changingIn(cells, busyRuns(clockCells, gap))
  }
  return busyRuns(cells, gap)
}

/** Blocks wider than this in all, in CSS pixels, make a line busy: drawn without the wash. */
export const BUSY_WIDTH = 8

/** How wide the blocks of `blocks` are within `left..right`, given each column's left edge. */
export function blockWidth(
  blocks: readonly ColumnRun[],
  x: (column: number) => number,
  left: number,
  right: number,
): number {
  return blocks.reduce(
    (sum, [from, to]) => sum + Math.max(0, Math.min(x(to), right) - Math.max(x(from), left)),
    0,
  )
}
