import { describe, expect, it } from 'vitest'

import {
  buildEntry,
  decoderSpec,
  entryChannels,
  entryStack,
  specFromInfo,
  stackFor,
} from './catalog'

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

describe('decoder stacks', () => {
  const i2c = { ...specFromInfoLike('1:i2c', ['logic'], ['i2c']) }
  const lightI2c = { ...specFromInfoLike('0:i2c', ['logic'], ['i2c']) }
  const eeprom = { ...specFromInfoLike('eeprom24xx', ['i2c'], []) }
  const lonely = { ...specFromInfoLike('lonely', ['nothing'], []) }

  function specFromInfoLike(id: string, inputs: string[], outputs: string[]) {
    return {
      id,
      label: id,
      bus: 'other' as const,
      roles: [],
      options: [{ id: 'level', kind: 'number' as const, default: 1, step: 1 }],
      inputs,
      outputs,
    }
  }

  it('sits a decoder on the one it reads, skipping light variants', () => {
    const catalog = [lightI2c, i2c, eeprom, lonely]
    expect(stackFor(eeprom, catalog)?.map((spec) => spec.id)).toEqual(['1:i2c', 'eeprom24xx'])
    expect(stackFor(i2c, catalog)?.map((spec) => spec.id)).toEqual(['1:i2c'])
    expect(stackFor(lonely, catalog)).toBeNull()
  })

  it('stores the stacked decoders with their options', () => {
    const entry = buildEntry(i2c, {}, {}, [{ spec: eeprom, options: { level: 3 } }])
    expect(entry['stacked decoders']).toEqual([{ id: 'eeprom24xx', options: { level: 3 } }])
    expect(entryStack(entry)).toEqual([{ id: 'eeprom24xx', options: { level: 3 } }])
  })
})
