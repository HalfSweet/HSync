import { describe, expect, it } from 'vitest'

import { covers, overscan } from './window'

describe('fetch windows', () => {
  const visible = { start: 1000, end: 2000, samplesPerPixel: 1 }

  it('reach a view width beyond the view, within the capture', () => {
    expect(overscan(visible, 10_000)).toEqual({ start: 0, end: 3000, samplesPerPixel: 1 })
    expect(overscan({ start: 8500, end: 9500.5, samplesPerPixel: 2 }, 10_000)).toEqual({
      start: 7499,
      end: 10_000,
      samplesPerPixel: 2,
    })
  })

  it('serve views inside them at the same zoom', () => {
    const fetched = overscan(visible, 10_000)
    expect(covers(fetched, { start: 1900, end: 2900, samplesPerPixel: 1 }, 10_000)).toBe(true)
    expect(covers(fetched, { start: 2100, end: 3100, samplesPerPixel: 1 }, 10_000)).toBe(false)
    // Zoomed: other columns are needed.
    expect(covers(fetched, { start: 1000, end: 1500, samplesPerPixel: 0.5 }, 10_000)).toBe(false)
    expect(covers(fetched, { start: 1000, end: 2000, samplesPerPixel: 1.01 }, 10_000)).toBe(true)
  })

  it('ignore the part of a view past the end of the capture', () => {
    const fetched = { start: 7499, end: 10_000, samplesPerPixel: 2 }
    expect(covers(fetched, { start: 9000, end: 11_000, samplesPerPixel: 2 }, 10_000)).toBe(true)
  })
})
