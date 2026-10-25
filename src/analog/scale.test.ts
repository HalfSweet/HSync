import { describe, expect, it } from 'vitest'

import { fitScale, moveScale, stepScale, voltsAtY, VOLTS_PER_DIV, yOfVolts } from './scale'

describe('analog vertical scale', () => {
  it('steps through the 1-2-5 sequence from 1 mV to 50 V', () => {
    expect(VOLTS_PER_DIV.slice(0, 4)).toEqual([0.001, 0.002, 0.005, 0.01])
    expect(VOLTS_PER_DIV.at(-1)).toBe(50)
    expect(VOLTS_PER_DIV).toContain(0.5)
  })

  it('fits samples with room above and below', () => {
    // A 3.3 V sine: four divisions of 1 V from -0.5 V to 3.5 V.
    expect(fitScale(0, 3.3)).toEqual({ perDiv: 1, centre: 1.5 })
    expect(fitScale(-2.5, 2.5)).toEqual({ perDiv: 2, centre: 0 })
    expect(fitScale(-0.1, 0.1)).toEqual({ perDiv: 0.1, centre: 0 })
    // A flat line sits on the middle line at the finest scale.
    expect(fitScale(3.3, 3.3)).toEqual({ perDiv: 0.001, centre: 3.3 })
    expect(fitScale(Number.NaN, Number.NaN)).toEqual({ perDiv: 1, centre: 0 })
  })

  it('maps volts to rows and back', () => {
    const scale = { perDiv: 1, centre: 1.5 }
    expect(yOfVolts(scale, 1.5, 80)).toBe(40)
    expect(yOfVolts(scale, 3.5, 80)).toBe(0)
    expect(yOfVolts(scale, -0.5, 80)).toBe(80)
    expect(voltsAtY(scale, 20, 80)).toBe(2.5)
  })

  it('keeps the volts under the pointer while stepping', () => {
    const scale = { perDiv: 1, centre: 1.5 }
    const finer = stepScale(scale, -1, 20, 80)
    expect(finer.perDiv).toBe(0.5)
    expect(voltsAtY(finer, 20, 80)).toBeCloseTo(2.5)
    expect(stepScale(scale, 1, 40, 80)).toEqual({ perDiv: 2, centre: 1.5 })
    expect(stepScale({ perDiv: 50, centre: 0 }, 3, 40, 80).perDiv).toBe(50)
    expect(stepScale({ perDiv: 0.001, centre: 0 }, -2, 40, 80).perDiv).toBe(0.001)
  })

  it('moves the trace with the pointer', () => {
    // Dragging down a division shows the volts a division higher on the middle line.
    expect(moveScale({ perDiv: 2, centre: 0 }, 20, 80)).toEqual({ perDiv: 2, centre: 2 })
  })
})
