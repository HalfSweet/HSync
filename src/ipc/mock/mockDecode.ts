/**
 * Decoded demo data for the browser mock: the presentations `cargo run -p hsync-decode --example
 * decode_fixture` exported, queried the way `hsync_decode::Presentation` answers them.
 */
import type { Bus } from '../bindings/Bus'
import type { DataLine } from '../bindings/DataLine'
import type { DecoderLanes } from '../bindings/DecoderLanes'
import type { DecoderStatus } from '../bindings/DecoderStatus'
import type { DecoderView } from '../bindings/DecoderView'
import type { LaneItem } from '../bindings/LaneItem'
import type { SearchHit } from '../bindings/SearchHit'
import type { SearchResult } from '../bindings/SearchResult'
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
  sequences: { line: DataLine | null; words: [number, number, number][] }[]
  ids: [number, number, number][]
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

const fixtures = (decoded as unknown as Entry[]).map((entry) => ({
  id: entry.id,
  presentation: new Presentation(entry.presentation),
}))

interface Matched {
  index: number
  id: string
  presentation: Presentation | null
}

/**
 * The mock's results for a decoder list: each entry gets the demo's presentation of the same
 * decoder id, the way the demo capture would decode it; others decode to nothing.
 */
function match(decoders: unknown): Matched[] {
  const list = Array.isArray(decoders) ? (decoders as unknown[]) : []
  return list.map((entry, index) => {
    const id =
      typeof entry === 'object' &&
      entry !== null &&
      typeof (entry as { id?: unknown }).id === 'string'
        ? (entry as { id: string }).id
        : ''
    return { index, id, presentation: fixtures.find((one) => one.id === id)?.presentation ?? null }
  })
}

/** Decoder states of a capture: finished, or still running while it is captured. */
export function mockStatuses(decoders: unknown, running: boolean): DecoderStatus[] {
  return match(decoders).map(({ index, id, presentation }) => ({
    index,
    id,
    bus: presentation?.snapshot.bus ?? null,
    state: running ? 'running' : presentation ? 'done' : 'unsupported',
    counts: running || !presentation ? null : presentation.snapshot.counts,
    lanes: [],
    error: null,
  }))
}

/** Lanes of the decoded demo over `start..end`. */
export function mockView(
  decoders: unknown,
  start: number,
  end: number,
  samplesPerPixel: number,
): DecoderLanes[] {
  return match(decoders).flatMap(({ index, presentation }) =>
    presentation ? [{ index, view: presentation.view(start, end, samplesPerPixel) }] : [],
  )
}

export function mockStream(
  decoders: unknown,
  index: number,
  mode: StreamMode,
  segment: [number, number] | null,
  onlySegment: boolean,
  offset: number,
  count: number,
): StreamPage | null {
  const presentation = match(decoders)[index]?.presentation
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

export function mockRowAt(
  decoders: unknown,
  index: number,
  mode: StreamMode,
  sample: number,
): number | null {
  const presentation = match(decoders)[index]?.presentation
  if (!presentation) return null
  const rows = presentation.rows(mode)
  const at = rows.findIndex((row) => row.end >= sample)
  return at < 0 ? rows.length : at
}

/** `Needle::parse` of hsync-decode: word sequences to find (text ignoring ASCII case). */
function parseNeedle(
  query: string,
): { sequences: [number[], boolean][]; number: number | null } | null {
  const trimmed = query.trim()
  if (!trimmed) return null
  const sequences: [number[], boolean][] = []
  let number: number | null = null
  const tokens = trimmed.split(/\s+/).map((token) => token.replace(/^0x/i, ''))
  if (tokens.every((digits) => /^[0-9a-f]{1,8}$/i.test(digits))) {
    const values = tokens.map((digits) => parseInt(digits, 16))
    const single = tokens.length === 1 ? tokens[0] : undefined
    if (single !== undefined) {
      number = values[0] ?? null
      if (single.length > 2 && single.length % 2 === 0) {
        sequences.push([single.match(/../g)?.map((pair) => parseInt(pair, 16)) ?? [], false])
      }
    }
    sequences.push([values, false])
  }
  const text = [...new TextEncoder().encode(trimmed)].map((byte) =>
    byte >= 65 && byte <= 90 ? byte + 32 : byte,
  )
  if (!sequences.some(([words]) => words.join() === text.join())) sequences.push([text, true])
  return { sequences, number }
}

/** The line a row lists words of; `undefined` for rows without words. */
function rowLine(row: StreamRow): DataLine | null | undefined {
  switch (row.type) {
    case 'i2c':
    case 'uartText':
    case 'can':
      return null
    case 'hex':
    case 'run':
      return row.line
    case 'spiText':
      return row.line
    default:
      return undefined
  }
}

/** Values or text in the decoded demo, as `DecodeSet::search` finds them. */
export function mockSearch(
  decoders: unknown,
  query: string,
  mode: StreamMode,
): SearchResult | null {
  const needle = parseNeedle(query)
  if (!needle) return null
  const hits: SearchHit[] = []
  for (const { index, presentation } of match(decoders)) {
    if (!presentation) continue
    const rows = presentation.rows(mode)
    const found: [number, number, DataLine | null][] = []
    for (const sequence of presentation.snapshot.sequences) {
      for (const [pattern, fold] of needle.sequences) {
        const words = sequence.words
        let at = 0
        while (pattern.length > 0 && at + pattern.length <= words.length) {
          const same = pattern.every((wanted, i) => {
            const word = words[at + i]?.[0] ?? -1
            return (fold && word >= 65 && word <= 90 ? word + 32 : word) === wanted
          })
          if (same) {
            found.push([
              words[at]?.[1] ?? 0,
              words[at + pattern.length - 1]?.[2] ?? 0,
              sequence.line,
            ])
            at += pattern.length
          } else {
            at += 1
          }
        }
      }
    }
    if (needle.number !== null) {
      for (const [id, start, end] of presentation.snapshot.ids) {
        if (id === needle.number) found.push([start, end, null])
      }
    }
    found.sort((a, b) => a[0] - b[0] || String(a[2]).localeCompare(String(b[2])) || b[1] - a[1])
    found.forEach(([start, end, line], i) => {
      const previous = found[i - 1]
      if (previous && previous[0] === start && previous[2] === line) return
      const row = rows.findIndex(
        (one) => rowLine(one) === line && one.start <= start && start < one.end,
      )
      hits.push({ decoder: index, start, end, line, row: row < 0 ? null : row })
    })
  }
  hits.sort((a, b) => a.start - b.start || a.decoder - b.decoder)
  return { hits: hits.slice(0, 10_000), truncated: hits.length > 10_000 }
}
