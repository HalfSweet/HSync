/**
 * Which stretches of a trace are drawn as blocks (DESIGN.md §9.6): where it changes too often to
 * draw edge by edge, as an SPI clock does within a byte; an SPI data line takes the bytes of its
 * clock in which it changes, so both show the same byte blocks.
 */
import { busyRuns, changingIn, type ColumnRun } from '@/render/shape'

import type { ChannelGroup, ChannelRef } from './groups'

/** Whether a decoder role is a clock line, whose idle level runs through its blocks. */
export function isClockRole(role: string | undefined): boolean {
  return role !== undefined && /(^|_)(s?clk|sck|scl|tck|swclk|b?clock|bck|bclk|mclk)$/i.test(role)
}

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
