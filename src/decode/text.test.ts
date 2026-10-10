import { describe, expect, it } from 'vitest'

import { escapePieces, hex } from './text'

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
})
