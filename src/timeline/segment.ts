/**
 * Placement of a segment on the ruler and over the rows (DESIGN.md §9.6): the A and B flags sit
 * outside the segment except near the edges of the view, where they turn inward; a wide segment
 * carries its length between the flags, a narrow one beside B (or beside A near the right edge).
 */
import { sampleToX, viewportEnd, type Viewport } from './viewport'

export interface SegmentEnd {
  /** Cursor position in track pixels. */
  x: number
  /** Whether the end lies within the view; ends outside draw neither cursor nor flag. */
  visible: boolean
  /** Left of the 14 px flag in track pixels. */
  flag: number
}

export type SegmentLabel =
  | { kind: 'between'; left: number; width: number; arrowA: boolean; arrowB: boolean }
  | { kind: 'besideB'; left: number }
  | { kind: 'besideA'; right: number }

export interface SegmentGeometry {
  /** The visible part of the segment, for the band over the rows. */
  band: { left: number; width: number } | null
  a: SegmentEnd
  b: SegmentEnd
  label: SegmentLabel | null
  /** An end lies outside the view, so the length is not shown. */
  open: boolean
}

const FLAG = 14
const FLAG_GAP = 2

export function segmentGeometry(from: number, to: number, view: Viewport): SegmentGeometry {
  const width = view.width
  const start = view.start
  const end = viewportEnd(view)
  const ax = sampleToX(view, from)
  const bx = sampleToX(view, to)
  const left = Math.max(ax, 0)
  const right = Math.min(bx, width)
  const visibleWidth = Math.max(0, right - left)
  const aVisible = from >= start && from <= end
  const bVisible = to >= start && to <= end
  const a: SegmentEnd = {
    x: ax,
    visible: aVisible,
    flag: ax < width * 0.015 ? ax + FLAG_GAP : ax - FLAG - FLAG_GAP,
  }
  const b: SegmentEnd = {
    x: bx,
    visible: bVisible,
    flag: bx > width * 0.986 ? bx - FLAG - FLAG_GAP : bx + FLAG_GAP,
  }

  let label: SegmentLabel | null = null
  if (to > from && visibleWidth > 0) {
    if (visibleWidth >= width * 0.09) {
      label = { kind: 'between', left, width: visibleWidth, arrowA: aVisible, arrowB: bVisible }
    } else if (bx <= width * 0.88) {
      label = { kind: 'besideB', left: bx + 20 }
    } else {
      label = { kind: 'besideA', right: width - ax + 20 }
    }
  }
  return {
    band: visibleWidth > 0 ? { left, width: visibleWidth } : null,
    a,
    b,
    label,
    open: !aVisible || !bVisible,
  }
}
