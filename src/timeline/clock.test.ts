import { describe, expect, it } from 'vitest'

import { I2C_RATES, nominalRate } from './clock'

describe('nominalRate', () => {
  it('snaps measured I2C clocks to bus speeds', () => {
    expect(nominalRate(25e6 / 251, I2C_RATES)).toBe(100e3)
    expect(nominalRate(395e3, I2C_RATES)).toBe(400e3)
  })

  it('leaves other rates alone', () => {
    expect(nominalRate(250e3, I2C_RATES)).toBeNull()
  })
})
