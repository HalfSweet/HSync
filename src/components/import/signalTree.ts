import type { WaveformItem } from '@/ipc/bindings/WaveformItem'

/**
 * The signal list of the import dialog: a waveform file's scopes and signals as the backend
 * lists them, in the file's order with a depth each, shown as a tree that folds and filters.
 */

/** Files with no more bits than this have every signal chosen from the start. */
export const PRESELECT_LIMIT = 32

/** An item of the file in the list, with where it stands. */
export interface SignalRow {
  /** Position in the file's items. */
  index: number
  item: WaveformItem
}

/** For every item, the index of the first item after it and, for a scope, its contents. */
export function subtreeEnds(items: readonly WaveformItem[]): number[] {
  const ends = new Array<number>(items.length)
  const open: number[] = []
  for (let index = 0; index < items.length; index++) {
    const depth = items[index]!.depth
    while (open.length > 0 && items[open[open.length - 1]!]!.depth >= depth) {
      ends[open.pop()!] = index
    }
    open.push(index)
  }
  for (const index of open) ends[index] = items.length
  return ends
}

/** The logic signals item `index` stands for: itself, or those inside a scope. */
export function signalsWithin(
  items: readonly WaveformItem[],
  ends: readonly number[],
  index: number,
): number[] {
  const found: number[] = []
  for (let at = index; at < (ends[index] ?? index + 1); at++) {
    const signal = items[at]?.signal
    if (signal != null) found.push(signal)
  }
  return found
}

/** What a scope's checkbox shows: all, some or none of its signals chosen. */
export function scopeState(
  items: readonly WaveformItem[],
  ends: readonly number[],
  index: number,
  chosen: ReadonlySet<number>,
): 'all' | 'some' | 'none' {
  const signals = signalsWithin(items, ends, index)
  const count = signals.filter((signal) => chosen.has(signal)).length
  if (count === 0) return 'none'
  return count === signals.length ? 'all' : 'some'
}

/**
 * The rows shown: every item outside a folded scope, or with a query, the items whose names
 * contain it along with the scopes they lie in.
 */
export function visibleRows(
  items: readonly WaveformItem[],
  ends: readonly number[],
  collapsed: ReadonlySet<number>,
  query: string,
): SignalRow[] {
  const rows: SignalRow[] = []
  const needle = query.trim().toLowerCase()
  if (needle === '') {
    let hiddenUntil = 0
    for (let index = 0; index < items.length; index++) {
      if (index < hiddenUntil) continue
      rows.push({ index, item: items[index]! })
      if (collapsed.has(index)) hiddenUntil = ends[index] ?? index + 1
    }
    return rows
  }
  const shown = new Array<boolean>(items.length).fill(false)
  const open: number[] = []
  for (let index = 0; index < items.length; index++) {
    const item = items[index]!
    while (open.length > 0 && items[open[open.length - 1]!]!.depth >= item.depth) open.pop()
    if (item.name.toLowerCase().includes(needle)) {
      shown[index] = true
      for (const ancestor of open) shown[ancestor] = true
    }
    if (item.kind === 'scope') open.push(index)
  }
  for (let index = 0; index < items.length; index++) {
    if (shown[index]) rows.push({ index, item: items[index]! })
  }
  return rows
}

/** Channels the chosen signals make: one per bit. */
export function channelCount(items: readonly WaveformItem[], chosen: ReadonlySet<number>): number {
  let count = 0
  for (const item of items) if (item.signal != null && chosen.has(item.signal)) count += item.width
  return count
}

/** The chosen signals in the file's order, which is the channel order. */
export function chosenInOrder(
  items: readonly WaveformItem[],
  chosen: ReadonlySet<number>,
): number[] {
  const order: number[] = []
  for (const item of items)
    if (item.signal != null && chosen.has(item.signal)) order.push(item.signal)
  return order
}

/** Every logic signal of the file. */
export function allSignals(items: readonly WaveformItem[]): Set<number> {
  const all = new Set<number>()
  for (const item of items) if (item.signal != null) all.add(item.signal)
  return all
}

/**
 * The choice to start from: every signal when each bit can be a channel without the capture
 * growing past what a logic analyzer shows, otherwise none.
 */
export function defaultChoice(items: readonly WaveformItem[]): Set<number> {
  const all = allSignals(items)
  return channelCount(items, all) <= PRESELECT_LIMIT ? all : new Set()
}
