import { describe, expect, it } from 'vitest'

import { CELL_FIRST_HIGH, CELL_LAST_HIGH, CELL_TRANSITION } from '@/ipc/bins'

import { analogSpans, frameRects, layoutFrame, parseColor, RECT_FLOATS } from './rects'
import type { FrameDescriptor, RenderTokens, TraceDraw } from './types'

const tokens: RenderTokens = {
  grid: 'rgb(23 27 31 / 0.07)',
  axis: '#808080',
  wires: Array.from({ length: 8 }, () => '#000000'),
  washes: Array.from({ length: 8 }, () => 'rgb(0 0 0 / 10%)'),
  blocks: Array.from({ length: 8 }, () => 'rgb(0 0 0 / 34%)'),
}

/** Colours of `tokens` as the characters {@link raster} paints them with. */
const PAINT = new Map([
  ['#000000', 'L'],
  ['rgb(0 0 0 / 10%)', 'w'],
  ['rgb(0 0 0 / 34%)', 'b'],
  ['rgb(23 27 31 / 0.07)', 'g'],
  ['#808080', 'a'],
])

/** One trace over eight one-sample columns, eight CSS pixels high, at 2× device pixels. */
function pictureFrame(trace: Pick<TraceDraw, 'cells' | 'blocks'>): FrameDescriptor {
  return {
    width: 8,
    height: 8,
    dpr: 2,
    trackLeft: 0,
    clipTop: 0,
    view: { start: 0, samplesPerPixel: 1, width: 8 },
    grid: [],
    bins: { start: 0, end: 8, bins: 8, channels: new Map() },
    traces: [{ top: 0, height: 8, wire: 0, narrow: true, wash: true, ...trace }],
    analogBins: null,
    analog: [],
  }
}

