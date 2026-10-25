import { describe, expect, it } from 'vitest'

import { decodeAnalogBins, encodeAnalogBins } from './analogBins'

describe('analog bins', () => {
  it('round-trips columns, extents and empty columns', () => {
    const columns = new Float32Array([-1, 1, 0, 4, Number.NaN, Number.NaN])
    const encoded = encodeAnalogBins({
      start: 10,
      end: 16,
      bins: 3,
      channels: new Map([[0x8000, { low: -1, high: 4, columns }]]),
    })
    expect(encoded.byteLength).toBe(32 + 12 + 24)
    const decoded = decodeAnalogBins(encoded)
    expect([decoded.start, decoded.end, decoded.bins]).toEqual([10, 16, 3])
    const channel = decoded.channels.get(0x8000)
    expect([channel?.low, channel?.high]).toEqual([-1, 4])
    expect(Array.from(channel?.columns ?? []).slice(0, 4)).toEqual([-1, 1, 0, 4])
    expect(Number.isNaN(channel?.columns[4])).toBe(true)
  })

  it('rejects truncated data and other versions', () => {
    expect(() => decodeAnalogBins(new ArrayBuffer(8))).toThrow('truncated header')
    const buffer = new ArrayBuffer(32)
    new DataView(buffer).setUint32(0, 9, true)
    expect(() => decodeAnalogBins(buffer)).toThrow('unsupported version 9')
  })
})
