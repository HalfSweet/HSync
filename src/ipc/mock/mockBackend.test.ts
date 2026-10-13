import { describe, expect, it } from 'vitest'

import { CELL_FIRST_HIGH, CELL_LAST_HIGH, CELL_TRANSITION } from '../bins'
import { createMockBackend, findPattern, MockChannel } from './mockBackend'

describe('MockChannel', () => {
  // Low for 0..10, high for 10..15, low for 15..20.
  const channel = new MockChannel(false, [10, 15], 20)

  it('answers point queries like hsync-capture', () => {
    expect(channel.sample(9)).toBe(false)
    expect(channel.sample(10)).toBe(true)
    expect(channel.nextChange(3)).toBe(10)
    expect(channel.nextChange(15)).toBeNull()
    expect(channel.prevChange(12)).toBe(9)
    expect(channel.prevChange(5)).toBeNull()
  })

  it('summarises bins', () => {
    const high = CELL_FIRST_HIGH | CELL_LAST_HIGH
    expect([...channel.bins(0, 20, 4)]).toEqual([0, 0, high, 0])
    expect([...channel.bins(0, 20, 2)]).toEqual([0, CELL_FIRST_HIGH | CELL_TRANSITION])
  })
})

describe('createMockBackend', () => {
  it('replays the demo capture', async () => {
    const backend = createMockBackend()
    const summary = await backend.openDemo()
    expect(summary.info.samplerate).toBe(25_000_000)
    expect(summary.channels[1]?.name).toBe('SCL')
    // The first SCL edge, as asserted for the Rust workspace.
    expect(await backend.edge(summary.id, 1, 0, 'next')).toBe(45_479)
    const bins = await backend.bins({
      captureId: summary.id,
      channels: [1],
      start: 0,
      end: 1000,
      bins: 10,
    })
    expect(bins.channels.get(1)?.every((cell) => cell === (CELL_FIRST_HIGH | CELL_LAST_HIGH))).toBe(
      true,
    )
  })

  it('measures clocks like the Rust session', async () => {
    const backend = createMockBackend()
    const summary = await backend.openDemo()
    expect(await backend.clockPeriod(summary.id, 1)).toBe(251)
    expect(await backend.clockPeriod(summary.id, 12)).toBe(19)
  })

  it('counts edges within a segment like the Rust session', async () => {
    const backend = createMockBackend()
    const summary = await backend.openDemo()
    // SCL (D1) starts high; its first edge falls at 45 479.
    expect(await backend.edgeCounts(summary.id, 0, 45_479, [1])).toEqual([
      { channel: 1, rising: 0, falling: 1 },
    ])
    expect(await backend.edgeCounts(summary.id, 45_478, 0, [1])).toEqual([
      { channel: 1, rising: 0, falling: 0 },
    ])
  })

  it('rejects queries for replaced captures', async () => {
    const backend = createMockBackend()
    const first = await backend.openDemo()
    await backend.openDemo()
    await expect(backend.pulse(first.id, 1, 0)).rejects.toMatchObject({
      code: 'error.session.staleCapture',
    })
  })
})

describe('findPattern', () => {
  // Low for 0..10, high for 10..15, low for 15..20; and high for 0..12, low after.
  const a = new MockChannel(false, [10, 15], 20)
  const b = new MockChannel(true, [12], 20)

  it('hits levels at the start of each stretch and every edge', () => {
    const both = [
      { channel: a, level: 'high' as const },
      { channel: b, level: 'high' as const },
    ]
    expect(findPattern(both, 20, 0, true)).toBe(10)
    expect(findPattern(both, 20, 11, true)).toBeNull()
    expect(findPattern(both, 20, 19, false)).toBe(10)
    expect(findPattern([{ channel: a, level: 'edge' }], 20, 11, true)).toBe(15)
    expect(findPattern([{ channel: a, level: 'falling' }], 20, 14, false)).toBeNull()
  })

  it('finds the I2C start conditions the decoder found', async () => {
    const backend = createMockBackend()
    const summary = await backend.openDemo()
    // SDA (D0) falls while SCL (D1) is high.
    const conditions = [
      { channel: 0, level: 'falling' as const },
      { channel: 1, level: 'high' as const },
    ]
    const found = await backend.find(summary.id, conditions, 0, 'next')
    const page = await backend.decodeStream({
      captureId: summary.id,
      index: 0,
      stream: 0,
      segment: null,
      onlySegment: false,
      offset: 0,
      count: 1000,
    })
    const starts = page?.rows.filter((row) => row.kind === 'begin').map((row) => row.start) ?? []
    expect(starts.length).toBeGreaterThan(0)
    expect(found).toBe(starts[0])
  })
})
