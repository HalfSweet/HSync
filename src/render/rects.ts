/**
 * The waveform layer as axis-aligned rectangles in whole device pixels: every
 * piece of a trace is horizontal or vertical, so each line segment, wash, block and grid line is
 * one rectangle. Both renderers draw these rectangles, so they agree to the pixel, and lines meet
 * washes and blocks edge to edge rather than at a line's centre.
 */
import { DIVISIONS, yOfVolts } from '@/analog/scale'
import { binStart } from '@/ipc/bins'
import { sampleToX } from '@/timeline/viewport'

import { lineWidth, traceShape } from './shape'
import type { AnalogDraw, FrameDescriptor, RenderTokens, TraceDraw } from './types'

/** Floats per rectangle: x, y, width, height in device pixels, then premultiplied RGBA. */
export const RECT_FLOATS = 8

/**
 * The room above the high level and below the low one (DESIGN.md §6.2): a share of the row's
 * height, up to what a row of the design's height leaves, so that a row made taller gives the
 * extra to the swing between the levels.
 */
const MARGINS = {
  wide: { share: 6.5 / 30, most: 6.5 },
  narrow: { share: 4.5 / 18, most: 4.5 },
}

/** Where the high and low levels of a trace sit, in CSS pixels. */
function levelsOf(trace: TraceDraw): { high: number; low: number } {
  const margin = trace.narrow ? MARGINS.narrow : MARGINS.wide
  const room = Math.min(trace.height * margin.share, margin.most)
  return { high: trace.top + room, low: trace.top + trace.height - room }
}

export type Rgba = [number, number, number, number]

/** A CSS colour of the design tokens: `#rrggbb`, or `rgb(r g b / a)` and its comma forms. */
export function parseColor(css: string): Rgba {
  const text = css.trim()
  const hex = /^#([0-9a-f]{6})$/i.exec(text)
  if (hex) {
    const value = Number.parseInt(hex[1] ?? '0', 16)
    return [((value >> 16) & 0xff) / 255, ((value >> 8) & 0xff) / 255, (value & 0xff) / 255, 1]
  }
  const rgb = /^rgba?\(([^)]*)\)$/i.exec(text)
  if (rgb) {
    const parts = (rgb[1] ?? '').split(/[\s,/]+/).filter(Boolean)
    const channel = (part: string | undefined) => Number.parseFloat(part ?? '0') / 255
    const alphaText = parts[3]
    const alpha =
      alphaText === undefined
        ? 1
        : alphaText.endsWith('%')
          ? Number.parseFloat(alphaText) / 100
          : Number.parseFloat(alphaText)
    return [
      channel(parts[0]),
      channel(parts[1]),
      channel(parts[2]),
      Number.isFinite(alpha) ? alpha : 1,
    ]
  }
  return [0, 0, 0, 0]
}

/** Receives a rectangle in device pixels with its CSS colour; rectangles come in drawing order. */
export type RectSink = (x: number, y: number, width: number, height: number, color: string) => void

/**
 * Every rectangle of a frame in whole device pixels: grid lines first, then each trace's washes,
 * blocks and line, then each analog row's graticule and curve. A line `w` pixels wide covers the
 * `w` rows (or columns) from its position less `⌊w/2⌋`. Washes and blocks run from the top of the
 * high line to the bottom of the low line, so under a high pulse, where no low line is drawn, the
 * wash ends flush with the low line beside it.
 */
export function layoutFrame(frame: FrameDescriptor, tokens: RenderTokens, sink: RectSink): void {
  const rect: RectSink = (x, y, width, height, color) => {
    if (width > 0 && height > 0) sink(x, y, width, height, color)
  }
  const dpr = frame.dpr
  const device = (css: number) => Math.round(css * dpr)
  const width = lineWidth(dpr)
  const half = Math.floor(width / 2)

  const top = device(frame.clipTop)
  const bottom = device(frame.height)
  for (const x of frame.grid) {
    rect(device(frame.trackLeft + x) - half, top, width, bottom - top, tokens.grid)
  }
  const bins = frame.bins
  for (const trace of frame.traces) {
    if (!bins) break
    const levels = levelsOf(trace)
    const high = device(levels.high) - half
    const low = device(levels.low) - half
    const shape = traceShape(
      trace.cells,
      (column) => device(frame.trackLeft + sampleToX(frame.view, binStart(bins, column))),
      trace.blocks,
      trace.wash,
    )
    const wash = tokens.washes[trace.wire] ?? tokens.grid
    for (const [from, to] of shape.washes) rect(from, high, to - from, low + width - high, wash)
    const block = tokens.blocks[trace.wire] ?? tokens.grid
    for (const [from, to] of shape.blocks) rect(from, high, to - from, low + width - high, block)

    const color = tokens.wires[trace.wire] ?? tokens.grid
    let x = 0
    let y = 0
    for (const [command, at, isHigh] of shape.path) {
      const row = isHigh ? high : low
      if (command === 'L') {
        // A level runs on over the edges at its ends, so the corners are square.
        if (row === y) rect(Math.min(x, at) - half, row, Math.abs(at - x) + width, width, color)
        else rect(at - half, Math.min(y, row), width, Math.abs(row - y) + width, color)
      }
      x = at
      y = row
    }
  }
  for (const draw of frame.analog) layoutAnalog(frame, draw, tokens, rect)
}

