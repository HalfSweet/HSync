import { describe, expect, it } from 'vitest'

import { CELL_FIRST_HIGH, CELL_LAST_HIGH, CELL_TRANSITION } from '@/ipc/bins'

import { frameRects, parseColor, RECT_FLOATS } from './rects'
import type { FrameDescriptor, RenderTokens } from './types'

const tokens: RenderTokens = {
  grid: 'rgb(23 27 31 / 0.07)',
  wires: Array.from({ length: 8 }, () => '#000000'),
  washes: Array.from({ length: 8 }, () => 'rgb(0 0 0 / 10%)'),
  blocks: Array.from({ length: 8 }, () => 'rgb(0 0 0 / 34%)'),
}

describe('waveform rectangles', () => {
  it('read token colours', () => {
    expect(parseColor('#ff8000')).toEqual([1, 128 / 255, 0, 1])
    expect(parseColor('rgb(23 27 31 / 0.07)')[3]).toBeCloseTo(0.07)
    expect(parseColor('rgb(0, 0, 0)')).toEqual([0, 0, 0, 1])
    expect(parseColor('rgb(0 0 0 / 10%)')[3]).toBeCloseTo(0.1)
  })

  it('draw a pulse as its wash and three line pieces', () => {
    const high = CELL_FIRST_HIGH | CELL_LAST_HIGH
    const frame: FrameDescriptor = {
      width: 100,
      height: 50,
      dpr: 1,
      trackLeft: 0,
      clipTop: 0,
      view: { start: 0, samplesPerPixel: 1, width: 4 },
      grid: [2],
      bins: { start: 0, end: 4, bins: 4, channels: new Map() },
      traces: [
        {
          top: 0,
          height: 30,
          wire: 0,
          narrow: false,
          cells: new Uint8Array([0, high, high | CELL_TRANSITION, 0]),
          blocks: [],
          wash: true,
        },
      ],
    }
    const { data, count } = frameRects(frame, tokens)
    // Grid line, then the wash, then line pieces.
    expect(count).toBeGreaterThanOrEqual(5)
    expect(data[0]).toBe(2)
    const wash = Array.from(data.slice(RECT_FLOATS, RECT_FLOATS + 4))
    expect(wash[0]).toBe(1)
    expect(wash[2]).toBeGreaterThan(0)
  })
})
