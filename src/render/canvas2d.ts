/**
 * Canvas2D waveform renderer: the frame's rectangles ({@link layoutFrame}) filled in whole device
 * pixels, the same ones the WebGPU renderer draws.
 */
import { layoutFrame } from './rects'
import type { FrameDescriptor, RenderTokens, WaveformRenderer } from './types'

export class Canvas2DRenderer implements WaveformRenderer {
  private readonly context: CanvasRenderingContext2D

  constructor(private readonly canvas: HTMLCanvasElement) {
    const context = canvas.getContext('2d', { alpha: true })
    if (!context) throw new Error('canvas 2d context unavailable')
    this.context = context
  }

  draw(frame: FrameDescriptor, tokens: RenderTokens): void {
    const { canvas, context } = this
    const pixelWidth = Math.max(1, Math.round(frame.width * frame.dpr))
    const pixelHeight = Math.max(1, Math.round(frame.height * frame.dpr))
    if (canvas.width !== pixelWidth || canvas.height !== pixelHeight) {
      canvas.width = pixelWidth
      canvas.height = pixelHeight
    }
    context.setTransform(1, 0, 0, 1, 0, 0)
    context.clearRect(0, 0, pixelWidth, pixelHeight)

    // Rows end at the sticky ruler.
    const clipTop = Math.min(pixelHeight, Math.max(0, Math.round(frame.clipTop * frame.dpr)))
    context.save()
    context.beginPath()
    context.rect(0, clipTop, pixelWidth, pixelHeight - clipTop)
    context.clip()

    // Consecutive rectangles of one colour fill as one path.
    let color: string | null = null
    let path = new Path2D()
    const fill = () => {
      if (color === null) return
      context.fillStyle = color
      context.fill(path)
    }
    layoutFrame(frame, tokens, (x, y, width, height, css) => {
      if (css !== color) {
        fill()
        color = css
        path = new Path2D()
      }
      path.rect(x, y, width, height)
    })
    fill()
    context.restore()
  }

  dispose(): void {
    this.canvas.width = 0
    this.canvas.height = 0
  }
}
