/**
 * What the top bar's trigger button lists (DESIGN.md §9.4): up to three channels with their
 * conditions; with more, the first two and a count of all.
 */
export interface ArmedChannel<C> {
  index: number
  condition: C
}

export interface TriggerSummary<C> {
  shown: ArmedChannel<C>[]
  /** Total number of armed channels when not all are shown, otherwise 0. */
  more: number
}

export function summarizeTriggers<C>(armed: readonly ArmedChannel<C>[]): TriggerSummary<C> {
  if (armed.length > 3) return { shown: armed.slice(0, 2), more: armed.length }
  return { shown: [...armed], more: 0 }
}
