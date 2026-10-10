import { describe, expect, it } from 'vitest'

import { pulseMode, readPulse } from './pulse'

const view = { start: 1000, samplesPerPixel: 1, width: 1000 }
const RATE = 25_000_000

function pulse(start: number, end: number, nextEnd: number | null = null, edges = true) {
  return { high: true, start, end, startIsEdge: edges, endIsEdge: edges, nextEnd }
}

describe('readPulse', () => {
  it('measures a pulse inside the view', () => {
    const reading = readPulse(pulse(1100, 1345), view, RATE, 'data')
    expect(reading.left).toBe(100)
    expect(reading.width).toBe(245)
    expect(reading.complete).toBe(true)
    expect(reading.seconds).toBeCloseTo(9.8e-6)
    expect(reading.period).toBeNull()
  })

  it('reports clocks with similar neighbouring pulses', () => {
    const clock = readPulse(pulse(1100, 1225, 1350), view, RATE, 'clock')
    expect(clock.period).toBeCloseTo(10e-6)
    const irregular = readPulse(pulse(1100, 1225, 1600), view, RATE, 'clock')
    expect(irregular.period).toBeNull()
  })

  it('counts UART bits', () => {
    // 8.7 µs per bit at 115200 Bd; three bits at 25 MHz.
    const reading = readPulse(pulse(1100, 1754), view, RATE, 'bits', 1 / 115_200)
    expect(reading.bits).toBe(3)
  })

  it('clips pulses that leave the view and marks them incomplete', () => {
    const reading = readPulse(pulse(900, 1200), view, RATE, 'clock')
    expect(reading.left).toBe(0)
    expect(reading.width).toBe(200)
    expect(reading.complete).toBe(false)
    expect(reading.center).toBe(100)
    expect(readPulse(pulse(1100, 1200, null, false), view, RATE, 'data').complete).toBe(false)
  })

  it('keeps the read-out away from the edges', () => {
    expect(readPulse(pulse(1990, 2000), view, RATE, 'data').center).toBe(910)
  })
})

describe('pulseMode', () => {
  it('derives the mode from the decoder role', () => {
    expect(pulseMode({ index: 5, label: 'RX', numeric: false, wire: 5, role: 'rxtx' })).toBe('bits')
    expect(pulseMode({ index: 0, label: 'SDA', numeric: false, wire: 0, role: 'sda' })).toBe('data')
    expect(pulseMode({ index: 1, label: 'SCL', numeric: false, wire: 1, role: 'scl' })).toBe(
      'clock',
    )
    expect(pulseMode({ index: 2, label: 'D2', numeric: true, wire: 2 })).toBe('clock')
  })
})
