import { describe, expect, it } from 'vitest'

import { CELL_EMPTY, CELL_FIRST_HIGH, CELL_LAST_HIGH, CELL_TRANSITION } from '@/ipc/bins'

import { traceShape } from './shape'

const HIGH = CELL_FIRST_HIGH | CELL_LAST_HIGH
const x = (column: number) => column * 10

describe('traceShape', () => {
  it('merges equal columns into one segment', () => {
    const shape = traceShape(Uint8Array.of(0, 0, 0), x)
    expect(shape.path).toEqual([
      ['M', 0, false],
      ['L', 30, false],
    ])
    expect(shape.washes).toEqual([])
  })

  it('steps at column boundaries and washes high runs', () => {
    const shape = traceShape(Uint8Array.of(0, HIGH, HIGH, 0), x)
    expect(shape.path).toEqual([
      ['M', 0, false],
      ['L', 10, false],
      ['L', 10, true],
      ['L', 30, true],
      ['L', 30, false],
      ['L', 40, false],
    ])
    expect(shape.washes).toEqual([[10, 30]])
  })

  it('draws a full swing for transitions inside a column', () => {
    const shape = traceShape(Uint8Array.of(CELL_TRANSITION | CELL_LAST_HIGH, HIGH), x)
    expect(shape.path).toEqual([
      ['M', 0, false],
      ['L', 0, false],
      ['L', 0, true],
      ['M', 0, true],
      ['L', 20, true],
    ])
    expect(shape.washes).toEqual([[0, 20]])
  })

  it('stops at empty columns', () => {
    const shape = traceShape(Uint8Array.of(HIGH, CELL_EMPTY, CELL_EMPTY), x)
    expect(shape.path.at(-1)).toEqual(['L', 10, true])
    expect(shape.washes).toEqual([[0, 10]])
    expect(traceShape(Uint8Array.of(CELL_EMPTY), x).path).toEqual([])
  })
})
