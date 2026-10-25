import { describe, expect, it } from 'vitest'

import { ANALOG_BASE } from '@/analog/channels'

import { mockAnalogBins } from './mockAnalog'

describe('mock analog channels', () => {
  const request = (start: number, end: number, bins: number) => ({
    captureId: 1,
    channels: [ANALOG_BASE, ANALOG_BASE + 1],
    start,
    end,
    bins,
  })

  it('covers whole periods with the full swing', () => {
    // 1 MHz: A0's period is 1000 samples.
    const result = mockAnalogBins(request(0, 4000, 2), 1_000_000, 4000)
    const a0 = result.channels.get(ANALOG_BASE)
    expect(Array.from(a0?.columns ?? [])).toEqual([0, 3.3, 0, 3.3].map(Math.fround))
    expect([a0?.low, a0?.high]).toEqual([0, 3.3])
  })

  it('takes the ends of a stretch without a peak', () => {
    // Samples 0..=100 rise from the centre without reaching the peak at 250.
    const a0 = mockAnalogBins(request(0, 101, 1), 1_000_000, 101).channels.get(ANALOG_BASE)
    expect(a0?.columns[0]).toBeCloseTo(1.65)
    expect(a0?.columns[1]).toBeCloseTo(1.65 + 1.65 * Math.sin((2 * Math.PI * 100) / 1000), 5)
  })

  it('leaves the columns past the captured samples empty', () => {
    const result = mockAnalogBins(request(0, 1000, 4), 1_000_000, 300)
    const a1 = result.channels.get(ANALOG_BASE + 1)
    expect(Number.isNaN(a1?.columns[2])).toBe(false)
    expect(Number.isNaN(a1?.columns[4])).toBe(true)
  })
})
