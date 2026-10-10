import { describe, expect, it } from 'vitest'

import fixture from './fixtures/protocol-demo.json'
import { mockSearch, mockStatuses, mockStream, mockView } from './mockDecode'

const DECODERS = fixture.decoders

// The design's viewport, 1.86–2.6 ms at 25 MHz on a 1236 px track.
const START = 46_500
const END = 65_000
const SPP = (END - START) / 1236

describe('mock decoders', () => {
  it('answers lanes like hsync-decode', () => {
    const [i2c, uart, spi, can] = mockView(DECODERS, START, END, SPP)
    const bytes = i2c?.view.lanes[0]?.items.flatMap((item) =>
      item.type === 'i2cAddress' ? [item.address] : item.type === 'i2cData' ? [item.value] : [],
    )
    expect(bytes).toEqual([0x62, 0x04, 0x62, 0x4f, 0x62, 0x02, 0x62, 0x32])
    const text = uart?.view.lanes[0]?.items
      .flatMap((item) => (item.type === 'uartFrame' ? [String.fromCharCode(item.value)] : []))
      .join('')
    expect(text).toBe('USB-based ')
    expect(spi?.view.lanes.map((lane) => lane.lane)).toEqual(['miso', 'mosi'])
    expect(spi?.view.lanes[1]?.items.every((item) => item.type === 'spiWord')).toBe(true)
    expect(can?.view.lanes.every((lane) => lane.items.length === 0)).toBe(true)
    expect(can?.view.next).not.toBeNull()
  })

  it('pages the data stream', () => {
    const page = mockStream(DECODERS, 0, 'text', null, false, 0, 4)
    expect(page?.total).toBe(6)
    expect(page?.rows).toHaveLength(4)
    expect(mockStatuses(DECODERS, false).every((status) => status.state === 'done')).toBe(true)
  })

  it('searches like hsync-decode', () => {
    // 0x4F is also the O of the SPI text.
    const register = mockSearch(DECODERS, '4F', 'text')?.hits.filter((hit) => hit.decoder === 0)
    expect(register).toHaveLength(1)
    expect(register?.[0]?.row).toEqual(expect.any(Number))
    expect(mockSearch(DECODERS, '32 BB', 'text')?.hits).toHaveLength(3)
    const host = mockSearch(DECODERS, 'host', 'hex')?.hits ?? []
    expect(host.length).toBeGreaterThan(0)
    expect(host.every((hit) => hit.decoder === 2 && hit.row !== null)).toBe(true)
    expect(
      mockSearch(DECODERS, '0x100', 'text')?.hits.filter((hit) => hit.decoder === 3),
    ).toHaveLength(3)
    expect(mockSearch(DECODERS, '  ', 'text')).toBeNull()
  })
})
