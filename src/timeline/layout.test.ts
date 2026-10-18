import { describe, expect, it } from 'vitest'

import fixture from '@/ipc/mock/fixtures/protocol-demo.json'
import decoded from '@/ipc/mock/fixtures/protocol-uiharu.json'

import { groupChannels, uartFormat } from './groups'
import {
  clampTraceHeight,
  collapsedByDefault,
  COMFORTABLE_METRICS,
  fitRows,
  layoutRows,
  MAX_TRACE_HEIGHT,
  type Row,
  visibleRows,
} from './layout'

const channels = fixture.channels.map(({ index, name }) => ({ index, name }))
const groups = groupChannels(channels, decoded.entries)

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
    expect(groups.map((group) => group.type)).toEqual(['i2c', 'uart', 'spi', 'can', null])
    expect(groups.map((group) => group.key)).toEqual([
      'decoder:0',
      'decoder:1',
      'decoder:2',
      'decoder:3',
      'unnamed',
    ])
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
      decoded.entries,
    )
    const scl = renamed[0]?.channels[1]
    expect([scl?.label, scl?.roleLabel]).toEqual(['Clock', 'SCL'])
    // Back to its number, it shows its role again.
    const plain = groupChannels(
      channels.map((channel) => (channel.index === 1 ? { ...channel, name: '1' } : channel)),
      decoded.entries,
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

  it('names the lines of a channel list after their position', () => {
    const parallel = groupChannels(channels, [
      { nodes: { bus: { type: 'parallel', config: { data: [2, 3, 4], clock: 6 } } } },
    ])
    expect(parallel[0]?.channels.map((channel) => [channel.role, channel.label])).toEqual([
      ['data0', 'DATA0'],
      ['data1', 'DATA1'],
      ['data2', 'DATA2'],
      ['clock', 'CLOCK'],
    ])
    expect(parallel[0]?.title).toBe('')
    expect(parallel[0]?.type).toBe('parallel')
  })

  it('formats the UART line settings', () => {
    expect(uartFormat(groups[1]?.options ?? {})).toBe('115200 8N1')
    expect(uartFormat({ baud: 9600, data_bits: 7, parity: 'even', stop_bits: 'two' })).toBe(
      '9600 7E2',
    )
    expect(uartFormat({})).toBeNull()
  })
})

describe('layoutRows', () => {
  // The I²C decoder carries a stacked EEPROM: two streams, so a lane row of its own.
  const streamsOf = (group: { type: string | null }) => (group.type === 'i2c' ? 2 : 1)
  const layout = layoutRows(groups, COMFORTABLE_METRICS, collapsedByDefault, streamsOf)
  const kinds = (key: string) =>
    layout.rows.filter((row) => row.group.key === key).map((row) => row.kind)

  it('follows the design row structure', () => {
    expect(kinds('decoder:0')).toEqual(['gap', 'header', 'lane', 'trace', 'trace', 'gap'])
    expect(layout.rows.find((row) => row.kind === 'lane')?.stream).toBe(1)
    expect(kinds('decoder:1')).toEqual(['gap', 'header', 'trace', 'gap'])
    expect(kinds('decoder:2')).toEqual([
      'gap',
      'header',
      'trace-narrow',
      'trace-narrow',
      'trace-narrow',
      'trace-narrow',
      'gap',
    ])
    // CAN-FD starts collapsed.
    expect(kinds('decoder:3')).toEqual(['gap', 'header', 'gap'])
  })

  it('stacks rows without gaps or overlaps', () => {
    let top = 0
    for (const row of layout.rows) {
      expect(row.top).toBe(top)
      top += row.height
    }
    expect(layout.height).toBe(top)
    // I2C: 8 + 26 + 26 + 30 + 30 + 8 = 128.
    expect(layout.rows.find((row) => row.key === 'decoder:1:before')?.top).toBe(128)
    expect(layout.rows.find((row) => row.key === 'decoder:1:before')?.separator).toBe(true)
  })

  it('gives channels the heights the user dragged, within bounds', () => {
    const [sda, scl] = groups[0]?.channels ?? []
    const [miso, mosi, clk] = groups[2]?.channels ?? []
    const own = new Map([
      [sda?.index, 64.4],
      [miso?.index, 1_000],
      [mosi?.index, 3],
    ])
    const sized = layoutRows(groups, COMFORTABLE_METRICS, collapsedByDefault, streamsOf, (one) =>
      own.get(one.index),
    )
    const row = (index?: number) => sized.rows.find((one) => one.channel?.index === index)
    expect(row(sda?.index)?.height).toBe(64)
    expect(row(scl?.index)?.height).toBe(COMFORTABLE_METRICS.trace)
    expect(row(miso?.index)?.height).toBe(MAX_TRACE_HEIGHT)
    expect(row(mosi?.index)?.height).toBe(COMFORTABLE_METRICS.traceNarrow)
    expect(row(clk?.index)?.height).toBe(COMFORTABLE_METRICS.traceNarrow)
    // A row keeps its kind, and the rows below move down.
    expect(row(sda?.index)?.kind).toBe('trace')
    expect(row(scl?.index)?.top).toBe((row(sda?.index)?.top ?? 0) + 64)
    expect(sized.height).toBe(layout.height + (64 - 30) + (MAX_TRACE_HEIGHT - 18))
  })

  it('keeps the heights given to channels whole and within bounds', () => {
    expect(clampTraceHeight(41.6, COMFORTABLE_METRICS)).toBe(42)
    expect(clampTraceHeight(-20, COMFORTABLE_METRICS)).toBe(COMFORTABLE_METRICS.traceNarrow)
    expect(clampTraceHeight(Number.POSITIVE_INFINITY, COMFORTABLE_METRICS)).toBe(MAX_TRACE_HEIGHT)
  })

  it('finds the rows in view', () => {
    const shown = visibleRows(layout.rows, 0, 40)
    expect(shown.map((row) => row.kind)).toEqual(['gap', 'header', 'lane'])
  })
})

