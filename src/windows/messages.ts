/**
 * What the main window and the data stream's own window tell each other (DESIGN.md §9). Each
 * piece of state has one owner: the main window owns the capture shown, the segment and the
 * search; the data stream window owns its listing's scroll, its selection and the row under
 * the pointer. The stream listed is told both ways, and each side takes the other's change.
 */
import type { CaptureSummary } from '@/ipc/bindings/CaptureSummary'
import type { LiveStatus } from '@/ipc/bindings/LiveStatus'
import type { SearchResult } from '@/ipc/bindings/SearchResult'
import type { Segment } from '@/stores/view'

/** Samples `start..end` of an event. */
export interface Span {
  start: number
  end: number
}

/** A decoder's stream, by position. */
export interface ListingRef {
  decoder: number | null
  stream: number
}

/** From the main window to the data stream window. */
export type ToStream =
  | { type: 'capture'; summary: CaptureSummary | null; live: LiveStatus | null }
  | { type: 'segment'; segment: Segment | null }
  | { type: 'found'; result: SearchResult | null; current: number | null }
  | { type: 'current'; current: number | null }
  | { type: 'listing'; listing: ListingRef }

/** From the data stream window to the main window. */
export type ToMain =
  /** The window is ready: the main window tells it everything. */
  | { type: 'hello' }
  /** An event was selected: the traces show it, with the segment over it. */
  | { type: 'reveal'; span: Span }
  /** The row under the pointer, marked on the timeline and the overview strip. */
  | { type: 'hot'; span: Span | null }
  | { type: 'listing'; listing: ListingRef }
  /** Back beside the timeline. */
  | { type: 'dock' }
