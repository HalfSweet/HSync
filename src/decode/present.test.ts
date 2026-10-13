import { describe, expect, it } from 'vitest'

import { fieldTexts, payloadFields, primaryField, valueText, type Wording } from './present'

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
})
