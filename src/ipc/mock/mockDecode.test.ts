import { describe, expect, it } from 'vitest'

import fixture from './fixtures/protocol-demo.json'
import { mockStatuses, mockStream, mockView } from './mockDecode'

const DECODERS = fixture.decoders

// The design's viewport, 1.86–2.6 ms at 25 MHz on a 1236 px track.
const START = 46_500
const END = 65_000
const SPP = (END - START) / 1236

describe('mock decoders', () => {
  it('answers lanes like hsync-decode', () => {
    const [i2c, uart, spi, can] = mockView(DECODERS, START, END, SPP)
    const meanings = i2c?.view.lanes[0]?.items.flatMap((item) =>
      item.type === 'i2cTransfer' ? [item.meaning] : [],
    )
    expect(meanings).toEqual([
      { type: 'readRegister', register: 0x04, data: [0x4f] },
      { type: 'readRegister', register: 0x02, data: [0x32, 0xbb] },
    ])
    expect(i2c?.view.address).toBe(0x62)
    const text = uart?.view.lanes[0]?.items
      .flatMap((item) => (item.type === 'uartFrame' ? [String.fromCharCode(item.value)] : []))
      .join('')
    expect(text).toBe('USB-based ')
    const transfer = spi?.view.lanes[0]?.items[0]
    expect(transfer?.type).toBe('spiTransfer')
    if (transfer?.type === 'spiTransfer') expect(transfer.last).toBeGreaterThan(transfer.first)
    expect(can?.view.lanes.every((lane) => lane.items.length === 0)).toBe(true)
    expect(can?.view.next).not.toBeNull()
  })

  it('pages the data stream', () => {
    const page = mockStream(DECODERS, 0, 'text', null, false, 0, 4)
    expect(page?.total).toBe(6)
    expect(page?.rows).toHaveLength(4)
    expect(mockStatuses(DECODERS, false).every((status) => status.state === 'done')).toBe(true)
  })
})
