/**
 * Decoded demo data for the browser mock: the events the real decoders put out for the demo
 * capture (`HSYNC_WRITE_FIXTURES=1 cargo test -p hsync-session --test fixtures`), queried the
 * way the stores of `hsync-uiharu` answer.
 */
import { entryKinds, entryLabel, isEntry } from '@/decode/catalog'

import type { DecoderLanes } from '../bindings/DecoderLanes'
import type { DecoderStatus } from '../bindings/DecoderStatus'
import type { EventRow } from '../bindings/EventRow'
import type { LaneBlock } from '../bindings/LaneBlock'
import type { SearchHit } from '../bindings/SearchHit'
import type { SearchResult } from '../bindings/SearchResult'
import type { StreamPage } from '../bindings/StreamPage'
import type { StreamView } from '../bindings/StreamView'
import demo from './fixtures/protocol-demo.json'
import fixture from './fixtures/protocol-uiharu.json'

interface FixtureStream {
  name: string
  kind: string
  schema: string
  rows: EventRow[]
}

interface FixtureDecoder {
  label: string
  kinds: string[]
  streams: FixtureStream[]
}

const RECORDED = fixture.decoders as FixtureDecoder[]

/** The demo capture's decoder entries, as the backend translates DSView's. */
export const MOCK_ENTRIES: unknown[] = fixture.entries

/** Hits a search returns at most, as the backend. */
const SEARCH_LIMIT = 10_000
/** Blocks a lane returns at most, as the backend. */
const LANE_LIMIT = 4096

/** The first sample after an event, an event without length counting as one sample wide. */
const reach = (row: EventRow) => Math.max(row.end, row.start + 1)

/** The recording that goes with each of the capture's entries: the one of its first node type. */
function matched(decoders: unknown): { entry: unknown; recorded: FixtureDecoder | null }[] {
  return (Array.isArray(decoders) ? decoders : []).map((entry) => ({
    entry,
    recorded: isEntry(entry)
      ? (RECORDED.find((one) => one.kinds[0] === entryKinds(entry)[0]) ?? null)
      : null,
  }))
}

function streamOf(decoders: unknown, index: number, stream: number): FixtureStream | null {
  return matched(decoders)[index]?.recorded?.streams[stream] ?? null
}

export function mockStatuses(decoders: unknown, running: boolean): DecoderStatus[] {
  return matched(decoders).map(({ entry, recorded }, index) => {
    const kinds = isEntry(entry) ? entryKinds(entry) : []
    const label = isEntry(entry) ? entryLabel(entry, (type) => type) : ''
    if (!recorded) {
      return {
        index,
        label,
        kinds,
        state: 'invalid',
        error: { code: 'error.decode.invalid', params: { detail: 'not in the recording' } },
        streams: [],
      }
    }
    return {
      index,
      label,
      kinds,
      state: running ? 'running' : 'done',
      error: null,
      streams: recorded.streams.map((stream, at) => ({
        index: at,
        name: stream.name,
        kind: stream.kind,
        schema: stream.schema,
        progress: {
          events: running ? 0 : stream.rows.length,
          settled: running ? 0 : demo.sampleCount,
          ended: !running,
        },
      })),
    }
  })
}

/** The rows overlapping `start..end`, in time order. */
function overlapping(rows: readonly EventRow[], start: number, end: number): EventRow[] {
  return rows.filter((row) => row.start < end && reach(row) > start)
}

/** The lane over `start..end`, as `Store::view`: narrow events pack into dense blocks. */
function laneView(rows: readonly EventRow[], start: number, end: number, spp: number): StreamView {
  const unit = Math.max(1, Math.floor(spp * 2))
  const blocks: LaneBlock[] = []
  let run: { first: EventRow; start: number; end: number; count: number } | null = null
  const flush = () => {
    if (!run) return
    if (run.count === 1) blocks.push({ type: 'event', ...run.first })
    else blocks.push({ type: 'dense', start: run.start, end: run.end, count: run.count })
    run = null
  }
  for (const row of overlapping(rows, start, end)) {
    if (blocks.length >= LANE_LIMIT) return { blocks, truncated: true }
    if (reach(row) - row.start >= unit) {
      flush()
      blocks.push({ type: 'event', ...row })
      continue
    }
    if (run && row.start <= run.end + unit) {
      run.end = Math.max(run.end, reach(row))
      run.count += 1
    } else {
      flush()
      run = { first: row, start: row.start, end: reach(row), count: 1 }
    }
  }
  flush()
  return { blocks, truncated: false }
}

export function mockView(
  decoders: unknown,
  start: number,
  end: number,
  samplesPerPixel: number,
): DecoderLanes[] {
  return matched(decoders).flatMap(({ recorded }, index) =>
    recorded
      ? [
          {
            index,
            streams: recorded.streams.map((stream, at) => ({
              stream: at,
              view: laneView(stream.rows, start, end, samplesPerPixel),
            })),
          },
        ]
      : [],
  )
}

