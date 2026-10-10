import { describe, expect, it } from 'vitest'

import { defaultCaptureName, withExtension } from './useFileActions'

describe('file names', () => {
  it('stamps the date and time', () => {
    expect(defaultCaptureName(new Date(2026, 9, 10, 14, 5, 9))).toBe('HSync-20261010-140509.dsl')
    expect(defaultCaptureName(new Date(2026, 9, 10, 14, 5, 9), 'vcd')).toBe(
      'HSync-20261010-140509.vcd',
    )
  })

  it('adds a missing extension', () => {
    expect(withExtension('/tmp/a', 'csv')).toBe('/tmp/a.csv')
    expect(withExtension('/tmp/a.CSV', 'csv')).toBe('/tmp/a.CSV')
  })
})
