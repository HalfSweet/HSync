import { describe, expect, it } from 'vitest'

import { CELL_EMPTY, CELL_FIRST_HIGH, CELL_LAST_HIGH, CELL_TRANSITION } from '@/ipc/bins'

import { busyRuns, changingIn, traceShape } from './shape'

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

  it('draws a dense burst as an outlined block, the line resuming on its far side', () => {
    const T = CELL_TRANSITION
    // Low, a burst of three busy columns ending high, then high.
    const cells = Uint8Array.of(0, T, T | CELL_LAST_HIGH, HIGH | T, HIGH)
    const shape = traceShape(cells, x, [[1, 4]])
    expect(shape.blocks).toEqual([[10, 40]])
    expect(shape.path).toEqual([
      ['M', 0, false],
      ['L', 10, false],
      // The block's outline: left end, top, bottom, right end.
      ['L', 10, true],
      ['M', 10, true],
      ['L', 40, true],
      ['M', 10, false],
      ['L', 40, false],
      ['M', 40, true],
      ['L', 40, false],
      ['M', 40, true],
      ['L', 50, true],
    ])
    expect(shape.washes).toEqual([[40, 50]])
  })

  it('leaves the wash off when asked, as for a busy line', () => {
    const T = CELL_TRANSITION
    // High, a burst, high, low, a high pulse, low.
    const cells = Uint8Array.of(HIGH, HIGH, T | HIGH, T | HIGH, HIGH, 0, HIGH, HIGH, 0)
    expect(traceShape(cells, x, [[2, 4]]).washes).toEqual([
      [0, 20],
      [40, 50],
      [60, 80],
    ])
    const busy = traceShape(cells, x, [[2, 4]], false)
    expect(busy.washes).toEqual([])
    expect(busy.blocks).toEqual([[20, 40]])
  })
})

describe('dense runs', () => {
  const T = CELL_TRANSITION

  it('gather busy columns, bridging single quiet ones', () => {
    expect(busyRuns(Uint8Array.of(0, T, T, 0, T, 0, 0, T, 0))).toEqual([[1, 5]])
    // A lone edge stays an edge; two quiet columns end a run.
    expect(busyRuns(Uint8Array.of(T, 0, 0, T, T, 0))).toEqual([[3, 5]])
    expect(busyRuns(Uint8Array.of(T, T, CELL_EMPTY, T, T))).toEqual([[0, 2]])
  })

  it("keep a clock's runs where a data line changes, between columns too", () => {
    const runs = [
      [0, 3],
      [5, 8],
    ] as const
    // Changes at a column boundary in the first run, none in the second.
    const data = Uint8Array.of(0, HIGH, HIGH, 0, 0, HIGH, HIGH, HIGH)
    expect(changingIn(data, runs)).toEqual([[0, 3]])
  })
})
