/**
 * The shape of one trace from its column cells: a polyline between the high and low levels, the
 * spans of high level that get the wash, and blocks where the trace changes too often to draw
 * edge by edge (DESIGN.md §6.2), outlined at both levels. Neighbouring columns with the same
 * level merge, so the shape has as many segments as there are transitions in view.
 */
import { CELL_EMPTY, CELL_FIRST_HIGH, CELL_LAST_HIGH, CELL_TRANSITION } from '@/ipc/bins'

/** `M` starts a subpath, `L` continues it; `high` selects the level the point sits on. */
export type ShapeCommand = readonly ['M' | 'L', number, boolean]

/** Columns `[from, to)`. */
export type ColumnRun = readonly [number, number]

export interface TraceShape {
  path: ShapeCommand[]
  /** `[from, to]` x spans of high level. */
  washes: [number, number][]
  /** `[from, to]` x spans drawn as blocks. */
  blocks: [number, number][]
}

/** Whether column `column` has an edge: within it, or between it and the column before. */
function hasEdge(cells: Uint8Array, column: number): boolean {
  const cell = cells[column] ?? 0
  if (cell & CELL_TRANSITION) return true
  if (column === 0) return false
  const before = ((cells[column - 1] ?? 0) & CELL_LAST_HIGH) !== 0
  return before !== ((cell & CELL_FIRST_HIGH) !== 0)
}

/**
 * Where a trace changes too often to draw edge by edge: runs of at least `minimum` columns with
 * edges, quiet stretches of up to `gap` columns between them included. A clock whose edges are
 * closer than the gap becomes one run per burst.
 */
export function busyRuns(cells: Uint8Array, gap = 1, minimum = 2): [number, number][] {
  const runs: [number, number][] = []
  let from = -1
  let last = -1
  let busy = 0
  const close = () => {
    if (from >= 0 && busy >= minimum) runs.push([from, last + 1])
    from = -1
    busy = 0
  }
  for (let column = 0; column < cells.length; column++) {
    const cell = cells[column] ?? CELL_EMPTY
    if (cell & CELL_EMPTY) break
    if (!hasEdge(cells, column)) continue
    if (from >= 0 && column - last - 1 > gap) close()
    if (from < 0) from = column
    last = column
    busy += 1
  }
  close()
  return runs
}

/** The runs of `runs` within which `cells` change at all, as a data line follows its clock. */
export function changingIn(cells: Uint8Array, runs: readonly ColumnRun[]): [number, number][] {
  return runs
    .filter(([from, to]) => {
      for (let column = from; column < to; column++) {
        if ((cells[column] ?? 0) & CELL_TRANSITION) return true
        if (column > from && hasEdge(cells, column)) return true
      }
      return false
    })
    .map(([from, to]) => [from, to])
}

/** Quiet columns a run of edges may span: edges closer than 3 CSS pixels are not worth telling
 * apart; a UART frame or an I²C burst becomes one block. */
export function denseGap(dpr: number): number {
  return Math.max(1, Math.round(3 * dpr) - 1)
}

/**
 * Blocks are outlined at both levels and both ends.
 *
 * @param cells one byte per column, see `hsync_capture::cell`
 * @param x the x of a column's left edge; `x(cells.length)` is the right edge of the last column
 * @param blocks column runs to draw as blocks, ascending and apart
 * @param wash whether high stretches get the wash; a busy line is clearer as lines and blocks
 */
export function traceShape(
  cells: Uint8Array,
  x: (column: number) => number,
  blocks: readonly ColumnRun[] = [],
  wash = true,
): TraceShape {
  const path: ShapeCommand[] = []
  const washes: [number, number][] = []
  const spans: [number, number][] = []
  let end = cells.length
  for (let column = 0; column < cells.length; column++) {
    if ((cells[column] ?? CELL_EMPTY) & CELL_EMPTY) {
      end = column
      break
    }
  }
  if (end === 0) return { path, washes, blocks: spans }

  let level = ((cells[0] ?? 0) & CELL_FIRST_HIGH) !== 0
  let washFrom: number | null = level ? x(0) : null
  path.push(['M', x(0), level])

  const closeWash = (at: number) => {
    if (washFrom !== null && wash) washes.push([washFrom, at])
    washFrom = null
  }

  const lineTo = (at: number, high: boolean) => {
    const last = path.at(-1)
    if (last?.[1] !== at || last[2] !== high) path.push(['L', at, high])
  }
  const go = (at: number, high: boolean) => {
    if (high === level) return
    lineTo(at, level)
    path.push(['L', at, high])
    if (high) washFrom = at
    else closeWash(at)
    level = high
  }

  let next = 0
  let column = 0
  while (column < end) {
    const block = blocks[next]
    if (block && block[0] <= column) {
      next += 1
      const to = Math.min(block[1], end)
      if (to <= column) continue
      const left = x(column)
      const right = x(to)
      const after = ((cells[to - 1] ?? 0) & CELL_LAST_HIGH) !== 0
      closeWash(left)
      lineTo(left, level)
      // The outline: the left end, both levels, the right end.
      path.push(
        ['L', left, !level],
        ['M', left, true],
        ['L', right, true],
        ['M', left, false],
        ['L', right, false],
        ['M', right, true],
        ['L', right, false],
        ['M', right, after],
      )
      spans.push([left, right])
      level = after
      if (level) washFrom = right
      column = to
      continue
    }
    const cell = cells[column] ?? CELL_EMPTY
    const at = x(column)
    go(at, (cell & CELL_FIRST_HIGH) !== 0)
    if (cell & CELL_TRANSITION) {
      // Transitions within one column show as a full swing at its left edge.
      const last = (cell & CELL_LAST_HIGH) !== 0
      path.push(['L', at, level], ['L', at, !level], ['M', at, last])
      if (last && !level) washFrom = at
      else if (!last && level) closeWash(at)
      level = last
    }
    column += 1
  }
  const right = x(end)
  lineTo(right, level)
  closeWash(right)
  return { path, washes, blocks: spans }
}

/** Lines, of the grid and of traces, are a CSS pixel wide in whole device pixels. */
export function lineWidth(dpr: number): number {
  return Math.max(1, Math.round(dpr))
}
