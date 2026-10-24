import { describe, expect, it } from 'vitest'

import { rowParts } from './rows'

const labels: Record<string, string> = {
  tx: 'Transaction',
  scan: 'Scan',
  address: 'Address',
  read: 'Read',
  state: 'State',
}
const labelOf = (name: string) => labels[name] ?? name

describe('data stream rows', () => {
  it('sets the transfer number apart and labels the other fields', () => {
    const parts = rowParts(
      [
        { name: 'tx', text: '1' },
        { name: 'address', text: '62' },
        { name: 'read', text: 'no' },
      ],
      'Data',
      labelOf,
    )
    expect(parts).toEqual({
      counter: '1',
      cells: [
        { label: 'Address', text: '62' },
        { label: 'Read', text: 'no' },
      ],
    })
  })

  it('leaves out a label that repeats the kind, whatever its case', () => {
    expect(rowParts([{ name: 'address', text: '62' }], 'address', labelOf).cells).toEqual([
      { label: null, text: '62' },
    ])
    expect(rowParts([{ name: 'state', text: 'Run-Test/Idle' }], 'State', labelOf)).toEqual({
      counter: null,
      cells: [{ label: null, text: 'Run-Test/Idle' }],
    })
  })

  it('takes only the first transfer number apart', () => {
    const parts = rowParts(
      [
        { name: 'scan', text: '2' },
        { name: 'tx', text: '7' },
      ],
      'Shift',
      labelOf,
    )
    expect(parts.counter).toBe('2')
    expect(parts.cells).toEqual([{ label: 'Transaction', text: '7' }])
  })
})
