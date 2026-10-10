import { describe, expect, it } from 'vitest'

import type { WaveformItem } from '@/ipc/bindings/WaveformItem'

import {
  channelCount,
  chosenInOrder,
  defaultChoice,
  scopeState,
  signalsWithin,
  subtreeEnds,
  visibleRows,
} from './signalTree'

const scope = (depth: number, name: string): WaveformItem => ({
  depth,
  name,
  kind: 'scope',
  signal: null,
  width: 0,
  range: null,
})
const logic = (depth: number, name: string, signal: number, width = 1): WaveformItem => ({
  depth,
  name,
  kind: 'logic',
  signal,
  width,
  range: width > 1 ? [width - 1, 0] : null,
})
const other = (depth: number, name: string): WaveformItem => ({
  depth,
  name,
  kind: 'other',
  signal: null,
  width: 0,
  range: null,
})

// top { clk, data[3:0], uart { tx, temperature }, cpu { rst } } clk2
const items: WaveformItem[] = [
  scope(0, 'top'),
  logic(1, 'clk', 0),
  logic(1, 'data', 1, 4),
  scope(1, 'uart'),
  logic(2, 'tx', 2),
  other(2, 'temperature'),
  scope(1, 'cpu'),
  logic(2, 'rst', 3),
  logic(0, 'clk2', 4),
]
const ends = subtreeEnds(items)

describe('subtreeEnds', () => {
  it('finds where every scope ends', () => {
    expect(ends).toEqual([8, 2, 3, 6, 5, 6, 8, 8, 9])
  })
})

describe('signalsWithin', () => {
  it('lists a scope’s signals and a signal itself', () => {
    expect(signalsWithin(items, ends, 0)).toEqual([0, 1, 2, 3])
    expect(signalsWithin(items, ends, 3)).toEqual([2])
    expect(signalsWithin(items, ends, 2)).toEqual([1])
    expect(signalsWithin(items, ends, 5)).toEqual([])
  })
})

describe('scopeState', () => {
  it('tells all, some and none', () => {
    expect(scopeState(items, ends, 0, new Set([0, 1, 2, 3]))).toBe('all')
    expect(scopeState(items, ends, 0, new Set([2]))).toBe('some')
    expect(scopeState(items, ends, 3, new Set([0]))).toBe('none')
    expect(scopeState(items, ends, 3, new Set([2]))).toBe('all')
  })
})

describe('visibleRows', () => {
  it('shows everything when nothing is folded', () => {
    expect(visibleRows(items, ends, new Set(), '').map((row) => row.index)).toEqual([
      0, 1, 2, 3, 4, 5, 6, 7, 8,
    ])
  })

  it('hides the contents of folded scopes', () => {
    expect(visibleRows(items, ends, new Set([3]), '').map((row) => row.index)).toEqual([
      0, 1, 2, 3, 6, 7, 8,
    ])
    expect(visibleRows(items, ends, new Set([0]), '').map((row) => row.index)).toEqual([0, 8])
  })

  it('searches names, keeping the scopes a match lies in', () => {
    expect(visibleRows(items, ends, new Set([0]), 'TX').map((row) => row.index)).toEqual([0, 3, 4])
    expect(visibleRows(items, ends, new Set(), 'clk').map((row) => row.index)).toEqual([0, 1, 8])
    expect(visibleRows(items, ends, new Set(), 'nothing')).toEqual([])
  })
})

describe('choices', () => {
  it('counts a channel per bit, in the file’s order', () => {
    const chosen = new Set([4, 1, 3])
    expect(channelCount(items, chosen)).toBe(6)
    expect(chosenInOrder(items, chosen)).toEqual([1, 3, 4])
  })

  it('starts with every signal of a small file and none of a large one', () => {
    expect([...defaultChoice(items)].sort()).toEqual([0, 1, 2, 3, 4])
    const wide = [scope(0, 'top'), logic(1, 'bus', 0, 40)]
    expect(defaultChoice(wide).size).toBe(0)
  })
})
