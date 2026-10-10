/**
 * Waveform renderers draw the canvas layer of the timeline (DESIGN.md §6): grid lines and
 * traces. Canvas2D is the baseline; a WebGPU renderer can implement the same interface.
 */
import type { BinsResult } from '@/ipc/bins'
import type { Viewport } from '@/timeline/viewport'

/** Colours resolved from the design tokens of the current theme. */
export interface RenderTokens {
  grid: string
  /** Line colour of each of the eight wire colours. */
  wires: string[]
  /** Fill of the high level of each wire colour (the line colour at its wash opacity). */
  washes: string[]
}

export interface TraceDraw {
  /** Top of the row in canvas CSS pixels. */
  top: number
  height: number
  wire: number
  /** Narrow rows use a thinner line and smaller margins. */
  narrow: boolean
  cells: Uint8Array
}

export interface FrameDescriptor {
  /** Canvas size in CSS pixels. */
  width: number
  height: number
  dpr: number
  /** Canvas x of the track's left edge (the label column width minus horizontal scroll). */
  trackLeft: number
  /** Rows are drawn below this canvas y (the bottom of the sticky ruler). */
  clipTop: number
  view: Viewport
  /** Track x of the major ticks. */
  grid: number[]
  /** The columns every trace's cells refer to. */
  bins: BinsResult | null
  traces: TraceDraw[]
}

export interface WaveformRenderer {
  draw(frame: FrameDescriptor, tokens: RenderTokens): void
  dispose(): void
}
