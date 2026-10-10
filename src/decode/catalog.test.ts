import { describe, expect, it } from 'vitest'

import { buildEntry, decoderSpec, entryChannels } from './catalog'

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
