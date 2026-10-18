import { describe, expect, it } from 'vitest'

import { placeFloat, POINTER_GAP, type FloatAnchor } from './float'

const bounds = { left: 0, right: 800, top: 38, bottom: 600 }
const size = { width: 160, height: 22 }
const row = (x: number, align: FloatAnchor['align'] = 'beside'): FloatAnchor => ({
  x,
  align,
  top: 200,
  bottom: 230,
  side: 'above',
})

describe('placeFloat', () => {
  it('sits on the row, just right of the pointer', () => {
    expect(placeFloat(row(300), size, bounds)).toEqual({ left: 300 + POINTER_GAP, top: 178 })
  })

  it('goes left of the pointer near the right edge', () => {
    expect(placeFloat(row(700), size, bounds)).toEqual({ left: 700 - POINTER_GAP - 160, top: 178 })
  })

  it('centres on a measurement and stays inside the tracks', () => {
    expect(placeFloat(row(400, 'centre'), size, bounds).left).toBe(320)
    expect(placeFloat(row(30, 'centre'), size, bounds).left).toBe(0)
    expect(placeFloat(row(790, 'centre'), size, bounds).left).toBe(640)
  })

  it('keeps clear of the label column when the rows scroll sideways', () => {
    expect(placeFloat(row(130), size, { ...bounds, left: 120 }).left).toBe(142)
    expect(placeFloat(row(130, 'centre'), size, { ...bounds, left: 120 }).left).toBe(120)
  })

  it('keeps the start of a read-out wider than the tracks in view', () => {
    expect(placeFloat(row(100), { width: 900, height: 22 }, bounds).left).toBe(0)
  })

  it('goes under the row when the ruler leaves no room above it', () => {
    const first = { ...row(300), top: 40, bottom: 70 }
    expect(placeFloat(first, size, bounds).top).toBe(70)
  })

  it('takes the side asked for, or the other when the bottom edge cuts it off', () => {
    const below: FloatAnchor = { ...row(300), side: 'below' }
    expect(placeFloat(below, size, bounds).top).toBe(230)
    expect(placeFloat({ ...below, top: 570, bottom: 590 }, size, bounds).top).toBe(548)
  })

  it('stays on its side when neither has room', () => {
    const cramped = { ...bounds, top: 190, bottom: 240 }
    expect(placeFloat(row(300), size, cramped).top).toBe(178)
  })
})