/** The graticule's dashes and the gaps between them, in CSS pixels. */
const DASH = 3
const DASH_GAP = 3
/** Samples at least this many CSS pixels apart are marked with a dot each. */
const DOT_SPACING = 6

/**
 * The lowest and highest volts each device pixel of the track `left..right` covers: each
 * column's own range, joined to the next column's by a straight line, so that the curve stays
 * connected however far apart its samples are drawn. The last column before the samples end
 * holds its value to its end, as a logic trace's last sample does. Pixels with nothing are
 * left at `+∞` and `−∞`.
 *
 * @param columns lowest and highest volts of each column in turn, NaN where there are none
 * @param x device x of a column's first sample; `x(n)` is where the last column ends
 */
export function analogSpans(
  columns: Float32Array,
  x: (column: number) => number,
  left: number,
  right: number,
): { low: Float64Array; high: Float64Array } {
  const size = Math.max(0, right - left)
  const low = new Float64Array(size).fill(Number.POSITIVE_INFINITY)
  const high = new Float64Array(size).fill(Number.NEGATIVE_INFINITY)
  const add = (pixel: number, a: number, b: number) => {
    const at = pixel - left
    if (at < 0 || at >= size) return
    low[at] = Math.min(low[at] ?? Infinity, a, b)
    high[at] = Math.max(high[at] ?? -Infinity, a, b)
  }
  const count = columns.length / 2
  for (let column = 0; column < count; column++) {
    const lo = columns[2 * column] ?? Number.NaN
    const hi = columns[2 * column + 1] ?? Number.NaN
    if (Number.isNaN(lo) || Number.isNaN(hi)) continue
    const x0 = x(column)
    const x1 = x(column + 1)
    const first = Math.floor(x0)
    add(first, lo, hi)
    const nextLow = columns[2 * column + 2] ?? Number.NaN
    const nextHigh = columns[2 * column + 3] ?? Number.NaN
    const from = Math.max(first, left)
    if (Number.isNaN(nextLow) || Number.isNaN(nextHigh)) {
      for (let pixel = from; pixel < Math.min(Math.ceil(x1), right); pixel++) add(pixel, lo, hi)
      continue
    }
    // A line runs from where this column leaves off to where the next one begins: from its
    // nearest end to the next range's middle, to the next range's nearest point. Where the
    // ranges overlap, the line stays level within both, still covering the pixels between.
    const exit = Math.min(Math.max((nextLow + nextHigh) / 2, lo), hi)
    const entry = Math.min(Math.max(exit, nextLow), nextHigh)
    const span = Math.max(x1 - x0, 1e-9)
    const along = (pixel: number) =>
      exit + (entry - exit) * Math.min(1, Math.max(0, (pixel - x0) / span))
    for (let pixel = from; pixel <= Math.min(Math.floor(x1), right - 1); pixel++) {
      add(pixel, along(pixel), along(pixel + 1))
    }
  }
  return { low, high }
}

/**
 * An analog row's rectangles: the graticule's dashed lines between its divisions, the 0 V line
 * where it falls inside the row, then the curve, clipped to the row.
 */