export function mockStream(
  decoders: unknown,
  index: number,
  stream: number,
  segment: [number, number] | null,
  onlySegment: boolean,
  offset: number,
  count: number,
): StreamPage | null {
  const found = streamOf(decoders, index, stream)
  if (!found) return null
  const scoped =
    segment && onlySegment ? overlapping(found.rows, segment[0], segment[1]) : found.rows
  const begin = Math.min(offset, scoped.length)
  const finish = Math.min(begin + count, scoped.length)
  const inside = segment
    ? overlapping(found.rows, segment[0], segment[1]).filter(
        (row) => row.start >= segment[0] && reach(row) <= segment[1],
      ).length
    : null
  return {
    total: scoped.length,
    offset: begin,
    rows: scoped.slice(begin, finish),
    events: found.rows.length,
    inside,
  }
}

export function mockRowAt(
  decoders: unknown,
  index: number,
  stream: number,
  sample: number,
): number | null {
  const found = streamOf(decoders, index, stream)
  if (!found) return null
  const at = found.rows.findIndex((row) => reach(row) > sample)
  return at < 0 ? found.rows.length : at
}

/** A query as `hsync_uiharu::Needle` reads it. */
export interface Needle {
  /** Runs of numbers: one is a value, more a sequence inside an array. */
  sequences: number[][]
  /** Kind and enumerated-value names, lower case. */
  names: string[]
}

function parseNumber(word: string): number | null {
  const hex = /^0x([0-9a-f]+)$/i.exec(word)
  if (hex?.[1]) return parseInt(hex[1], 16)
  if (/^-?\d+$/.test(word)) return parseInt(word, 10)
  if (/^-?\d*\.\d+$/.test(word)) return parseFloat(word)
  return null
}

export function parseNeedle(query: string): Needle | null {
  const sequences: number[][] = []
  const names: string[] = []
  let run: number[] = []
  for (const word of query.split(/\s+/).filter(Boolean)) {
    const number = parseNumber(word)
    if (number !== null) {
      run.push(number)
      continue
    }
    if (run.length > 0) sequences.push(run)
    run = []
    names.push(word.toLowerCase())
  }
  if (run.length > 0) sequences.push(run)
  return sequences.length > 0 || names.length > 0 ? { sequences, names } : null
}

/** Whether a payload matches: every name and every sequence is found; keys never match. */
function matches(payload: unknown, needle: Needle): boolean {
  const foundSequences = needle.sequences.map(() => false)
  const foundNames = needle.names.map(() => false)
  const number = (value: number) => {
    needle.sequences.forEach((sequence, at) => {
      if (sequence.length === 1 && sequence[0] === value) foundSequences[at] = true
    })
  }
  const array = (values: number[]) => {
    needle.sequences.forEach((sequence, at) => {
      if (sequence.length < 2 || foundSequences[at]) return
      for (let from = 0; from + sequence.length <= values.length; from++) {
        if (sequence.every((wanted, offset) => values[from + offset] === wanted)) {
          foundSequences[at] = true
          return
        }
      }
    })
  }
  const walk = (value: unknown): void => {
    if (typeof value === 'number') number(value)
    else if (typeof value === 'string') {
      const lower = value.toLowerCase()
      needle.names.forEach((name, at) => {
        if (lower === name) foundNames[at] = true
      })
    } else if (Array.isArray(value)) {
      if (value.every((item) => typeof item === 'number')) array(value)
      value.forEach(walk)
    } else if (typeof value === 'object' && value !== null) {
      Object.values(value).forEach(walk)
    }
  }
  walk(payload)
  return foundSequences.every(Boolean) && foundNames.every(Boolean)
}

export function mockSearch(decoders: unknown, query: string): SearchResult | null {
  const needle = parseNeedle(query)
  if (!needle) return null
  const hits: SearchHit[] = []
  let truncated = false
  matched(decoders).forEach(({ recorded }, decoder) => {
    recorded?.streams.forEach((stream, at) => {
      stream.rows.forEach((row, index) => {
        if (!matches(row.payload, needle)) return
        if (hits.length >= SEARCH_LIMIT) {
          truncated = true
          return
        }
        hits.push({ decoder, stream: at, row: index, start: row.start, end: row.end })
      })
    })
  })
  hits.sort((a, b) => a.start - b.start || a.decoder - b.decoder || a.stream - b.stream)
  return { hits, truncated }
}

/** Where decoder `index` has events within `start..end`, joined when closer than `resolution`. */
export function mockCoverage(
  decoders: unknown,
  index: number,
  start: number,
  end: number,
  resolution: number,
): [number, number][] | null {
  const recorded = matched(decoders)[index]?.recorded
  if (!recorded) return null
  const stretches = recorded.streams
    .flatMap((stream) => overlapping(stream.rows, start, end).map((row) => [row.start, reach(row)]))
    .sort((a, b) => (a[0] ?? 0) - (b[0] ?? 0))
  const joined: [number, number][] = []
  for (const [from = 0, to = 0] of stretches) {
    const last = joined.at(-1)
    if (last && from <= last[1] + resolution) last[1] = Math.max(last[1], to)
    else joined.push([from, to])
  }
  return joined
}
