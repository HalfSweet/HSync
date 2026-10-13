import { describe, expect, it } from 'vitest'

import {
  defaultCaptureName,
  isWaveformPath,
  suggestedCapturePath,
  withExtension,
} from './useFileActions'

describe('file names', () => {
  it('stamps the date and time', () => {
    expect(defaultCaptureName(new Date(2026, 9, 10, 14, 5, 9))).toBe('HSync-20261010-140509.hsync')
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
    expect(isWaveformPath('/tmp/capture.hsync')).toBe(false)
    expect(isWaveformPath('/tmp/capture.dsl')).toBe(false)
    expect(isWaveformPath('/tmp/vcd')).toBe(false)
  })

  it('suggests an .hsync name for the file a capture came from', () => {
    expect(suggestedCapturePath('/tmp/sim.vcd')).toBe('/tmp/sim.hsync')
    expect(suggestedCapturePath('/tmp/ila.fst')).toBe('/tmp/ila.hsync')
    expect(suggestedCapturePath('/tmp/capture.dsl')).toBe('/tmp/capture.hsync')
    expect(suggestedCapturePath('/tmp/capture.hsync')).toBe('/tmp/capture.hsync')
  })
})
