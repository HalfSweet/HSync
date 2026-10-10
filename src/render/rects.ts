/**
 * The waveform layer as axis-aligned rectangles in whole device pixels (DESIGN.md §6): every
 * piece of a trace is horizontal or vertical, so each line segment, wash, block and grid line is
 * one rectangle. Both renderers draw these rectangles, so they agree to the pixel, and lines meet
 * washes and blocks edge to edge rather than at a line's centre.
 */
import { binStart } from '@/ipc/bins'
import { sampleToX } from '@/timeline/viewport'

import { lineWidth, traceShape } from './shape'
import type { FrameDescriptor, RenderTokens } from './types'

/** Floats per rectangle: x, y, width, height in device pixels, then premultiplied RGBA. */
export const RECT_FLOATS = 8

/** Where the high and low levels sit within a row, as fractions of its height (DESIGN.md §6.3). */
const LEVELS = {
  wide: { high: 6.5 / 30, low: 23.5 / 30 },
  narrow: { high: 4.5 / 18, low: 13.5 / 18 },
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
 * blocks and line. A line `w` pixels wide covers the `w` rows (or columns) from its position less
 * `⌊w/2⌋`. Washes and blocks run from the top of the high line to the bottom of the low line, so
 * under a high pulse, where no low line is drawn, the wash ends flush with the low line beside it.
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
  if (!bins) return
  for (const trace of frame.traces) {
    const levels = trace.narrow ? LEVELS.narrow : LEVELS.wide
    const high = device(trace.top + trace.height * levels.high) - half
    const low = device(trace.top + trace.height * levels.low) - half
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
