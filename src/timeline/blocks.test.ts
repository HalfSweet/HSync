import { describe, expect, it } from 'vitest'

import { CELL_FIRST_HIGH, CELL_LAST_HIGH, CELL_TRANSITION } from '@/ipc/bins'

import { isClockRole, traceBlocks } from './blocks'
import type { ChannelGroup } from './groups'

const T = CELL_TRANSITION
const HIGH = CELL_FIRST_HIGH | CELL_LAST_HIGH

const spi: ChannelGroup = {
  key: '1:spi',
  kind: 'spi',
  title: 'SPI',
  unnamed: false,
  options: {},
  decoder: 0,
  channels: [
    { index: 12, label: 'CLK', numeric: false, wire: 4, role: 'clk' },
    { index: 15, label: 'MISO', numeric: false, wire: 7, role: 'miso' },
    { index: 14, label: 'MOSI', numeric: false, wire: 6, role: 'mosi' },
  ],
}

describe('trace blocks', () => {
  it('know clock roles', () => {
    expect(['clk', 'scl', 'sck', 'tck', 'swclk', 'bck', 'clock'].every(isClockRole)).toBe(true)
    expect(['sda', 'miso', 'rx', 'cs', undefined].some(isClockRole)).toBe(false)
  })

  it('give SPI data lines the bytes of their clock in which they change', () => {
    const cells = new Map<number, Uint8Array>([
      // Two bursts of clock edges.
      [12, Uint8Array.of(0, T, T, T, 0, 0, T, T, T, 0)],
      // MISO changes in the first byte only, MOSI stays high.
      [15, Uint8Array.of(0, T, 0, HIGH, HIGH, HIGH, HIGH, HIGH, HIGH, HIGH)],
      [14, Uint8Array.of(HIGH, HIGH, HIGH, HIGH, HIGH, HIGH, HIGH, HIGH, HIGH, HIGH)],
    ])
    const of = (index: number) => cells.get(index)
    const [clk, miso, mosi] = spi.channels
    expect(clk && traceBlocks(clk, spi, of)).toEqual([
      [1, 4],
      [6, 9],
    ])
    expect(miso && traceBlocks(miso, spi, of)).toEqual([[1, 4]])
    expect(mosi && traceBlocks(mosi, spi, of)).toEqual([])
  })
})
