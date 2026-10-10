import { describe, expect, it } from 'vitest'

import fixture from '@/ipc/mock/fixtures/protocol-demo.json'

import { groupChannels, uartFormat } from './groups'
import { collapsedByDefault, COMFORTABLE_METRICS, layoutRows, visibleRows } from './layout'

const channels = fixture.channels.map(({ index, name }) => ({ index, name }))
const groups = groupChannels(channels, fixture.decoders)

describe('groupChannels', () => {
  it('groups the demo capture by decoder', () => {
    // Channels named in the file show their names; the role goes beside them.
    expect(groups.map((group) => [group.title, group.channels.map((c) => c.label)])).toEqual([
      ['I²C', ['SDA', 'SCL']],
      ['UART', ['UART']],
      ['SPI', ['MISO', 'MOSI', 'CLK', 'CS#']],
      ['CAN-FD', ['CAN-FD']],
      ['', ['D2', 'D3', 'D4', 'D6', 'D7', 'D8', 'D10', 'D11']],
    ])
    expect(groups[1]?.channels[0]?.roleLabel).toBe('RX')
    expect(groups[0]?.channels[0]?.roleLabel).toBeUndefined()
    expect(groups.map((group) => group.kind)).toEqual(['i2c', 'uart', 'spi', 'can', 'other'])
    expect(groups.at(-1)?.unnamed).toBe(true)
  })

  it('colours wires by channel number', () => {
    const spi = groups[2]?.channels.map((channel) => [channel.index, channel.wire])
    expect(spi).toEqual([
      [15, 7],
      [14, 6],
      [12, 4],
      [13, 5],
    ])
  })

  it('shows a channel renamed within a decoder group by its name', () => {
    const renamed = groupChannels(
      channels.map((channel) => (channel.index === 1 ? { ...channel, name: 'Clock' } : channel)),
      fixture.decoders,
    )
    const scl = renamed[0]?.channels[1]
    expect([scl?.label, scl?.roleLabel]).toEqual(['Clock', 'SCL'])
    // Back to its number, it shows its role again.
    const plain = groupChannels(
      channels.map((channel) => (channel.index === 1 ? { ...channel, name: '1' } : channel)),
      fixture.decoders,
    )
    expect([plain[0]?.channels[1]?.label, plain[0]?.channels[1]?.roleLabel]).toEqual([
      'SCL',
      undefined,
    ])
  })

  it('puts every channel in the unnamed group without decoders', () => {
    const plain = groupChannels(channels, null)
    expect(plain).toHaveLength(1)
    expect(plain[0]?.channels).toHaveLength(16)
    expect(plain[0]?.channels[5]?.label).toBe('UART')
  })

  it('formats the UART line settings', () => {
    expect(uartFormat(groups[1]?.options ?? {})).toBe('115200 8N1')
    expect(uartFormat({})).toBeNull()
  })
})

describe('layoutRows', () => {
  const layout = layoutRows(groups, COMFORTABLE_METRICS, collapsedByDefault)
  const kinds = (key: string) =>
    layout.rows.filter((row) => row.group.key === key).map((row) => row.kind)

  it('follows the design row structure', () => {
    expect(kinds('1:i2c')).toEqual(['gap', 'header', 'lane', 'trace', 'trace', 'gap'])
    expect(kinds('1:uart')).toEqual(['gap', 'header', 'trace', 'gap'])
    expect(kinds('1:spi')).toEqual([
      'gap',
      'header',
      'lane',
      'trace-narrow',
      'lane',
      'trace-narrow',
      'trace-narrow',
      'trace-narrow',
      'gap',
    ])
    // CAN-FD starts collapsed.
    expect(kinds('can-fd')).toEqual(['gap', 'header', 'gap'])
  })

  it('stacks rows without gaps or overlaps', () => {
    let top = 0
    for (const row of layout.rows) {
      expect(row.top).toBe(top)
      top += row.height
    }
    expect(layout.height).toBe(top)
    // I2C: 8 + 26 + 26 + 30 + 30 + 8 = 128.
    expect(layout.rows.find((row) => row.key === '1:uart:before')?.top).toBe(128)
    expect(layout.rows.find((row) => row.key === '1:uart:before')?.separator).toBe(true)
  })

  it('finds the rows in view', () => {
    const shown = visibleRows(layout.rows, 0, 40)
    expect(shown.map((row) => row.kind)).toEqual(['gap', 'header', 'lane'])
  })
})
