import { describe, expect, it } from 'vitest'

import { windowTitle } from './useWindowTitle'

describe('window title', () => {
  it('names the open file', () => {
    expect(windowTitle({ kind: 'file', path: '/captures/i2c.dsl' })).toBe('i2c.dsl — HSync')
    expect(windowTitle({ kind: 'file', path: 'C:\\captures\\spi.dsl' })).toBe('spi.dsl — HSync')
    expect(windowTitle({ kind: 'device' })).toBe('HSync')
    expect(windowTitle(undefined)).toBe('HSync')
  })
})
