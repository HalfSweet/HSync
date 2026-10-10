import { describe, expect, it } from 'vitest'

import { isPattern, parsePattern, resolveChannel } from './pattern'

const CHANNELS = [
  { index: 0, names: ['SDA'] },
  { index: 1, names: ['SCL'] },
  { index: 13, names: ['CS#', 'CS'] },
]

describe('level patterns', () => {
  it('tells patterns from decoded values', () => {
    expect(isPattern('D3=R D5=1')).toBe(true)
    expect(isPattern(' sda=f ')).toBe(true)
    expect(isPattern('4F')).toBe(false)
    expect(isPattern('D3=R HOST')).toBe(false)
    expect(isPattern('D3=Q')).toBe(false)
    expect(isPattern('')).toBe(false)
  })

  it('names channels by number or name', () => {
    expect(resolveChannel('D1', CHANNELS)).toBe(1)
    expect(resolveChannel('13', CHANNELS)).toBe(13)
    expect(resolveChannel('cs', CHANNELS)).toBe(13)
    expect(resolveChannel('D7', CHANNELS)).toBeNull()
    expect(resolveChannel('MISO', CHANNELS)).toBeNull()
  })

  it('reads conditions', () => {
    expect(parsePattern('SDA=F d1=1 13=x', CHANNELS)).toEqual({
      kind: 'conditions',
      conditions: [
        { channel: 0, level: 'falling' },
        { channel: 1, level: 'high' },
      ],
    })
    expect(parsePattern('D0=R D0=C', CHANNELS)).toEqual({
      kind: 'conditions',
      conditions: [{ channel: 0, level: 'edge' }],
    })
    expect(parsePattern('D9=1', CHANNELS)).toEqual({ kind: 'unknownChannel', name: 'D9' })
    expect(parsePattern('HOST', CHANNELS)).toBeNull()
  })
})
