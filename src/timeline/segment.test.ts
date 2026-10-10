import { describe, expect, it } from 'vitest'

import { segmentGeometry } from './segment'

const view = { start: 0, samplesPerPixel: 1, width: 1000 }

describe('segmentGeometry', () => {
  it('labels a wide segment between its flags', () => {
    const geometry = segmentGeometry(100, 600, view)
    expect(geometry.band).toEqual({ left: 100, width: 500 })
    expect(geometry.a).toEqual({ x: 100, visible: true, flag: 84 })
    expect(geometry.b).toEqual({ x: 600, visible: true, flag: 602 })
    expect(geometry.label).toEqual({
      kind: 'between',
      left: 100,
      width: 500,
      arrowA: true,
      arrowB: true,
    })
    expect(geometry.open).toBe(false)
  })

  it('labels a narrow segment beside B, or beside A near the right edge', () => {
    expect(segmentGeometry(100, 150, view).label).toEqual({ kind: 'besideB', left: 170 })
    expect(segmentGeometry(900, 950, view).label).toEqual({ kind: 'besideA', right: 120 })
  })

  it('turns flags inward at the edges of the view', () => {
    expect(segmentGeometry(5, 600, view).a.flag).toBe(7)
    expect(segmentGeometry(100, 995, view).b.flag).toBe(979)
  })

  it('marks segments that leave the view as open', () => {
    const geometry = segmentGeometry(-200, 400, view)
    expect(geometry.a.visible).toBe(false)
    expect(geometry.band).toEqual({ left: 0, width: 400 })
    expect(geometry.open).toBe(true)
    expect(geometry.label).toMatchObject({ kind: 'between', arrowA: false, arrowB: true })
  })

  it('has no label until the drag moves', () => {
    expect(segmentGeometry(300, 300, view).label).toBeNull()
  })
})
