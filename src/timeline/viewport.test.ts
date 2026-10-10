import { describe, expect, it } from 'vitest'

import {
  clampViewport,
  fitViewport,
  panViewport,
  rangeViewport,
  sampleToX,
  viewportEnd,
  xToSample,
  zoomViewport,
} from './viewport'

const COUNT = 131_072

describe('viewport', () => {
  it('fits the whole capture', () => {
    const view = fitViewport(1024, COUNT)
    expect(view.samplesPerPixel).toBe(128)
    expect(viewportEnd(view)).toBe(COUNT)
  })

  it('converts between samples and pixels', () => {
    const view = { start: 1000, samplesPerPixel: 2, width: 500 }
    expect(sampleToX(view, 1100)).toBe(50)
    expect(xToSample(view, 50)).toBe(1100)
  })

  it('zooms around the anchor', () => {
    const view = { start: 1000, samplesPerPixel: 4, width: 500 }
    const zoomed = zoomViewport(view, 2, 100, COUNT)
    expect(zoomed.samplesPerPixel).toBe(2)
    expect(xToSample(zoomed, 100)).toBe(xToSample(view, 100))
  })

  it('never zooms out beyond the capture or in beyond 200 px per sample', () => {
    const view = fitViewport(1024, COUNT)
    expect(zoomViewport(view, 0.5, 0, COUNT)).toEqual(view)
    expect(zoomViewport(view, 1e9, 0, COUNT).samplesPerPixel).toBe(1 / 200)
  })

  it('keeps the visible range inside the capture', () => {
    const view = { start: 0, samplesPerPixel: 10, width: 1000 }
    expect(panViewport(view, -50, COUNT).start).toBe(0)
    expect(panViewport(view, 1e9, COUNT).start).toBe(COUNT - 10_000)
    expect(clampViewport({ ...view, start: -5 }, COUNT).start).toBe(0)
  })

  it('shows a given range', () => {
    // The design's viewport: 1.86 to 2.60 ms at 25 MHz.
    const view = rangeViewport(1236, 46_500, 65_000, COUNT)
    expect(view.start).toBe(46_500)
    expect(viewportEnd(view)).toBeCloseTo(65_000)
  })
})
