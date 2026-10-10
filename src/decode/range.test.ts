import { describe, expect, it } from 'vitest'

import { entryRange, RANGE_KEY, withRange } from './range'

describe('decode range', () => {
  it('reads stored ranges in either order', () => {
    expect(entryRange({ [RANGE_KEY]: [900, 100] })).toEqual([100, 900])
    expect(entryRange({ [RANGE_KEY]: [1] })).toBeNull()
    expect(entryRange({ id: '1:i2c' })).toBeNull()
    expect(entryRange(null)).toBeNull()
  })

  it('stores whole samples, and drops the range for the whole capture', () => {
    const entry = { id: '1:uart', options: {} }
    const ranged = withRange(entry, [1200.6, 40.2])
    expect(ranged[RANGE_KEY]).toEqual([40, 1201])
    expect(withRange(ranged, null)).toEqual(entry)
  })
})
