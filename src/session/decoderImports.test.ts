import { describe, expect, it } from 'vitest'

import { decoderImportWarnings } from './decoderImports'

const raw = { id: 'unknown', options: { opaque: [1, 2] }, 'stacked decoders': [{ id: 'child' }] }
const retained = {
  nodes: {},
  label: 'Original label',
  import: {
    format: 'dsview',
    raw,
    runnable: false,
    diagnostics: [{ code: 'decode.import.unsupportedDecoder', params: { decoder: 'unknown' } }],
  },
}

describe('retained decoder imports', () => {
  it('keeps original list indices and exposes localized diagnostics without changing raw data', () => {
    const entries = [{ nodes: { spi: { type: 'spi' } } }, retained]
    const before = JSON.stringify(entries)
    expect(decoderImportWarnings(entries)).toEqual([
      {
        index: 1,
        label: 'Original label',
        runnable: false,
        diagnostics: [{ code: 'decode.import.unsupportedDecoder', params: { decoder: 'unknown' } }],
      },
    ])
    expect(JSON.stringify(entries)).toBe(before)
    expect(JSON.parse(JSON.stringify(entries))[1].import.raw).toEqual(raw)
  })
  it('includes partial mappings and ignores malformed metadata', () => {
    expect(decoderImportWarnings([null, {}, { import: [] }])).toEqual([])
    expect(
      decoderImportWarnings([{ ...retained, import: { ...retained.import, runnable: true } }])[0]
        ?.runnable,
    ).toBe(true)
  })
})
