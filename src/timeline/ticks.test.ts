import { describe, expect, it } from 'vitest'

import { LABEL_GAP, labelWidth, overviewTicks, rulerTicks } from './ticks'
import { rangeViewport } from './viewport'

const base = { samplerate: 25_000_000, triggerSample: 0 }

describe('rulerTicks', () => {
  it('reproduces the design ruler: 50 µs per division, labels in ms', () => {
    // 1.860 to 2.600 ms over a 1236 px track.
    const view = rangeViewport(1236, 46_500, 65_000, 131_072)
    const ruler = rulerTicks(view, base)
    expect(ruler.step).toBeCloseTo(50e-6)
    expect(ruler.unit).toBe('ms')
    const majors = ruler.ticks.filter((tick) => tick.major).map((tick) => tick.label)
    expect(majors[0]).toBe('1.90')
    expect(majors.at(-1)).toBe('2.60')
    // Minor ticks every 10 µs.
    const [first, second] = ruler.ticks
    expect((second?.time ?? 0) - (first?.time ?? 0)).toBeCloseTo(10e-6)
  })

  it('places ticks relative to the trigger', () => {
    const view = { start: 0, samplesPerPixel: 25, width: 1000 } // 1 µs per px
    const ruler = rulerTicks(view, { samplerate: 25_000_000, triggerSample: 12_500 })
    const zero = ruler.ticks.find((tick) => tick.time === 0)
    expect(zero?.x).toBeCloseTo(500)
    expect(ruler.ticks.some((tick) => tick.time < 0)).toBe(true)
    expect(zero?.label).toBe('0')
  })

  it('keeps long labels apart deep into a long capture', () => {
    // Around 10.8 s at 100 MHz, 0.14 ns a pixel: 10 ns would be 71 px, too close for
    // labels such as 10.80115195.
    const fast = { samplerate: 100_000_000, triggerSample: 0 }
    const view = { start: 1_080_115_195, samplesPerPixel: 1 / 71, width: 1185 }
    const ruler = rulerTicks(view, fast)
    expect(ruler.unit).toBe('s')
    expect(ruler.step * 1e9).toBeCloseTo(20)
    const majors = ruler.ticks.filter((tick) => tick.major)
    expect(majors[0]?.label).toBe('10.80115196')
    for (const [index, tick] of majors.entries()) {
      const next = majors[index + 1]
      if (!next) break
      const room = (labelWidth(tick.label) + labelWidth(next.label)) / 2 + LABEL_GAP
      expect(next.x - tick.x).toBeGreaterThanOrEqual(room)
    }
  })

  it('measures labels in the ruler font', () => {
    // 11 px IBM Plex Sans: figures 6.6 px, the point 3 px.
    expect(labelWidth('10.80115195')).toBeCloseTo(69)
    expect(labelWidth('-1.25')).toBeCloseTo(27.2, 0)
  })

  it('returns nothing without a width or samplerate', () => {
    expect(rulerTicks({ start: 0, samplesPerPixel: 1, width: 0 }, base).ticks).toEqual([])
  })
})

describe('overviewTicks', () => {
  it('labels whole milliseconds across the demo capture', () => {
    const ruler = overviewTicks(1236, 131_072, base)
    expect(ruler.unit).toBe('ms')
    expect(ruler.ticks.map((tick) => tick.label)).toEqual(['0', '1', '2', '3', '4', '5'])
  })
})
