import { describe, expect, it } from 'vitest'

import { csvField, csvLines } from './csv'

describe('csv', () => {
  it('quotes only what needs it', () => {
    expect(csvField('4F')).toBe('4F')
    expect(csvField('a, b')).toBe('"a, b"')
    expect(csvField('say "hi"')).toBe('"say ""hi"""')
    expect(
      csvLines([
        ['time', 'text'],
        ['1.0', 'x,y'],
      ]),
    ).toBe('time,text\r\n1.0,"x,y"\r\n')
  })
})
