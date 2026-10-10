/**
 * The waveform layer as axis-aligned rectangles, for the WebGPU renderer (DESIGN.md §6): every
 * piece of a trace is horizontal or vertical, so each line segment, wash and grid line is one
 * rectangle drawn as an instance.
 */
import { binStart } from '@/ipc/bins'
import { sampleToX } from '@/timeline/viewport'

import { traceShape } from './shape'
import type { FrameDescriptor, RenderTokens } from './types'

/** Floats per rectangle: x, y, width, height in device pixels, then premultiplied RGBA. */
export const RECT_FLOATS = 8

/** Where the levels sit and how thick the line is, as the Canvas2D renderer draws them. */
const LEVELS = {
  wide: { high: 6.5 / 30, low: 23.5 / 30, line: 1.25 },
  narrow: { high: 4.5 / 18, low: 13.5 / 18, line: 1 },
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

/** Every rectangle of a frame in device pixels, grid first, then each trace's wash and line. */
export function frameRects(
  frame: FrameDescriptor,
  tokens: RenderTokens,
): { data: Float32Array; count: number } {
  const list = new RectList()
  const dpr = frame.dpr
  const grid = parseColor(tokens.grid)
  for (const x of frame.grid) {
    const px = Math.round((frame.trackLeft + x) * dpr)
    list.push(px, frame.clipTop * dpr, 1, (frame.height - frame.clipTop) * dpr, grid)
  }
  const bins = frame.bins
  if (!bins) return list
  for (const trace of frame.traces) {
    const geometry = trace.narrow ? LEVELS.narrow : LEVELS.wide
    const yHigh = trace.top + trace.height * geometry.high
    const yLow = trace.top + trace.height * geometry.low
    const shape = traceShape(
      trace.cells,
      (column) => frame.trackLeft + sampleToX(frame.view, binStart(bins, column)),
      trace.blocks,
      trace.clock,
    )
    const wash = parseColor(tokens.washes[trace.wire] ?? tokens.grid)
    for (const [from, to] of shape.washes) {
      list.push(from * dpr, yHigh * dpr, Math.max(to - from, 0.5) * dpr, (yLow - yHigh) * dpr, wash)
    }
    const block = parseColor(tokens.blocks[trace.wire] ?? tokens.grid)
    for (const [from, to] of shape.blocks) {
      list.push(
        from * dpr,
        yHigh * dpr,
        Math.max(to - from, 0.5) * dpr,
        (yLow - yHigh) * dpr,
        block,
      )
    }
    const color = parseColor(tokens.wires[trace.wire] ?? tokens.grid)
    const half = (geometry.line * dpr) / 2
    let x = 0
    let y = 0
    for (const [command, at, high] of shape.path) {
      const nextX = at * dpr
      const nextY = (high ? yHigh : yLow) * dpr
      if (command === 'L') {
        if (nextY === y) {
          // Horizontal: overlap half a line at each end so corners meet like mitres.
          const left = Math.min(x, nextX)
          list.push(left - half, y - half, Math.abs(nextX - x) + 2 * half, 2 * half, color)
        } else {
          const top = Math.min(y, nextY)
          list.push(nextX - half, top - half, 2 * half, Math.abs(nextY - y) + 2 * half, color)
        }
      }
      x = nextX
      y = nextY
    }
  }
  return list
}
