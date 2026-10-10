/**
 * Canvas2D waveform renderer: grid lines, then for each trace its high-level wash and its line
 * (see {@link traceShape}).
 */
import { binStart } from '@/ipc/bins'
import { sampleToX } from '@/timeline/viewport'

import { traceShape } from './shape'
import type { FrameDescriptor, RenderTokens, TraceDraw, WaveformRenderer } from './types'

/** Where the high and low levels sit within a row, as fractions of its height (DESIGN.md §6.3). */
const LEVELS = {
  wide: { high: 6.5 / 30, low: 23.5 / 30, line: 1.25 },
  narrow: { high: 4.5 / 18, low: 13.5 / 18, line: 1 },
}

export class Canvas2DRenderer implements WaveformRenderer {
  private readonly context: CanvasRenderingContext2D

  constructor(private readonly canvas: HTMLCanvasElement) {
    const context = canvas.getContext('2d', { alpha: true })
    if (!context) throw new Error('canvas 2d context unavailable')
    this.context = context
  }

  draw(frame: FrameDescriptor, tokens: RenderTokens): void {
    const { canvas, context } = this
    const pixelWidth = Math.round(frame.width * frame.dpr)
    const pixelHeight = Math.round(frame.height * frame.dpr)
    if (canvas.width !== pixelWidth || canvas.height !== pixelHeight) {
      canvas.width = pixelWidth
      canvas.height = pixelHeight
    }
    context.setTransform(1, 0, 0, 1, 0, 0)
    context.clearRect(0, 0, pixelWidth, pixelHeight)
    context.setTransform(frame.dpr, 0, 0, frame.dpr, 0, 0)

    context.save()
    context.beginPath()
    context.rect(0, frame.clipTop, frame.width, frame.height - frame.clipTop)
    context.clip()

    this.drawGrid(frame, tokens)
    if (frame.bins) {
      for (const trace of frame.traces) this.drawTrace(frame, trace, tokens)
    }
    context.restore()
  }

  private drawGrid(frame: FrameDescriptor, tokens: RenderTokens): void {
    const { context } = this
    context.beginPath()
    for (const x of frame.grid) {
      const px = Math.round((frame.trackLeft + x) * frame.dpr) / frame.dpr + 0.5 / frame.dpr
      context.moveTo(px, frame.clipTop)
      context.lineTo(px, frame.height)
    }
    context.strokeStyle = tokens.grid
    context.lineWidth = 1 / frame.dpr
    context.stroke()
  }

  private drawTrace(frame: FrameDescriptor, trace: TraceDraw, tokens: RenderTokens): void {
    const bins = frame.bins
    if (!bins) return
    const { context } = this
    const geometry = trace.narrow ? LEVELS.narrow : LEVELS.wide
    const yHigh = trace.top + trace.height * geometry.high
    const yLow = trace.top + trace.height * geometry.low
    const shape = traceShape(
      trace.cells,
      (column) => frame.trackLeft + sampleToX(frame.view, binStart(bins, column)),
    )

    const wash = new Path2D()
    for (const [from, to] of shape.washes)
      wash.rect(from, yHigh, Math.max(to - from, 0.5), yLow - yHigh)
    context.fillStyle = tokens.washes[trace.wire] ?? tokens.grid
    context.fill(wash)

    const line = new Path2D()
    for (const [command, at, high] of shape.path) {
      if (command === 'M') line.moveTo(at, high ? yHigh : yLow)
      else line.lineTo(at, high ? yHigh : yLow)
    }
    context.strokeStyle = tokens.wires[trace.wire] ?? tokens.grid
    context.lineWidth = geometry.line
    context.lineJoin = 'miter'
    context.lineCap = 'butt'
    context.stroke(line)
  }

  dispose(): void {
    this.canvas.width = 0
    this.canvas.height = 0
  }
}