describe('fitRows', () => {
  const metrics = COMFORTABLE_METRICS
  const layout = layoutRows(groups, metrics, collapsedByDefault)
  const isChannel = (row: Row) => row.kind === 'trace' || row.kind === 'trace-narrow'
  // I²C SDA and SCL and the UART line are wide rows; SPI and the unnamed channels narrow ones.
  const channelHeight = layout.rows.filter(isChannel).reduce((sum, row) => sum + row.height, 0)
  const fixed = layout.height - channelHeight
  const heights = (fitted: { rows: Row[] }) => [
    ...new Set(fitted.rows.filter(isChannel).map((row) => `${row.kind} ${row.height}`)),
  ]
  const stacked = (fitted: { rows: Row[]; height: number }) => {
    let top = 0
    for (const row of fitted.rows) {
      expect(row.top).toBe(top)
      top += row.height
    }
    expect(fitted.height).toBe(top)
  }

  it('grows the channel rows to fill the room, keeping their proportions', () => {
    expect(channelHeight).toBe(3 * 30 + 12 * 18)
    const fitted = fitRows(layout, metrics, layout.height + channelHeight)
    expect(fitted.scale).toBeCloseTo(2)
    expect(heights(fitted)).toEqual(['trace 60', 'trace-narrow 36'])
    expect(fitted.height).toBe(layout.height + channelHeight)
    // Titles, lanes and gaps keep their heights.
    const others = (rows: Row[]) => rows.filter((row) => !isChannel(row)).map((row) => row.height)
    expect(others(fitted.rows)).toEqual(others(layout.rows))
    stacked(fitted)
  })

  it('grows them no more than three times, leaving the rest of the room below', () => {
    const fitted = fitRows(layout, metrics, layout.height + 10_000)
    expect(fitted.scale).toBe(3)
    expect(heights(fitted)).toEqual(['trace 90', 'trace-narrow 54'])
    expect(fitted.height).toBe(fixed + 3 * channelHeight)
  })

  it('shrinks the wide rows towards the narrow height before the rows scroll', () => {
    // 3 wide rows of 20 px and 12 narrow ones of 18 px fill 30 px less.
    const tighter = fitRows(layout, metrics, layout.height - 30)
    expect(heights(tighter)).toEqual(['trace 20', 'trace-narrow 18'])
    expect(tighter.height).toBe(layout.height - 30)

    const scrolling = fitRows(layout, metrics, fixed + 100)
    expect(scrolling.scale).toBe(18 / 30)
    expect(heights(scrolling)).toEqual(['trace 18', 'trace-narrow 18'])
    expect(scrolling.height).toBe(fixed + 15 * 18)
  })

  it('scales the heights the user dragged with the others', () => {
    const sda = groups[0]?.channels[0]
    const sized = layoutRows(groups, metrics, collapsedByDefault, undefined, (one) =>
      one.index === sda?.index ? 60 : undefined,
    )
    const fitted = fitRows(sized, metrics, sized.height + channelHeight + 30)
    expect(fitted.rows.find((row) => row.channel?.index === sda?.index)?.height).toBe(120)
  })

  it('fills the room in whole pixels', () => {
    for (const extra of [1, 7, 101, 333]) {
      const fitted = fitRows(layout, metrics, layout.height + extra)
      expect(fitted.rows.every((row) => Number.isInteger(row.height))).toBe(true)
      expect(fitted.height).toBe(layout.height + extra)
      stacked(fitted)
    }
  })

  it('keeps a dragged row at the height the pointer gives it, the others fitting around it', () => {
    const sda = groups[0]?.channels[0]
    const pinned = { index: sda?.index ?? 0, height: 100 }
    const room = layout.height + channelHeight
    const fitted = fitRows(layout, metrics, room, pinned)
    const sdaRow = (rows: Row[]) => rows.find((row) => row.channel?.index === sda?.index)
    expect(sdaRow(fitted.rows)?.height).toBe(100)
    expect(fitted.height).toBe(room)
    // The others share what is left: 2 × 306 − 100 px over 276 px of their own heights.
    expect(fitted.scale).toBeCloseTo((2 * channelHeight - 100) / (channelHeight - 30))

    // Kept as its height before scaling, the row stays where the drag left it.
    const kept = layoutRows(groups, metrics, collapsedByDefault, undefined, (one) =>
      one.index === sda?.index ? clampTraceHeight(100 / fitted.scale, metrics) : undefined,
    )
    const after = sdaRow(fitRows(kept, metrics, room).rows)?.height ?? 0
    expect(Math.abs(after - 100)).toBeLessThanOrEqual(1)
  })

  it('leaves rows without channels as they are', () => {
    const closed = layoutRows(groups, metrics, () => true)
    const fitted = fitRows(closed, metrics, closed.height + 500)
    expect(fitted.rows.map((row) => row.height)).toEqual(closed.rows.map((row) => row.height))
    expect(fitted.scale).toBe(1)
  })
})
