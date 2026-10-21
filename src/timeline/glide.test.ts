import { describe, expect, it } from 'vitest'

import { between, easeOut, glideAt, glideDone, type Glide } from './glide'
import { panViewport, viewportEnd, xToSample, zoomViewport, type Viewport } from './viewport'

const EXTENT = 1_000_000
const view: Viewport = { start: 200_000, samplesPerPixel: 100, width: 1000 }

describe('easeOut', () => {
  it('is quadratic: three quarters of the way at half the time, at rest at the end', () => {
    expect(easeOut(0)).toBe(0)
    expect(easeOut(0.5)).toBe(0.75)
    expect(easeOut(1)).toBe(1)
    // Twice the average speed at the start, none at the end.
    expect(easeOut(0.001) / 0.001).toBeCloseTo(2, 2)
    expect((1 - easeOut(0.999)) / 0.001).toBeCloseTo(0, 2)
  })
})

describe('between', () => {
  it('keeps the sample under the pointer still while zooming around it', () => {
    const to = zoomViewport(view, 8, 300, EXTENT)
    for (const p of [0, 0.1, 0.25, 0.5, 0.75, 0.9, 1]) {
      expect(xToSample(between(view, to, p), 300)).toBeCloseTo(xToSample(view, 300), 6)
    }
  })

  it('zooms by the same ratio in equal steps', () => {
    const to = zoomViewport(view, 16, 500, EXTENT)
    expect(between(view, to, 0.5).samplesPerPixel).toBeCloseTo(view.samplesPerPixel / 4)
    expect(between(view, to, 0.25).samplesPerPixel).toBeCloseTo(view.samplesPerPixel / 2)
  })

  it('pans evenly', () => {
    const to = panViewport(view, 400, EXTENT)
    expect(between(view, to, 0.5)).toEqual({ ...view, start: view.start + 200 * 100 })
  })

  it('stays within the capture when both ends are', () => {
    const ends: Viewport[] = [
      { start: 0, samplesPerPixel: 1000, width: 1000 },
      { start: 0, samplesPerPixel: 0.01, width: 1000 },
      { start: EXTENT - 10, samplesPerPixel: 0.01, width: 1000 },
      { start: 500_000, samplesPerPixel: 2, width: 1000 },
      { start: 250_000, samplesPerPixel: 500, width: 1000 },
    ]
    for (const from of ends) {
      for (const to of ends) {
        for (let p = 0; p <= 1; p += 0.05) {
          const step = between(from, to, p)
          expect(step.start).toBeGreaterThanOrEqual(-1e-6)
          expect(viewportEnd(step)).toBeLessThanOrEqual(EXTENT + 1e-6)
        }
      }
    }
  })
})

describe('glideAt', () => {
  const glide: Glide = {
    from: view,
    to: zoomViewport(view, 2, 500, EXTENT),
    start: 1000,
    duration: 100,
  }

  it('eases from one view to the other over its duration', () => {
    expect(glideAt(glide, 1000)).toEqual(view)
    expect(glideAt(glide, 1050)).toEqual(between(glide.from, glide.to, 0.75))
    expect(glideAt(glide, 1100)).toEqual(glide.to)
    expect(glideAt(glide, 2000)).toEqual(glide.to)
    expect(glideDone(glide, 1099)).toBe(false)
    expect(glideDone(glide, 1100)).toBe(true)
  })

  it('ends at once without a duration', () => {
    expect(glideAt({ ...glide, duration: 0 }, 1000)).toEqual(glide.to)
  })
})
