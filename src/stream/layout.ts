/** How the data stream panel shares the window with the timeline (DESIGN.md §9). */

/** The width the panel opens at. */
export const PANEL_WIDTH = 400
/** The panel is never narrower than this. */
export const PANEL_MIN = 300
/** What the timeline keeps beside the panel, label column included: its rows' least width. */
export const TIMELINE_MIN = 640

/**
 * The panel's width when `wanted` is asked for in a window `room` px wide: no narrower than
 * its minimum, and no wider than leaves the timeline its share.
 */
export function panelWidth(wanted: number, room: number): number {
  return Math.max(PANEL_MIN, Math.min(wanted, room - TIMELINE_MIN))
}

/** Whether an event spans exactly `segment`, i.e. it is the one selected. */
export function spans(event: { start: number; end: number }, segment: [number, number] | null) {
  return segment !== null && event.start === segment[0] && event.end === segment[1]
}
