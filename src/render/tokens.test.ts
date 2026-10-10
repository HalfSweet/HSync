import { describe, expect, it } from 'vitest'

import { withOpacity } from './tokens'

describe('withOpacity', () => {
  it('applies percentages and fractions', () => {
    expect(withOpacity('#171B1F', '7.5%')).toBe('rgb(23 27 31 / 0.075)')
    expect(withOpacity(' #ffffff ', '0.5')).toBe('rgb(255 255 255 / 0.5)')
  })

  it('leaves colours it cannot parse alone', () => {
    expect(withOpacity('rgb(1 2 3)', '50%')).toBe('rgb(1 2 3)')
  })
})
