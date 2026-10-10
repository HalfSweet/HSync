import { describe, expect, it } from 'vitest'

import { escapePieces, hex, i2cMeaningText } from './text'

describe('decoded text', () => {
  it('formats hex', () => {
    expect(hex(0x4f)).toBe('4F')
    expect(hex(0x100, 3)).toBe('100')
  })

  it('escapes control characters', () => {
    expect(escapePieces('MX: 1\r\n')).toEqual([
      { text: 'MX: 1', escape: false },
      { text: '\\r', escape: true },
      { text: '\\n', escape: true },
    ])
    expect(escapePieces('a\u0001')).toEqual([
      { text: 'a', escape: false },
      { text: '\\x01', escape: true },
    ])
  })

  it('describes register reads', () => {
    const t = (key: string, params?: Record<string, unknown>) => `${key} ${JSON.stringify(params)}`
    expect(i2cMeaningText({ type: 'readRegister', register: 4, data: [0x4f] }, 0x62, t)).toBe(
      'decode.i2c.readRegister {"register":"04","data":"4F"}',
    )
  })
})
