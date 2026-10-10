/**
 * Decoded demo data for the browser mock: the presentations `cargo run -p hsync-decode --example
 * decode_fixture` exported, queried the way `hsync_decode::Presentation` answers them.
 */
import type { Bus } from '../bindings/Bus'
import type { DecoderLanes } from '../bindings/DecoderLanes'
import type { DecoderStatus } from '../bindings/DecoderStatus'
import type { DecoderView } from '../bindings/DecoderView'
import type { LaneItem } from '../bindings/LaneItem'
import type { StreamCounts } from '../bindings/StreamCounts'
import type { StreamMode } from '../bindings/StreamMode'
import type { StreamPage } from '../bindings/StreamPage'
import type { StreamRow } from '../bindings/StreamRow'
import decoded from './fixtures/protocol-decoded.json'

interface Snapshot {
  bus: Bus
  lanes: [string, LaneItem[]][]
  textRows: StreamRow[]
  hexRows: StreamRow[]
  units: [number, number][]
  counts: StreamCounts
  addresses: [number, number, number][]
  transferWords: [number, [number, number][]][]
}

interface Entry {
  index: number
  id: string
  presentation: Snapshot
}

interface Lane {
  name: string
  items: LaneItem[]
  maxEnd: number[]
}

/** First index in `[0, length)` where `predicate` turns false (it must be monotone). */
function partition(length: number, predicate: (index: number) => boolean): number {
  let lo = 0
  let hi = length
  while (lo < hi) {
    const mid = (lo + hi) >>> 1
    if (predicate(mid)) lo = mid + 1
    else hi = mid
  }
  return lo
}

function lane(name: string, items: LaneItem[]): Lane {
  const sorted = [...items].sort((a, b) => a.start - b.start)
  let running = 0
  const maxEnd = sorted.map((item) => (running = Math.max(running, item.end)))
  return { name, items: sorted, maxEnd }
}

/** `Lane::view` of hsync-decode. */
function laneView(lane: Lane, start: number, end: number, minWidth: number): LaneItem[] {
  const first = partition(lane.maxEnd.length, (i) => (lane.maxEnd[i] ?? 0) < start)
  const out: LaneItem[] = []
  for (let i = first; i < lane.items.length; i++) {
    const item = lane.items[i]
    if (!item || item.start >= end) break
    const inside = item.start === item.end ? item.start >= start : item.end > start
    if (!inside) continue
    const narrow = item.end - item.start < minWidth
    const last = out.at(-1)
    if (narrow && last) {
      if (last.type === 'dense' && item.start <= last.end + minWidth) {
        last.end = Math.max(last.end, item.end)
        continue
      }
      if (last.end - last.start < minWidth && item.start <= last.end + minWidth) {
        out.pop()
        out.push({ type: 'dense', start: last.start, end: Math.max(item.end, last.start) })
        continue
      }
    }
    out.push({ ...item })
  }
  return out
}

class Presentation {
  readonly lanes: Lane[]

  constructor(readonly snapshot: Snapshot) {
    this.lanes = snapshot.lanes.map(([name, items]) => lane(name, items))
  }

  view(start: number, end: number, samplesPerPixel: number): DecoderView {
    const minWidth = Math.ceil(samplesPerPixel * 3)
    const lanes = this.lanes.map((one) => ({
      lane: one.name,
      items: laneView(one, start, end, minWidth),
    }))
    for (const item of lanes.flatMap((one) => one.items)) {
      if (item.type !== 'spiTransfer') continue
      const words = this.snapshot.transferWords.find(([at]) => at === item.start)?.[1] ?? []
      const from = partition(words.length, (i) => (words[i]?.[1] ?? 0) <= start)
      const to = partition(words.length, (i) => (words[i]?.[0] ?? 0) < end)
      if (to > from) {
        item.first = from
        item.last = to - 1
      }
    }
    const empty = lanes.every((one) => one.items.length === 0)
    const header = this.lanes[0]
    const counts = new Map<number, number>()
    for (const [from, to, address] of this.snapshot.addresses) {
      if (from < end && to > start) counts.set(address, (counts.get(address) ?? 0) + 1)
    }
    let address: number | null = null
    let best = 0
    for (const [candidate, count] of counts) {
      if (count > best) [address, best] = [candidate, count]
    }
    let previous: number | null = null
    let next: number | null = null
    if (empty && header) {
      previous = header.items.filter((item) => item.end <= start).at(-1)?.start ?? null
      next = header.items.find((item) => item.start >= end)?.start ?? null
    }
    return { lanes, address, previous, next }
  }

  rows(mode: StreamMode): StreamRow[] {
    return mode === 'text' ? this.snapshot.textRows : this.snapshot.hexRows
  }

  countInside(start: number, end: number): number {
    return this.snapshot.units.filter(([from, to]) => from >= start && from < end && to <= end)
      .length
  }
}

const entries = (decoded as unknown as Entry[]).map((entry) => ({
  index: entry.index,
  id: entry.id,
  presentation: new Presentation(entry.presentation),
}))

/** Decoder states of a capture: all finished, or all still running while it is captured. */
export function mockStatuses(running: boolean): DecoderStatus[] {
  return entries.map(({ index, id, presentation }) => ({
    index,
    id,
    bus: presentation.snapshot.bus,
    state: running ? 'running' : 'done',
    counts: running ? null : presentation.snapshot.counts,
  }))
}

/** Lanes of the decoded demo over `start..end`, limited to samples actually captured. */
export function mockView(start: number, end: number, samplesPerPixel: number): DecoderLanes[] {
  return entries.map(({ index, presentation }) => ({
    index,
    view: presentation.view(start, end, samplesPerPixel),
  }))
}

export function mockStream(
  index: number,
  mode: StreamMode,
  segment: [number, number] | null,
  onlySegment: boolean,
  offset: number,
  count: number,
): StreamPage | null {
  const presentation = entries.find((entry) => entry.index === index)?.presentation
  if (!presentation) return null
  const rows = presentation.rows(mode)
  const scoped =
    segment && onlySegment
      ? rows.filter((row) => row.end >= segment[0] && row.start < segment[1])
      : rows
  const begin = Math.min(offset, scoped.length)
  return {
    total: scoped.length,
    offset: begin,
    rows: scoped.slice(begin, begin + count),
    counts: presentation.snapshot.counts,
    inside: segment ? presentation.countInside(segment[0], segment[1]) : null,
  }
}

export function mockRowAt(index: number, mode: StreamMode, sample: number): number | null {
  const presentation = entries.find((entry) => entry.index === index)?.presentation
  if (!presentation) return null
  const rows = presentation.rows(mode)
  const at = rows.findIndex((row) => row.end >= sample)
  return at < 0 ? rows.length : at
}
