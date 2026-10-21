import { describe, expect, it } from 'vitest'

import {
  fieldTexts,
  payloadFields,
  isQuantity,
  primaryField,
  primaryFields,
  valueText,
  type Wording,
} from './present'

const wording: Wording = {
  enumValue: (field, value) => `${field}=${value}`,
  yes: 'yes',
  no: 'no',
  none: '—',
}

describe('generic presentation', () => {
  it('lists the fields of a payload without its kind', () => {
    expect(payloadFields({ kind: 'frame', value: 65, parity: null })).toEqual([
      ['value', 65],
      ['parity', null],
    ])
  })

  it('picks the field a block shows', () => {
    expect(primaryField({ kind: 'frame', value: 65, framingOk: true })).toEqual(['value', 65])
    expect(primaryField({ kind: 'data', tx: 1, byte: 4, ack: true })).toEqual(['byte', 4])
    expect(primaryField({ kind: 'begin', tx: 3, partial: false })).toEqual(['tx', 3])
    expect(primaryField({ kind: 'end', reason: 'stop' })).toEqual(['reason', 'stop'])
    expect(primaryField({ kind: 'data', bytes: [1, 2] })).toEqual(['bytes', [1, 2]])
    expect(primaryField({ kind: 'ack', ok: true })).toBeNull()
    // Both lines of an SPI word, but one value at most of a counter or a string.
    expect(primaryFields({ kind: 'word', tx: 1, mosi: 17, miso: 48 })).toEqual([
      ['mosi', 17],
      ['miso', 48],
    ])
    expect(primaryFields({ kind: 'crc', frame: 1, value: 7, ok: true, stuffCount: 4 }, 1)).toEqual([
      ['value', 7],
    ])
    expect(primaryFields({ kind: 'begin', tx: 3, partial: false })).toEqual([['tx', 3]])
    expect(primaryFields({ kind: 'end', reason: 'stop' })).toEqual([['reason', 'stop']])
    expect(primaryFields({ kind: 'ack', ok: true })).toEqual([])
  })

  it('shows JTAG shift data before the scan, offset and bit count', () => {
    const shift = { kind: 'shift', scan: 2, offset: 256, tdi: [15, 1], tdo: [18, 0], bits: 9 }
    expect(primaryFields(shift)).toEqual([
      ['tdi', [15, 1]],
      ['tdo', [18, 0]],
    ])
    expect(primaryField(shift)).toEqual(['tdi', [15, 1]])
    expect(primaryFields({ ...shift, tdi: null })).toEqual([['tdo', [18, 0]]])
    expect(primaryField({ ...shift, tdi: null })).toEqual(['tdo', [18, 0]])
    expect(primaryFields({ ...shift, tdo: null })).toEqual([['tdi', [15, 1]]])
  })

  it('shows USB data before its byte offset', () => {
    const data = { kind: 'data', pkt: 2, offset: 32, bytes: [1, 2] }
    expect(primaryFields(data)).toEqual([['bytes', [1, 2]]])
    expect(primaryField(data)).toEqual(['bytes', [1, 2]])
  })

  it('keeps measurements when an event carries no bus data', () => {
    const cycle = { kind: 'cycle', periodTicks: 100, activeTicks: 25 }
    expect(primaryFields(cycle)).toEqual([
      ['periodTicks', 100],
      ['activeTicks', 25],
    ])
    expect(primaryField(cycle)).toEqual(['periodTicks', 100])
    const edge = { kind: 'edge', slope: 'rising', sincePrevious: 250 }
    expect(primaryFields(edge)).toEqual([['sincePrevious', 250]])
    expect(primaryField(edge)).toEqual(['sincePrevious', 250])
  })

  it.each<[Record<string, unknown>, [string, unknown]]>([
    [{ kind: 'scan-begin', scan: 2, register: 'dr' }, ['register', 'dr']],
    [{ kind: 'end', tx: 3, reason: 'stop' }, ['reason', 'stop']],
    [{ kind: 'begin', scan: 2 }, ['scan', 2]],
    [{ kind: 'begin', tx: 3, partial: false }, ['tx', 3]],
    [{ kind: 'edge', slope: 'falling', sincePrevious: null }, ['slope', 'falling']],
  ])('uses the same fallback for one or multiple fields: %j', (payload, expected) => {
    expect(primaryFields(payload)).toEqual([expected])
    expect(primaryField(payload)).toEqual(expected)
  })

  it('shows values in the number format and the words given', () => {
    expect(valueText('value', 0x41, 'hex', wording)).toBe('41')
    expect(valueText('value', 0x41, 'ascii', wording)).toBe('A')
    expect(valueText('bytes', [1, 255], 'hex', wording)).toBe('01 FF')
    expect(valueText('reason', 'stop', 'hex', wording)).toBe('reason=stop')
    expect(valueText('ack', true, 'hex', wording)).toBe('yes')
    expect(valueText('ack', false, 'hex', wording)).toBe('no')
    expect(valueText('parity', null, 'hex', wording)).toBe('—')
    expect(valueText('nested', { a: 1 }, 'hex', wording)).toBe('{"a":1}')
  })

  it('texts every field in order', () => {
    expect(
      fieldTexts({ kind: 'frame', value: 65, parity: null, framingOk: true }, 'dec', wording),
    ).toEqual([
      { name: 'value', text: '65' },
      { name: 'parity', text: '—' },
      { name: 'framingOk', text: 'yes' },
    ])
  })

  it('reads counts and measures in decimal whatever the format', () => {
    expect(isQuantity('tx')).toBe(true)
    expect(isQuantity('scan')).toBe(true)
    expect(isQuantity('periodTicks')).toBe(true)
    expect(isQuantity('droppedBits')).toBe(true)
    expect(isQuantity('value')).toBe(false)
    expect(isQuantity('address')).toBe(false)
    expect(valueText('tx', 12, 'hex', wording)).toBe('12')
    expect(valueText('scan', 12, 'hex', wording)).toBe('12')
    expect(valueText('length', 64, 'bin', wording)).toBe('64')
    expect(valueText('address', 0x50, 'dec', wording)).toBe('80')
    expect(fieldTexts({ kind: 'word', tx: 10, mosi: 10 }, 'hex', wording)).toEqual([
      { name: 'tx', text: '10' },
      { name: 'mosi', text: '0A' },
    ])
    expect(primaryFields({ kind: 'word', tx: 1, mosi: 17, miso: 48 })).toEqual([
      ['mosi', 17],
      ['miso', 48],
    ])
    expect(primaryFields({ kind: 'begin', tx: 3, partial: false })).toEqual([['tx', 3]])
    expect(primaryFields({ kind: 'ack', ok: true })).toEqual([])
  })
})
