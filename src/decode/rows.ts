/**
 * Which annotation rows of a decoder show, stored as DSView stores it: the entry's `show` map,
 * keyed `<decoder id>: <row description>` (`pv/data/decode/row.cpp`); rows not listed show.
 */
import type { AnnotationLane } from '@/ipc/bindings/AnnotationLane'

/** The `show` key of a lane (`eeprom24xx/fields` described `Fields`). */
export function showKey(lane: AnnotationLane): string {
  const decoder = lane.lane.slice(0, lane.lane.lastIndexOf('/'))
  return `${decoder}: ${lane.desc}`
}

function showMap(entry: unknown): Record<string, unknown> {
  const show = (entry as { show?: unknown } | undefined)?.show
  return typeof show === 'object' && show !== null ? (show as Record<string, unknown>) : {}
}

export function laneShown(entry: unknown, lane: AnnotationLane): boolean {
  return showMap(entry)[showKey(lane)] !== false
}

/** The entry with `lane` shown or hidden. */
export function withLaneShown(entry: unknown, lane: AnnotationLane, shown: boolean): unknown {
  const base = (typeof entry === 'object' && entry !== null ? entry : {}) as Record<string, unknown>
  return { ...base, show: { ...showMap(entry), [showKey(lane)]: shown } }
}
