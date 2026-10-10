import { describe, expect, it } from 'vitest'

import {
  digitsFor,
  formatBytes,
  formatCount,
  formatFrequency,
  formatFrequencyShort,
  formatSamplerate,
  formatSpan,
  formatTime,
  formatTimeShort,
  niceStep,
  timeUnitFor,
} from './units'

describe('time formatting', () => {
  it('picks the unit by magnitude', () => {
    expect(timeUnitFor(5e-9)).toBe('ns')
    expect(timeUnitFor(9.8e-6)).toBe('µs')
    expect(timeUnitFor(5.24e-3)).toBe('ms')
    expect(timeUnitFor(2)).toBe('s')
    expect(timeUnitFor(-3e-6)).toBe('µs')
  })

  it('formats like the design', () => {
    expect(formatTime(9.8e-6)).toBe('9.8 µs')
    expect(formatTimeShort(5.24288e-3)).toBe('5.24 ms')
    expect(formatTimeShort(50e-6)).toBe('50 µs')
    expect(formatSpan(540e-6)).toBe('540.0 µs')
    expect(formatSpan(1.046e-3)).toBe('1.0460 ms')
    expect(formatTime(-0.00001e-6, 1)).toBe('0.0 ns')
  })
})

describe('frequency formatting', () => {
  it('formats frequencies and sample rates', () => {
    expect(formatFrequency(100_000)).toBe('100.0 kHz')
    expect(formatSamplerate(25_000_000)).toBe('25 MHz')
    expect(formatSamplerate(1_250_000)).toBe('1.25 MHz')
    expect(formatSamplerate(1_000_000_000)).toBe('1 GHz')
    expect(formatSamplerate(500)).toBe('500 Hz')
    expect(formatFrequencyShort(25e6 / 251)).toBe('99.6 kHz')
    expect(formatFrequencyShort(25e6 / 19)).toBe('1.32 MHz')
  })
})

describe('count formatting', () => {
  it('groups Chinese counts with narrow spaces', () => {
    expect(formatCount(131_072, 'zh-CN')).toBe('131 072')
    expect(formatCount(999, 'zh-CN')).toBe('999')
  })

  it('uses Intl grouping for English', () => {
    expect(formatCount(131_072, 'en-US')).toBe('131,072')
  })
})

describe('steps', () => {
  it('rounds up to the 1-2-5 sequence', () => {
    expect(niceStep(0.7)).toBe(1)
    expect(niceStep(1)).toBe(1)
    expect(niceStep(1.2)).toBe(2)
    expect(niceStep(3)).toBe(5)
    expect(niceStep(42e-6)).toBeCloseTo(50e-6)
    expect(niceStep(0)).toBe(1)
  })

  it('counts the digits multiples of a step need', () => {
    expect(digitsFor(0.05)).toBe(2)
    expect(digitsFor(0.1)).toBe(1)
    expect(digitsFor(0.2)).toBe(1)
    expect(digitsFor(1)).toBe(0)
    expect(digitsFor(50)).toBe(0)
  })
})

describe('size formatting', () => {
  it('uses binary units', () => {
    expect(formatBytes(512)).toBe('512 B')
    expect(formatBytes(1536)).toBe('1.5 KiB')
    expect(formatBytes(16 * 2 ** 30)).toBe('16 GiB')
    expect(formatBytes(1.25 * 2 ** 40)).toBe('1.25 TiB')
  })
})