function layoutAnalog(
  frame: FrameDescriptor,
  draw: AnalogDraw,
  tokens: RenderTokens,
  rect: RectSink,
) {
  const dpr = frame.dpr
  const device = (css: number) => Math.round(css * dpr)
  const width = lineWidth(dpr)
  const half = Math.floor(width / 2)
  const origin = device(frame.trackLeft)
  const left = Math.max(origin, 0)
  const right = device(frame.width)
  const top = device(draw.top)
  const bottom = device(draw.top + draw.height)

  const dash = Math.max(1, device(DASH))
  const period = dash + Math.max(1, device(DASH_GAP))
  for (let division = 1; division < DIVISIONS; division++) {
    const y = device(draw.top + (draw.height * division) / DIVISIONS) - half
    for (let x = origin; x < right; x += period) {
      const from = Math.max(x, left)
      rect(from, y, x + dash - from, width, tokens.grid)
    }
  }
  const zero = yOfVolts(draw.scale, 0, draw.height)
  if (zero > 0 && zero < draw.height) {
    rect(left, device(draw.top + zero) - half, right - left, width, tokens.axis)
  }

  const bins = frame.analogBins
  if (!bins || !draw.columns) return
  const { low, high } = analogSpans(
    draw.columns,
    (column) => dpr * (frame.trackLeft + sampleToX(frame.view, binStart(bins, column))),
    left,
    right,
  )
  const color = tokens.wires[draw.wire] ?? tokens.grid
  const yOf = (volts: number) => device(draw.top + yOfVolts(draw.scale, volts, draw.height)) - half
  // Neighbouring pixels that cover the same rows merge into one rectangle.
  let run: { from: number; y: number; end: number } | null = null
  const flush = (to: number) => {
    if (run) rect(run.from - half, run.y, to - run.from + width, run.end - run.y, color)
    run = null
  }
  for (let at = 0; at < low.length; at++) {
    const pixel = left + at
    const lo = low[at] ?? Infinity
    const hi = high[at] ?? -Infinity
    const y = lo <= hi ? Math.max(top, yOf(hi)) : bottom
    const end = lo <= hi ? Math.min(bottom, yOf(lo) + width) : top
    if (end <= y) {
      flush(pixel - 1)
      continue
    }
    if (run && run.y === y && run.end === end) continue
    flush(pixel - 1)
    run = { from: pixel, y, end }
  }
  flush(left + low.length - 1)

  // Zoomed in until each column is one sample and samples stand apart, each gets a dot.
  if (bins.end - bins.start > bins.bins || frame.view.samplesPerPixel * DOT_SPACING > 1) return
  const size = 3 * width
  const insets = dotInsets(size)
  for (let column = 0; column < bins.bins; column++) {
    const volts = draw.columns[2 * column] ?? Number.NaN
    if (Number.isNaN(volts)) continue
    const at = dpr * (frame.trackLeft + sampleToX(frame.view, binStart(bins, column)))
    // Centred on the curve, which covers `width` pixels from its position less `half`.
    const x = Math.floor(at) - half - width
    const y = yOf(volts) - width
    insets.forEach((inset, row) => {
      if (y + row < top || y + row >= bottom) return
      const from = Math.max(x + inset, left)
      rect(from, y + row, Math.min(x + size - inset, right) - from, 1, color)
    })
  }
}

/** How far each row of a round dot `size` pixels across starts in from its square. */
function dotInsets(size: number): number[] {
  const radius = size / 2
  return Array.from({ length: size }, (_, row) => {
    const offset = row + 0.5 - radius
    const chord = 2 * Math.sqrt(Math.max(0, radius * radius - offset * offset))
    return Math.round((size - chord) / 2)
  })
}

/** Grows a float buffer as rectangles are added. */
class RectList {
  data = new Float32Array(1024 * RECT_FLOATS)
  count = 0

  push(x: number, y: number, width: number, height: number, color: Rgba): void {
    if (width <= 0 || height <= 0 || color[3] <= 0) return
    if ((this.count + 1) * RECT_FLOATS > this.data.length) {
      const grown = new Float32Array(this.data.length * 2)
      grown.set(this.data)
      this.data = grown
    }
    const at = this.count * RECT_FLOATS
    const [r, g, b, a] = color
    this.data.set([x, y, width, height, r * a, g * a, b * a, a], at)
    this.count += 1
  }
}

/** {@link layoutFrame} as instance data for the WebGPU renderer. */
export function frameRects(
  frame: FrameDescriptor,
  tokens: RenderTokens,
): { data: Float32Array; count: number } {
  const list = new RectList()
  const colors = new Map<string, Rgba>()
  layoutFrame(frame, tokens, (x, y, width, height, css) => {
    let color = colors.get(css)
    if (!color) {
      color = parseColor(css)
      colors.set(css, color)
    }
    list.push(x, y, width, height, color)
  })
  return list
}
