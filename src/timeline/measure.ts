/**
 * Edge measurement (DESIGN.md §6.5), after DSView's edge jump: click an edge, point at another
 * on the same channel or any other, and read the time between them. The end under the pointer
 * snaps to the nearest edge, and clicking there keeps the measurement on screen.
 */
import type { PulseInfo } from '@/ipc/bindings/PulseInfo'

import { sampleToX, type Viewport } from './viewport'

/** How near the pointer an edge is picked, in CSS pixels, as in DSView. */
export const SNAP_RADIUS = 10

/** One end of a measurement. */
export interface MeasureEnd {
  /** The first sample at an edge's new level, or the sample under the pointer. */
  sample: number
  /** The row the end is on. */
  rowKey: string
  /** Whether the end is an edge of the row's channel rather than wherever the pointer is. */
  edge: boolean
}

export type Measure =
  /** The first edge is chosen; the other end follows the pointer. */
  | { phase: 'measuring'; from: MeasureEnd }
  /** Both edges are chosen. */
  | { phase: 'done'; from: MeasureEnd; to: MeasureEnd }

/**
 * The edge of `pulse`, the run of equal samples under the pointer, nearest to track x `x` and at
 * most `radius` pixels from it. `except` is an edge not to pick: the measurement's first.
 */
export function nearestEdge(
  pulse: PulseInfo,
  x: number,
  view: Viewport,
  except: number | null = null,
  radius = SNAP_RADIUS,
): number | null {
  const candidates: [number, boolean][] = [
    [pulse.start, pulse.startIsEdge],
    [pulse.end, pulse.endIsEdge],
  ]
  let found: number | null = null
  let distance = Infinity
  for (const [sample, isEdge] of candidates) {
    if (!isEdge || sample === except) continue
    const away = Math.abs(sampleToX(view, sample) - x)
    if (away <= radius && away < distance) {
      found = sample
      distance = away
    }
  }
  return found
}

const sameEnd = (a: MeasureEnd, b: MeasureEnd) => a.rowKey === b.rowKey && a.sample === b.sample

/**
 * What a click on the tracks does, given the edge it lands on: on an edge it starts a
 * measurement, or completes the one under way; anywhere else, or on the first edge again, it
 * lets go.
 */
export function clickMeasure(measure: Measure | null, edge: MeasureEnd | null): Measure | null {
  if (measure?.phase === 'measuring') {
    return edge && !sameEnd(edge, measure.from)
      ? { phase: 'done', from: measure.from, to: edge }
      : null
  }
  return edge ? { phase: 'measuring', from: edge } : null
}

/**
 * Whether the end following the pointer has left the first edge: it sits on another edge, on
 * another row, or a snap radius away.
 */
export function leftStart(from: MeasureEnd, to: MeasureEnd, view: Viewport): boolean {
  if (to.edge || to.rowKey !== from.rowKey) return !sameEnd(from, to)
  return Math.abs(sampleToX(view, to.sample) - sampleToX(view, from.sample)) >= SNAP_RADIUS
}

/** A row's place among the timeline's rows, in CSS pixels. */
export interface RowBox {
  top: number
  height: number
}

export interface MeasurePoint {
  /** Track x of the end. */
  x: number
  /** Its row. */
  top: number
  height: number
  /** Whether the end lies within the view. */
  visible: boolean
  edge: boolean
}

export interface MeasureGeometry {
  /** The ends, unless their rows are hidden. */
  from: MeasurePoint | null
  to: MeasurePoint | null
  /** The line between the ends across the middle of the second end's row, clipped to the view. */
  span: { left: number; width: number; y: number } | null
  /** The drop from the first end to the line when the ends are on different rows. */
  drop: { x: number; top: number; height: number } | null
}

/** Where the parts of a measurement from `from` to `to` go; `rowOf` places a row. */
export function measureGeometry(
  from: MeasureEnd,
  to: MeasureEnd,
  view: Viewport,
  rowOf: (key: string) => RowBox | null,
): MeasureGeometry {
  const point = (end: MeasureEnd): MeasurePoint | null => {
    const row = rowOf(end.rowKey)
    if (!row) return null
    const x = sampleToX(view, end.sample)
    return { ...row, x, visible: x >= 0 && x <= view.width, edge: end.edge }
  }
  const a = point(from)
  const b = point(to)
  const line = b ?? a
  if (!line) return { from: a, to: b, span: null, drop: null }

  const y = line.top + line.height / 2
  const ax = sampleToX(view, from.sample)
  const bx = sampleToX(view, to.sample)
  const left = Math.max(0, Math.min(ax, bx))
  const right = Math.min(view.width, Math.max(ax, bx))
  const span = right > left ? { left, width: right - left, y } : null
  let drop: MeasureGeometry['drop'] = null
  if (a && b && a.top !== b.top && a.visible) {
    const from = a.top + a.height / 2
    drop = { x: ax, top: Math.min(from, y), height: Math.abs(from - y) }
  }
  return { from: a, to: b, span, drop }
}
