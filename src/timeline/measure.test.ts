import { describe, expect, it } from 'vitest'

import type { PulseInfo } from '@/ipc/bindings/PulseInfo'

import {
  clickMeasure,
  leftStart,
  measureGeometry,
  nearestEdge,
  type Measure,
  type MeasureEnd,
  type RowBox,
} from './measure'
import type { Viewport } from './viewport'

/** Ten samples a pixel, 100 px wide: sample 100 is at x 10. */
const view: Viewport = { start: 0, samplesPerPixel: 10, width: 100 }

const pulse = (start: number, end: number, startIsEdge = true, endIsEdge = true): PulseInfo => ({
  high: true,
  start,
  end,
  startIsEdge,
  endIsEdge,
  nextEnd: null,
  afterEnd: null,
})

const edge = (sample: number, rowKey = 'd0'): MeasureEnd => ({ sample, rowKey, edge: true })

describe('the edge under the pointer', () => {
  it('is the nearer end of the pulse within ten pixels', () => {
    expect(nearestEdge(pulse(100, 200), 12, view)).toBe(100)
    expect(nearestEdge(pulse(100, 200), 19, view)).toBe(200)
    // Halfway, the earlier edge.
    expect(nearestEdge(pulse(100, 200), 15, view)).toBe(100)
    expect(nearestEdge(pulse(100, 400), 25, view)).toBeNull()
  })

  it('is never the start or end of the capture, nor the first edge', () => {
    expect(nearestEdge(pulse(0, 300, false), 1, view)).toBeNull()
    expect(nearestEdge(pulse(100, 200), 12, view, 100)).toBe(200)
    expect(nearestEdge(pulse(100, 400), 12, view, 100)).toBeNull()
  })
})

describe('clicks on the tracks', () => {
  it('start on an edge and finish on another', () => {
    const measuring = clickMeasure(null, edge(100))
    expect(measuring).toEqual({ phase: 'measuring', from: edge(100) })
    expect(clickMeasure(measuring, edge(250, 'd3'))).toEqual({
      phase: 'done',
      from: edge(100),
      to: edge(250, 'd3'),
    })
  })

  it('let go off an edge, or on the first edge again', () => {
    const measuring: Measure = { phase: 'measuring', from: edge(100) }
    expect(clickMeasure(measuring, null)).toBeNull()
    expect(clickMeasure(measuring, edge(100))).toBeNull()
    expect(clickMeasure(null, null)).toBeNull()
  })

  it('start over from a finished measurement', () => {
    const done: Measure = { phase: 'done', from: edge(100), to: edge(200) }
    expect(clickMeasure(done, edge(300))).toEqual({ phase: 'measuring', from: edge(300) })
    expect(clickMeasure(done, null)).toBeNull()
  })
})

describe('the end following the pointer', () => {
  it('leaves the first edge for another edge, another row or a snap radius away', () => {
    const from = edge(100)
    expect(leftStart(from, edge(130), view)).toBe(true)
    expect(leftStart(from, { sample: 100, rowKey: 'd3', edge: false }, view)).toBe(true)
    expect(leftStart(from, { sample: 150, rowKey: 'd0', edge: false }, view)).toBe(false)
    expect(leftStart(from, { sample: 200, rowKey: 'd0', edge: false }, view)).toBe(true)
  })
})

describe('measurement geometry', () => {
  const rows: Record<string, RowBox> = { d0: { top: 40, height: 30 }, d3: { top: 100, height: 18 } }
  const rowOf = (key: string) => rows[key] ?? null

  it('runs a line across the row between edges on it, the read-out above', () => {
    const geometry = measureGeometry(edge(100), edge(300), view, rowOf)
    expect(geometry.from).toMatchObject({ x: 10, top: 40, visible: true, edge: true })
    expect(geometry.span).toEqual({ left: 10, width: 20, y: 55 })
    expect(geometry.drop).toBeNull()
    expect(geometry.label).toEqual({ center: 20, bottom: 42 })
  })

  it('drops from the first edge to the row of the second', () => {
    const geometry = measureGeometry(edge(300), edge(100, 'd3'), view, rowOf)
    expect(geometry.span).toEqual({ left: 10, width: 20, y: 109 })
    expect(geometry.drop).toEqual({ x: 30, top: 55, height: 54 })
    expect(geometry.label?.bottom).toBe(102)
  })

  it('clips to the view and hides the read-out of a measurement out of it', () => {
    const across = measureGeometry(edge(-500), edge(5000), view, rowOf)
    expect(across.span).toEqual({ left: 0, width: 100, y: 55 })
    expect(across.label?.center).toBe(50)
    expect(across.from?.visible).toBe(false)
    const before = measureGeometry(edge(-500), edge(-100), view, rowOf)
    expect(before.span).toBeNull()
    expect(before.label).toBeNull()
  })

  it('leaves out the end on a hidden row', () => {
    const geometry = measureGeometry(edge(100, 'gone'), edge(300), view, rowOf)
    expect(geometry.from).toBeNull()
    expect(geometry.drop).toBeNull()
    expect(geometry.span).toEqual({ left: 10, width: 20, y: 55 })
  })
})
