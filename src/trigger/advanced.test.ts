import { describe, expect, it } from 'vitest'

import {
  advancedTrigger,
  conditionForKey,
  newSerial,
  newStage,
  nextCondition,
  normalizeValue,
  patternList,
  valueBits,
  valueFromHex,
  valueHex,
} from './advanced'

describe('trigger patterns', () => {
  it('lists the conditions set on the device channels', () => {
    expect(patternList({ 3: 'rise', 0: 'high', 5: 'none', 20: 'low' }, 16)).toEqual([
      [0, 'high'],
      [3, 'rise'],
    ])
  })

  it('cycles and reads conditions like DSView', () => {
    expect(nextCondition('none')).toBe('rise')
    expect(nextCondition('low')).toBe('none')
    expect(nextCondition('none', true)).toBe('low')
    expect(conditionForKey('R')).toBe('rise')
    expect(conditionForKey('c')).toBe('both')
    expect(conditionForKey('0')).toBe('low')
    expect(conditionForKey('q')).toBeNull()
  })
})

describe('serial data values', () => {
  it('reads bit positions, highest first', () => {
    expect(normalizeValue('1010 0101')).toBe('XXXXXXXX10100101')
    expect(normalizeValue('10a')).toBeNull()
    expect(valueBits('XXXXXXXX1010XXXX')).toEqual({ value: 0xa0, mask: 0xf0 })
  })

  it('converts the bits in use to and from hex', () => {
    expect(valueFromHex('a5', 8)).toBe('XXXXXXXX10100101')
    expect(valueFromHex('1a5', 8)).toBe('XXXXXXXX10100101')
    expect(valueFromHex('5', 3)).toBe('XXXXXXXXXXXXX101')
    expect(valueHex('XXXXXXXX10100101', 8)).toBe('A5')
    expect(valueHex('XXXXXXXX1010X101', 8)).toBeNull()
    expect(valueHex('0000000010100101', 16)).toBe('00A5')
    expect(valueFromHex('xyz', 8)).toBeNull()
  })
})

describe('advanced triggers', () => {
  it('sends the stages in use', () => {
    const first = { ...newStage(), pattern0: { 0: 'rise' as const }, count: 3 }
    const stages = [first, newStage(), newStage()]
    expect(advancedTrigger('stages', stages, 2, newSerial(), 16)).toEqual({
      mode: 'stages',
      stages: [
        {
          pattern0: [[0, 'rise']],
          pattern1: [],
          both: true,
          invert0: false,
          invert1: false,
          count: 3,
          contiguous: false,
        },
        {
          pattern0: [],
          pattern1: [],
          both: true,
          invert0: false,
          invert1: false,
          count: 1,
          contiguous: false,
        },
      ],
    })
  })

  it('sends the serial fields', () => {
    const serial = {
      ...newSerial(),
      clock: { 1: 'rise' as const },
      dataChannel: 2,
      value: 'XXXXXXXX10100101',
    }
    expect(advancedTrigger('serial', [], 1, serial, 16)).toEqual({
      mode: 'serial',
      start: [],
      stop: [],
      clock: [[1, 'rise']],
      dataChannel: 2,
      bits: 8,
      value: 0xa5,
      mask: 0xff,
    })
    expect(advancedTrigger('simple', [], 1, serial, 16)).toBeNull()
  })
})
