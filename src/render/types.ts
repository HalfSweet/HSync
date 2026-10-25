/**
 * Waveform renderers draw the canvas layer of the timeline: grid lines and
 * traces. Canvas2D is the baseline; a WebGPU renderer can implement the same interface.
 */
import type { VerticalScale } from '@/analog/scale'
import type { AnalogBinsResult } from '@/ipc/analogBins'
import type { BinsResult } from '@/ipc/bins'
import type { Viewport } from '@/timeline/viewport'

import type { ColumnRun } from './shape'

/** Colours resolved from the design tokens of the current theme. */
export interface RenderTokens {
  grid: string
  /** The 0 V line of analog rows. */
  axis: string
  /** Line colour of each of the eight wire colours. */
  wires: string[]
  /** Fill of the high level of each wire colour (the line colour at its wash opacity). */
  washes: string[]
  /** Fill of blocks, stretches too dense to draw edge by edge: the wash, `--bk` times as opaque. */
  blocks: string[]
}

export interface TraceDraw {
  /** Top of the row in canvas CSS pixels. */
  top: number
  height: number
  wire: number
  /** Narrow rows leave smaller margins above and below the levels. */
  narrow: boolean
  cells: Uint8Array
  /** Column runs drawn as blocks, ascending and apart. */
  blocks: readonly ColumnRun[]
  /** Whether high stretches get the wash: not on a line busy in view. */
  wash: boolean
}

/** An analog channel's row: its graticule, 0 V line and curve (DESIGN.md §6.2). */
export interface AnalogDraw {
  /** Top of the row in canvas CSS pixels. */
  top: number
  height: number
  wire: number
  /** Lowest and highest volts of each column of {@link FrameDescriptor.analogBins}; `null`
   * before they arrive. */
  columns: Float32Array | null
  scale: VerticalScale
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
  /** The columns every analog row's columns refer to. */
  analogBins: AnalogBinsResult | null
  analog: AnalogDraw[]
}

export interface WaveformRenderer {
  draw(frame: FrameDescriptor, tokens: RenderTokens): void
  dispose(): void
}
