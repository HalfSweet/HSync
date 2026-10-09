import { describe, expect, it } from 'vitest'

import { binStart, CELL_FIRST_HIGH, CELL_LAST_HIGH, decodeBins, encodeBins } from './bins'

describe('bins', () => {
  const result = {
    start: 100,
    end: 200,
    bins: 3,
    channels: new Map([
      [7, Uint8Array.of(0, CELL_FIRST_HIGH, CELL_LAST_HIGH)],
      [2, Uint8Array.of(1, 2, 3)],
    ]),
  }

  it('round-trips through the binary layout', () => {
    const decoded = decodeBins(encodeBins(result))
    expect(decoded.start).toBe(100)
    expect(decoded.end).toBe(200)
    expect(decoded.bins).toBe(3)
    expect([...decoded.channels.keys()]).toEqual([7, 2])
    expect([...(decoded.channels.get(7) ?? [])]).toEqual([0, 1, 2])
  })

  it('rejects other versions and truncated data', () => {
    const buffer = encodeBins(result)
    new DataView(buffer).setUint32(0, 2, true)
    expect(() => decodeBins(buffer)).toThrow(/version/)
    expect(() => decodeBins(encodeBins(result).slice(0, 40))).toThrow(/truncated/)
  })

  it('locates the first sample of each column', () => {
    expect([0, 1, 2, 3].map((column) => binStart(result, column))).toEqual([100, 133, 166, 200])
  })
})
