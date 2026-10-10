import { describe, expect, it } from 'vitest'

import {
  defaultCaptureName,
  isWaveformPath,
  suggestedDslPath,
  withExtension,
} from './useFileActions'

describe('file names', () => {
  it('stamps the date and time', () => {
    expect(defaultCaptureName(new Date(2026, 9, 10, 14, 5, 9))).toBe('HSync-20261010-140509.dsl')
    expect(defaultCaptureName(new Date(2026, 9, 10, 14, 5, 9), 'vcd')).toBe(
      'HSync-20261010-140509.vcd',
    )
  })

  it('adds a missing extension', () => {
    expect(withExtension('/tmp/a', 'csv')).toBe('/tmp/a.csv')
    expect(withExtension('/tmp/a.CSV', 'csv')).toBe('/tmp/a.CSV')
  })

  it('tells waveform files from captures', () => {
    expect(isWaveformPath('/tmp/sim.vcd')).toBe(true)
    expect(isWaveformPath('C:\\dumps\\ila.FST')).toBe(true)
    expect(isWaveformPath('/tmp/tb.ghw')).toBe(true)
    expect(isWaveformPath('/tmp/capture.dsl')).toBe(false)
    expect(isWaveformPath('/tmp/vcd')).toBe(false)
  })

  it('suggests a .dsl name for an imported waveform', () => {
    expect(suggestedDslPath('/tmp/sim.vcd')).toBe('/tmp/sim.dsl')
    expect(suggestedDslPath('/tmp/ila.fst')).toBe('/tmp/ila.dsl')
    expect(suggestedDslPath('/tmp/capture.dsl')).toBe('/tmp/capture.dsl')
  })
})
