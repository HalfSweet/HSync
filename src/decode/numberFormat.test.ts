import { describe, expect, it } from 'vitest'

import { entryFormat, formatNumber } from './numberFormat'

describe('number formats', () => {
  it('convert whole numbers', () => {
    expect(formatNumber(0x53, 'hex')).toBe('53')
    expect(formatNumber(0x123, 'hex')).toBe('0123')
    expect(formatNumber(5, 'bin')).toBe('00000101')
    expect(formatNumber(8, 'oct')).toBe('10')
    expect(formatNumber(1234, 'dec')).toBe('1234')
    expect(formatNumber(65, 'ascii')).toBe('A')
    expect(formatNumber(10, 'ascii')).toBe('[0A]')
    expect(formatNumber(-1, 'hex')).toBe('-01')
    expect(formatNumber(1.5, 'hex')).toBe('1.5')
  })

  it('read the format of an entry', () => {
    expect(entryFormat({ show: { numbers: 'dec' } })).toBe('dec')
    expect(entryFormat({ show: { numbers: 'nonsense' } })).toBe('hex')
    expect(entryFormat({})).toBe('hex')
    expect(entryFormat(undefined)).toBe('hex')
  })
})
