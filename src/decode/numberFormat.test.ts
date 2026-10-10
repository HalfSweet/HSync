import { describe, expect, it } from 'vitest'

import { entryFormat, formatValue, shownTexts } from './numberFormat'

describe('number formats', () => {
  it('convert like DSView', () => {
    expect(formatValue('4F', 'hex')).toBe('4F')
    expect(formatValue('4F', 'dec')).toBe('79')
    expect(formatValue('4F', 'bin')).toBe('01001111')
    expect(formatValue('4F', 'oct')).toBe('117')
    expect(formatValue('0F', 'oct')).toBe('017')
    expect(formatValue('41', 'ascii')).toBe('A')
    expect(formatValue('0A', 'ascii')).toBe('[0A]')
    expect(formatValue('12 34', 'dec')).toBe('18 52')
  })

  it('fill in the texts', () => {
    expect(shownTexts(['Data: {$}', '{$}'], '4F', 'dec')).toEqual(['Data: 79', '79'])
    expect(shownTexts([], '41', 'ascii')).toEqual(['A'])
    expect(shownTexts(['Start'], null, 'hex')).toEqual(['Start'])
  })

  it('read the format of an entry', () => {
    expect(entryFormat({ 'number format': 'bin' })).toBe('bin')
    expect(entryFormat({ 'number format': 'weird' })).toBe('hex')
    expect(entryFormat(undefined)).toBe('hex')
  })
})
