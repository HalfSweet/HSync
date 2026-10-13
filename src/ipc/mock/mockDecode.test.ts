import { describe, expect, it } from 'vitest'

import {
  MOCK_ENTRIES,
  mockCoverage,
  mockRowAt,
  mockSearch,
  mockStatuses,
  mockStream,
  mockView,
  parseNeedle,
} from './mockDecode'

describe('mock decoders', () => {
  it('answer statuses from the recording', () => {
    const statuses = mockStatuses(MOCK_ENTRIES, false)
    expect(statuses.map((status) => status.kinds)).toEqual([
      ['i2c', 'eeprom24xx'],
      ['uart'],
      ['spi'],
      ['can'],
    ])
    expect(statuses.map((status) => status.streams.length)).toEqual([2, 1, 1, 1])
    expect(statuses.every((status) => status.state === 'done')).toBe(true)
    expect(statuses[1]?.streams[0]?.progress.events).toBe(47)
    expect(mockStatuses(MOCK_ENTRIES, true)[0]?.state).toBe('running')
    const unknown = mockStatuses([{ nodes: { x: { type: 'pwm' } } }], false)
    expect(unknown[0]?.state).toBe('invalid')
  })

  it('answers lanes like the store', () => {
    // The design's viewport, 1236 px wide.
    const lanes = mockView(MOCK_ENTRIES, 46_500, 65_000, 15)
    expect(lanes.map((lane) => lane.index)).toEqual([0, 1, 2, 3])
    const uart = lanes[1]?.streams[0]?.view
    expect(
      uart?.blocks.some((block) => block.type === 'event' && block.payload.value === 0x53),
    ).toBe(true)
    // Zoomed out, the SPI words pack into dense blocks.
    const whole = mockView(MOCK_ENTRIES, 0, 131_072, 131_072 / 100)
    const spi = whole[2]?.streams[0]?.view
    expect(spi?.blocks.some((block) => block.type === 'dense')).toBe(true)
  })

  it('pages the listings', () => {
    const page = mockStream(MOCK_ENTRIES, 1, 0, null, false, 0, 10)
    expect([page?.total, page?.events, page?.rows.length]).toEqual([47, 47, 10])
    expect(page?.rows[0]?.kind).toBe('frame')
    const first = page?.rows[0]?.start ?? 0
    const scoped = mockStream(MOCK_ENTRIES, 1, 0, [first, first + 10_000], true, 0, 100)
    expect(scoped?.total).toBeGreaterThan(0)
    expect(scoped?.total).toBeLessThan(47)
    expect(scoped?.inside).toBeGreaterThan(0)
    expect(mockStream(MOCK_ENTRIES, 9, 0, null, false, 0, 10)).toBeNull()
    expect(mockRowAt(MOCK_ENTRIES, 1, 0, 0)).toBe(0)
    expect(mockRowAt(MOCK_ENTRIES, 1, 0, first + 1)).toBe(0)
    expect(mockRowAt(MOCK_ENTRIES, 1, 0, 10_000_000)).toBe(47)
  })

  it('searches like the store', () => {
    expect(parseNeedle('frame 0x41 65 -2')).toEqual({
      sequences: [[0x41, 65, -2]],
      names: ['frame'],
    })
    expect(parseNeedle('  ')).toBeNull()
    const frames = mockSearch(MOCK_ENTRIES, 'frame 0x53')
    expect(frames?.hits.length).toBeGreaterThan(0)
    expect(frames?.hits.every((hit) => hit.decoder === 1)).toBe(true)
    const data = mockSearch(MOCK_ENTRIES, 'data')
    const decoders = new Set(data?.hits.map((hit) => hit.decoder))
    expect(decoders.size).toBeGreaterThanOrEqual(2)
    expect(
      data?.hits.every((hit, at, all) => at === 0 || (all[at - 1]?.start ?? 0) <= hit.start),
    ).toBe(true)
    // Field names never match.
    expect(mockSearch(MOCK_ENTRIES, 'value')?.hits).toEqual([])
    expect(mockSearch(MOCK_ENTRIES, '  ')).toBeNull()
  })

  it('tells where a decoder has events', () => {
    const coverage = mockCoverage(MOCK_ENTRIES, 1, 0, 131_072, 1000)
    expect(coverage?.length).toBeGreaterThan(0)
    expect(coverage?.every(([start, end]) => start < end)).toBe(true)
    expect(mockCoverage(MOCK_ENTRIES, 9, 0, 131_072, 1000)).toBeNull()
  })
})
