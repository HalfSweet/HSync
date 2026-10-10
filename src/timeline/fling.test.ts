import { describe, expect, it } from 'vitest'

import { flingStep, releaseSpeed } from './fling'

describe('fling', () => {
  it('measures the release speed over the last moves only', () => {
    const moves = [
      { x: 0, t: 0 },
      { x: 10, t: 400 },
      { x: 60, t: 450 },
      { x: 110, t: 500 },
    ]
    expect(releaseSpeed(moves, 500)).toBeCloseTo(1)
    // A pointer held still before release does not fling.
    expect(releaseSpeed(moves, 700)).toBe(0)
    expect(releaseSpeed([{ x: 5, t: 10 }], 10)).toBe(0)
  })

  it('travels the same way at any frame rate and stops', () => {
    const travel = (frame: number) => {
      let speed = -2
      let moved = 0
      for (let step = flingStep(speed, frame); step; step = flingStep(speed, frame)) {
        moved += step.dx
        speed = step.speed
      }
      return moved
    }
    // The whole glide is speed × time constant, less what the stopping speed cuts off.
    expect(Math.abs(travel(16) + 650)).toBeLessThan(10)
    expect(Math.abs(travel(8) - travel(33))).toBeLessThan(10)
    expect(flingStep(0.01, 16)).toBeNull()
  })
})
