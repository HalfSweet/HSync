import { describe, expect, it } from 'vitest'

import { buildEntry, decoderSpec, entryChannels, specFromInfo } from './catalog'

describe('decoder catalog', () => {
  it('builds DSView entries', () => {
    const spec = decoderSpec('1:spi')
    expect(spec).toBeDefined()
    if (!spec) return
    const entry = buildEntry(spec, { clk: 12, miso: 15, mosi: null, cs: 13 }, { cpol: 1 })
    expect(entry.channel).toEqual([{ clk: 12 }, { miso: 15 }, { cs: 13 }])
    expect(entry.options).toMatchObject({ cpol: 1, cpha: 0, wordsize: 8 })
    expect(entryChannels(entry)).toEqual({ clk: 12, miso: 15, cs: 13 })
  })
})

describe('catalog entries', () => {
  it('turn into decoder specs', () => {
    const spec = specFromInfo({
      id: 'onewire_link',
      name: '1-Wire link layer',
      longname: '1-Wire serial communication bus (link layer)',
      desc: 'Bidirectional, half-duplex, asynchronous serial bus.',
      license: 'gplv2+',
      inputs: ['logic'],
      outputs: ['onewire_link'],
      tags: ['Embedded/industrial'],
      channels: [{ id: 'owr', name: 'OWR', desc: '1-Wire signal line' }],
      optionalChannels: [{ id: 'pwr', name: 'PWR', desc: '1-Wire power supply pin' }],
      options: [
        { id: 'overdrive', desc: 'Start in overdrive speed', default: 'no', values: ['yes', 'no'] },
        { id: 'cnt_normal_bit', desc: 'Normal mode sample bit time (μs)', default: 15, values: [] },
        { id: 'tag', desc: 'Tag', default: 'x', values: [] },
        { id: 'weird', desc: 'Weird', default: null, values: [] },
      ],
      annotations: [],
      annotationRows: [],
    })
    expect(spec.bus).toBe('other')
    expect(spec.roles).toEqual([
      { id: 'owr', name: 'OWR', required: true, desc: '1-Wire signal line' },
      { id: 'pwr', name: 'PWR', required: false, desc: '1-Wire power supply pin' },
    ])
    expect(spec.options.map((option) => option.kind)).toEqual(['choice', 'number', 'text'])
  })
})