/** Device-pixel rows `from..to` of a frame's rectangles, painted in order, one character a pixel. */
function raster(frame: FrameDescriptor, from: number, to: number): string[] {
  const width = Math.round(frame.width * frame.dpr)
  const rows = Array.from({ length: Math.round(frame.height * frame.dpr) }, () =>
    Array.from({ length: width }, () => '.'),
  )
  layoutFrame(frame, tokens, (x, y, w, h, color) => {
    for (let row = Math.max(0, y); row < Math.min(rows.length, y + h); row++)
      for (let column = Math.max(0, x); column < Math.min(width, x + w); column++)
        rows[row]![column] = PAINT.get(color) ?? '?'
  })
  return rows.slice(from, to + 1).map((row) => row.join(''))
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
      analogBins: null,
      analog: [],
    }
    const { data, count } = frameRects(frame, tokens)
    // Grid line, then the wash, then line pieces.
    expect(count).toBeGreaterThanOrEqual(5)
    expect(data[0]).toBe(2)
    const wash = Array.from(data.slice(RECT_FLOATS, RECT_FLOATS + 4))
    expect(wash[0]).toBe(1)
    expect(wash[2]).toBeGreaterThan(0)
  })

  it('end the wash of a pulse flush with the low line beside it', () => {
    const high = CELL_FIRST_HIGH | CELL_LAST_HIGH
    const cells = Uint8Array.of(0, 0, high, high, high, 0, 0, 0)
    // Lines are two device pixels; the wash fills the pulse down to the low line's bottom edge.
    expect(raster(pictureFrame({ cells, blocks: [] }), 2, 13)).toEqual([
      '................',
      '...LLLLLLLL.....',
      '...LLLLLLLL.....',
      '...LLwwwwLL.....',
      '...LLwwwwLL.....',
      '...LLwwwwLL.....',
      '...LLwwwwLL.....',
      '...LLwwwwLL.....',
      '...LLwwwwLL.....',
      'LLLLLwwwwLLLLLLL',
      'LLLLLwwwwLLLLLLL',
      '................',
    ])
  })

  it('outline a block on all four sides, its fill inside', () => {
    const T = CELL_TRANSITION
    const cells = Uint8Array.of(0, 0, T, T, T, 0, 0, 0)
    expect(raster(pictureFrame({ cells, blocks: [[2, 5]] }), 2, 13)).toEqual([
      '................',
      '...LLLLLLLL.....',
      '...LLLLLLLL.....',
      '...LLbbbbLL.....',
      '...LLbbbbLL.....',
      '...LLbbbbLL.....',
      '...LLbbbbLL.....',
      '...LLbbbbLL.....',
      '...LLbbbbLL.....',
      'LLLLLLLLLLLLLLLL',
      'LLLLLLLLLLLLLLLL',
      '................',
    ])
  })

  it('give a row made taller the extra height as swing, keeping its margins', () => {
    const high = CELL_FIRST_HIGH | CELL_LAST_HIGH
    /** The device-pixel rows of a pulse's high and low lines in a row `height` high. */
    const levels = (height: number, narrow: boolean) => {
      const frame: FrameDescriptor = {
        ...pictureFrame({ cells: Uint8Array.of(0, 0, high, high, high, 0, 0, 0), blocks: [] }),
        height,
        dpr: 1,
      }
      frame.traces[0] = { ...frame.traces[0]!, height, narrow }
      const rows = new Set<number>()
      layoutFrame(frame, tokens, (_x, y, w, h, color) => {
        if (color === '#000000' && w > h) rows.add(y)
      })
      return [...rows].sort((a, b) => a - b)
    }
    // At the design's heights the levels sit where they always did.
    expect(levels(30, false)).toEqual([7, 24])
    expect(levels(24, false)).toEqual([5, 19])
    expect(levels(18, true)).toEqual([5, 14])
    // Taller rows keep those margins.
    expect(levels(100, false)).toEqual([7, 94])
    expect(levels(100, true)).toEqual([5, 96])
  })

  it('join analog columns with straight lines and hold the last sample', () => {
    // Three samples four pixels apart: 0 V, 1 V, 2 V.
    const { low, high } = analogSpans(Float32Array.of(0, 0, 1, 1, 2, 2), (c) => c * 4, 0, 12)
    expect(Array.from(low)).toEqual([0, 0.25, 0.5, 0.75, 1, 1.25, 1.5, 1.75, 2, 2, 2, 2])
    expect(Array.from(high)).toEqual([0.25, 0.5, 0.75, 1, 1.25, 1.5, 1.75, 2, 2, 2, 2, 2])
    // Equal samples far apart stay joined, rather than leaving gaps between them.
    const level = analogSpans(Float32Array.of(1, 1, 1, 1, 2, 2), (c) => c * 4, 0, 10)
    expect(Array.from(level.low)).toEqual([1, 1, 1, 1, 1, 1.25, 1.5, 1.75, 2, 2])
    expect(Array.from(level.high)).toEqual([1, 1, 1, 1, 1.25, 1.5, 1.75, 2, 2, 2])
    // Columns a pixel apart whose ranges overlap keep their own; gaps between them are joined.
    const dense = analogSpans(Float32Array.of(0, 3, 1, 2, 5, 6), (c) => c, 0, 3)
    expect(Array.from(dense.low)).toEqual([0, 1, 5])
    expect(Array.from(dense.high)).toEqual([3, 5, 6])
    // Nothing where there are no samples.
    const empty = analogSpans(Float32Array.of(Number.NaN, Number.NaN), (c) => c, 0, 2)
    expect(Array.from(empty.low)).toEqual([Infinity, Infinity])
  })

  it('draw an analog row: dashed divisions, the 0 V line and the curve', () => {
    const frame: FrameDescriptor = {
      width: 16,
      height: 8,
      dpr: 1,
      trackLeft: 0,
      clipTop: 0,
      view: { start: 0, samplesPerPixel: 1, width: 16 },
      grid: [],
      bins: null,
      traces: [],
      analogBins: { start: 0, end: 16, bins: 16, channels: new Map() },
      analog: [
        {
          top: 0,
          height: 8,
          wire: 0,
          columns: new Float32Array(32).fill(1),
          // ±2 V over the row: 1 V a quarter of the way down.
          scale: { perDiv: 1, centre: 0 },
        },
      ],
    }
    expect(raster(frame, 0, 7)).toEqual([
      '................',
      '................',
      'LLLLLLLLLLLLLLLL',
      '................',
      'aaaaaaaaaaaaaaaa',
      '................',
      'ggg...ggg...ggg.',
      '................',
    ])
    // Off the scale, the curve is not drawn at the row's edge.
    frame.analog[0]!.columns = new Float32Array(32).fill(5)
    expect(raster(frame, 0, 7).join('')).not.toContain('L')
  })

  it('mark samples that stand apart with dots on the curve', () => {
    const frame: FrameDescriptor = {
      width: 16,
      height: 8,
      dpr: 1,
      trackLeft: 0,
      clipTop: 0,
      // Eight pixels a sample.
      view: { start: 0, samplesPerPixel: 1 / 8, width: 16 },
      grid: [],
      bins: null,
      traces: [],
      analogBins: { start: 0, end: 2, bins: 2, channels: new Map() },
      analog: [
        {
          top: 0,
          height: 8,
          wire: 0,
          columns: Float32Array.of(1, 1, 1, 1),
          scale: { perDiv: 1, centre: 0 },
        },
      ],
    }
    expect(raster(frame, 1, 3)).toEqual([
      'LL.....LLL......',
      'LLLLLLLLLLLLLLLL',
      'LL.....LLL......',
    ])
    // Closer together, the curve alone.
    frame.view.samplesPerPixel = 1 / 4
    frame.analogBins = { start: 0, end: 4, bins: 4, channels: new Map() }
    frame.analog[0]!.columns = new Float32Array(8).fill(1)
    expect(raster(frame, 1, 3)).toEqual([
      '................',
      'LLLLLLLLLLLLLLLL',
      '................',
    ])
  })

  it('keep every rectangle on whole device pixels', () => {
    const high = CELL_FIRST_HIGH | CELL_LAST_HIGH
    const frame: FrameDescriptor = {
      ...pictureFrame({ cells: Uint8Array.of(0, high, high | CELL_TRANSITION, 0), blocks: [] }),
      dpr: 1.5,
      trackLeft: 10.3,
      view: { start: 0.37, samplesPerPixel: 0.9, width: 8 },
      grid: [3.3],
    }
    frame.traces[0]!.top = 3.7
    frame.traces[0]!.narrow = false
    const all: number[] = []
    layoutFrame(frame, tokens, (x, y, w, h) => all.push(x, y, w, h))
    expect(all.length).toBeGreaterThan(0)
    expect(all.every(Number.isInteger)).toBe(true)
  })
})
