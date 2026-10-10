import { describe, expect, it } from 'vitest'

import { defaultCaptureName } from './useFileActions'

describe('defaultCaptureName', () => {
  it('stamps the date and time', () => {
    expect(defaultCaptureName(new Date(2026, 9, 10, 14, 5, 9))).toBe('HSync-20261010-140509.dsl')
  })
})
