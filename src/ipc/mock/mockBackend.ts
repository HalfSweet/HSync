/**
 * In-browser stand-in for the Rust backend, used by `pnpm dev` outside Tauri. It replays DSView's
 * demo capture from edge lists that `cargo run -p hsync-fileio --example dsl_edges` exported, and
 * answers queries with the same algorithms as `hsync-capture` (see its tests for the semantics).
 */
import type { Backend } from '../backend'
import type { BinsRequest } from '../bindings/BinsRequest'
import type { CaptureSummary } from '../bindings/CaptureSummary'
import type { Direction } from '../bindings/Direction'
import type { PulseInfo } from '../bindings/PulseInfo'
import {
  CELL_EMPTY,
  CELL_FIRST_HIGH,
  CELL_LAST_HIGH,
  CELL_TRANSITION,
  type BinsResult,
} from '../bins'
import fixture from './fixtures/protocol-demo.json'

/** Index of the first edge after `index`, i.e. the number of edges at or before it. */
function upperBound(edges: readonly number[], index: number): number {
  let lo = 0
  let hi = edges.length
  while (lo < hi) {
    const mid = (lo + hi) >>> 1
    if ((edges[mid] ?? 0) <= index) lo = mid + 1
    else hi = mid
  }
  return lo
}

export class MockChannel {
  constructor(
    readonly first: boolean,
    readonly edges: readonly number[],
    readonly length: number,
  ) {}

  sample(index: number): boolean {
    return this.first !== (upperBound(this.edges, index) % 2 === 1)
  }

  nextChange(index: number): number | null {
    if (index >= this.length) return null
    return this.edges[upperBound(this.edges, index)] ?? null
  }

  prevChange(index: number): number | null {
    if (index >= this.length || index === 0) return null
    const at = upperBound(this.edges, index)
    return at > 0 ? (this.edges[at - 1] ?? 0) - 1 : null
  }

  bins(start: number, end: number, bins: number): Uint8Array {
    const out = new Uint8Array(bins).fill(CELL_EMPTY)
    const bound = (i: number) => start + Math.floor(((end - start) * i) / bins)
    for (let i = 0; i < bins; i++) {
      const lo = bound(i)
      const hi = Math.min(bound(i + 1), this.length)
      if (lo >= hi) continue
      const level = this.sample(lo)
      const change = this.nextChange(lo)
      let cell = level ? CELL_FIRST_HIGH : 0
      if (change !== null && change < hi) {
        cell |= CELL_TRANSITION
        if (this.sample(hi - 1)) cell |= CELL_LAST_HIGH
      } else if (level) {
        cell |= CELL_LAST_HIGH
      }
      out[i] = cell
    }
    return out
  }
}

interface Fixture {
  samplerate: number
  sampleCount: number
  triggerSample: number | null
  triggerTimeMs: number | null
  channels: { index: number; name: string; first: boolean; edges: number[] }[]
  decoders: unknown
}

/** Rejection reason shaped like a backend UiError, but an Error as lint rules require. */
function uiError(code: string, params: Record<string, string | number | boolean> = {}) {
  return Object.assign(new Error(code), { code, params })
}

export function createMockBackend(data: Fixture = fixture): Backend {
  const channels = new Map(
    data.channels.map((channel) => [
      channel.index,
      new MockChannel(channel.first, channel.edges, data.sampleCount),
    ]),
  )
  let summary: CaptureSummary | null = null
  let lastId = 0

  const channel = (captureId: number, index: number): MockChannel => {
    if (!summary || summary.id !== captureId) throw uiError('error.session.staleCapture')
    const found = channels.get(index)
    if (!found) throw uiError('error.session.unknownChannel', { channel: index })
    return found
  }

  return {
    async captureSummary() {
      return Promise.resolve(summary)
    },

    async openDemo() {
      lastId += 1
      summary = {
        id: lastId,
        source: { kind: 'demo' },
        info: {
          samplerate: data.samplerate,
          sampleCount: data.sampleCount,
          triggerSample: data.triggerSample,
          triggerTimeMs: data.triggerTimeMs,
          channels: data.channels.map((channel) => channel.index),
        },
        channels: data.channels.map(({ index, name }) => ({ index, name })),
        decoders: data.decoders,
      }
      return Promise.resolve(summary)
    },

    async openFile() {
      return Promise.reject(uiError('error.app.unavailableInBrowser'))
    },

    async bins(request: BinsRequest): Promise<BinsResult> {
      const bins = Math.min(request.bins, request.end - request.start)
      const cells = request.channels.map(
        (index) =>
          [
            index,
            channel(request.captureId, index).bins(request.start, request.end, bins),
          ] as const,
      )
      return Promise.resolve({
        start: request.start,
        end: request.end,
        bins,
        channels: new Map(cells),
      })
    },

    async pulse(captureId, index, sample): Promise<PulseInfo | null> {
      const data = channel(captureId, index)
      if (sample >= data.length) return Promise.resolve(null)
      const prev = data.prevChange(sample)
      const next = data.nextChange(sample)
      return Promise.resolve({
        high: data.sample(sample),
        start: prev === null ? 0 : prev + 1,
        end: next ?? data.length,
        startIsEdge: prev !== null,
        endIsEdge: next !== null,
        nextEnd: next === null ? null : data.nextChange(next),
      })
    },

    async edge(captureId, index, sample, direction: Direction) {
      const data = channel(captureId, index)
      if (direction === 'next') return Promise.resolve(data.nextChange(sample))
      const prev = data.prevChange(sample)
      return Promise.resolve(prev === null ? null : prev + 1)
    },
  }
}
