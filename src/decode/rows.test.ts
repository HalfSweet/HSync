import { describe, expect, it } from 'vitest'

import { laneShown, showKey, withLaneShown } from './rows'

const fields = { lane: 'eeprom24xx/fields', desc: 'Fields' }

describe('annotation rows', () => {
  it('use DSView show keys', () => {
    expect(showKey(fields)).toBe('eeprom24xx: Fields')
    expect(showKey({ lane: '1:i2c/addr-data', desc: 'Address/Data' })).toBe('1:i2c: Address/Data')
  })

  it('show unless hidden', () => {
    const entry = { id: '1:i2c', show: { 'eeprom24xx: Fields': false } }
    expect(laneShown(entry, fields)).toBe(false)
    expect(laneShown({ id: '1:i2c' }, fields)).toBe(true)
    const shownAgain = withLaneShown(entry, fields, true)
    expect(laneShown(shownAgain, fields)).toBe(true)
    expect((shownAgain as { id: string }).id).toBe('1:i2c')
  })
})
