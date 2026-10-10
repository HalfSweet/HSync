import { describe, expect, it } from 'vitest'

import { CELL_FIRST_HIGH, CELL_LAST_HIGH, CELL_TRANSITION } from '../bins'
import { createMockBackend, MockChannel } from './mockBackend'

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

  it('rejects queries for replaced captures', async () => {
    const backend = createMockBackend()
    const first = await backend.openDemo()
    await backend.openDemo()
    await expect(backend.pulse(first.id, 1, 0)).rejects.toMatchObject({
      code: 'error.session.staleCapture',
    })
  })
})
