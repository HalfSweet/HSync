/**
 * The shape of one trace from its column cells: a polyline between the high and low levels and
 * the spans of high level that get the wash. Neighbouring columns with the same level merge, so
 * the shape has as many segments as there are transitions in view.
 */
import { CELL_EMPTY, CELL_FIRST_HIGH, CELL_LAST_HIGH, CELL_TRANSITION } from '@/ipc/bins'

/** `M` starts a subpath, `L` continues it; `high` selects the level the point sits on. */
export type ShapeCommand = readonly ['M' | 'L', number, boolean]

export interface TraceShape {
  path: ShapeCommand[]
  /** `[from, to]` x spans of high level. */
  washes: [number, number][]
}

/**
 * @param cells one byte per column, see `hsync_capture::cell`
 * @param x the x of a column's left edge; `x(cells.length)` is the right edge of the last column
 */
export function traceShape(cells: Uint8Array, x: (column: number) => number): TraceShape {
  const path: ShapeCommand[] = []
  const washes: [number, number][] = []
  if (cells.length === 0 || ((cells[0] ?? 0) & CELL_EMPTY) !== 0) return { path, washes }

  let level = ((cells[0] ?? 0) & CELL_FIRST_HIGH) !== 0
  let washFrom: number | null = level ? x(0) : null
  path.push(['M', x(0), level])

  const go = (at: number, high: boolean) => {
    if (high === level) return
    path.push(['L', at, level], ['L', at, high])
    if (high) washFrom = at
    else if (washFrom !== null) {
      washes.push([washFrom, at])
      washFrom = null
    }
    level = high
  }

  let end = cells.length
  for (let column = 0; column < cells.length; column++) {
    const cell = cells[column] ?? CELL_EMPTY
    if (cell & CELL_EMPTY) {
      end = column
      break
    }
    const at = x(column)
    go(at, (cell & CELL_FIRST_HIGH) !== 0)
    if (cell & CELL_TRANSITION) {
      // Transitions within one column show as a full swing at its left edge.
      const last = (cell & CELL_LAST_HIGH) !== 0
      path.push(['L', at, level], ['L', at, !level], ['M', at, last])
      if (last && !level) washFrom = at
      else if (!last && level && washFrom !== null) {
        washes.push([washFrom, at])
        washFrom = null
      }
      level = last
    }
  }
  const right = x(end)
  path.push(['L', right, level])
  if (washFrom !== null) washes.push([washFrom, right])
  return { path, washes }
}
