import { describe, expect, it } from 'vitest'

import { closestDuration, durationChoices, samplesFor } from './durations'

describe('durationChoices', () => {
  it('steps down a 1-2-5 sequence from the longest capture', () => {
    // 1 M samples at 1 MHz.
    const choices = durationChoices(1_000_000, 1_048_576)
    expect(choices[0]).toBeCloseTo(1.048576)
    expect(choices.slice(1, 6)).toEqual([1, 0.5, 0.2, 0.1, 0.05])
    // Ends before fewer than 1024 samples remain: 2 ms is 2000 samples, 1 ms only 1000.
    expect(choices.at(-1)).toBeCloseTo(0.002)
  })

  it('walks minutes and hours through 50 s and 50 min', () => {
    const choices = durationChoices(1000, 1000 * 2 * 3600)
    // As DSView: 2 h, 1 h, 50 min, then 20, 10, 5, 2, 1 min and 50 s.
    expect(choices.slice(0, 10)).toEqual([7200, 3600, 3000, 1200, 600, 300, 120, 60, 50, 20])
  })

  it('takes each step once at microseconds', () => {
    const choices = durationChoices(800_000_000, 2 ** 34)
    expect(choices.slice(-4)).toEqual([2e-5, 1e-5, 5e-6, 2e-6])
    expect(new Set(choices).size).toBe(choices.length)
  })

  it('offers nothing below the minimum capture', () => {
    expect(durationChoices(1_000_000, 512)).toEqual([])
    expect(durationChoices(0, 1 << 20)).toEqual([])
  })
})

describe('samplesFor', () => {
  it('rounds up to whole kilo-samples', () => {
    expect(samplesFor(0.001, 1_000_000)).toBe(1024)
    expect(samplesFor(1, 1_000_000)).toBe(1_000_448)
    expect(samplesFor(1e-6, 1000)).toBe(1024)
  })
})

describe('closestDuration', () => {
  it('picks the nearest choice on a log scale', () => {
    expect(closestDuration([1, 0.5, 0.2, 0.1], 0.3)).toBe(0.2)
    expect(closestDuration([1, 0.5, 0.2, 0.1], 0.4)).toBe(0.5)
    expect(closestDuration([], 1)).toBeUndefined()
  })
})
