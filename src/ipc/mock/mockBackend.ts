/**
 * In-browser stand-in for the Rust backend, used by `pnpm dev` outside Tauri. It replays DSView's
 * demo capture from edge lists that `cargo run -p hsync-fileio --example dsl_edges` exported, and
 * answers queries with the same algorithms as `hsync-capture` (see its tests for the semantics).
 */
import type { Backend, DecodedEvent, StreamRequest, Unsubscribe } from '../backend'
import type { BinsRequest } from '../bindings/BinsRequest'
import type { CaptureRequest } from '../bindings/CaptureRequest'
import type { CaptureSummary } from '../bindings/CaptureSummary'
import type { DeviceState } from '../bindings/DeviceState'
import type { DeviceSummary } from '../bindings/DeviceSummary'
import type { Direction } from '../bindings/Direction'
import type { LiveStatus } from '../bindings/LiveStatus'
import type { OptionKey } from '../bindings/OptionKey'
import type { OptionValue } from '../bindings/OptionValue'
import type { PatternCondition } from '../bindings/PatternCondition'
import type { PulseInfo } from '../bindings/PulseInfo'
import {
  CELL_EMPTY,
  CELL_FIRST_HIGH,
  CELL_LAST_HIGH,
  CELL_TRANSITION,
  type BinsResult,
} from '../bins'
import fixture from './fixtures/protocol-demo.json'
import { mockRowAt, mockSearch, mockStatuses, mockStream, mockView } from './mockDecode'
import { DEMO_ID, MOCK_DEVICES, MockDemoDevice, uiError } from './mockDevice'

/** How long a simulated capture takes to fill, and in how many steps. */
const LIVE_STEPS = 12
const LIVE_STEP_MS = 90
/** How long a triggered capture waits for its trigger. */
const TRIGGER_WAIT_MS = 700

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
    /** Samples captured so far; grows while a simulated capture runs. */
    public length: number,
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

  /** Median samples between rising edges over the first 65536 of them (see `clock_period`). */
  clockPeriod(): number | null {
    const rising = this.edges.filter((edge) => this.sample(edge)).slice(0, 1 << 16)
    const periods = rising.slice(1).map((edge, i) => edge - (rising[i] ?? 0))
    if (periods.length === 0) return null
    periods.sort((a, b) => a - b)
    return periods[Math.floor(periods.length / 2)] ?? null
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

/**
 * `LogicCapture::find_pattern` of hsync-capture over edge lists: every hit is at sample 0 or at
 * an edge of one of the channels, so only those samples need checking.
 */
export function findPattern(
  channels: { channel: MockChannel; level: PatternCondition['level'] }[],
  length: number,
  from: number,
  forward: boolean,
): number | null {
  if (channels.length === 0 || length === 0) return null
  const edges = channels.some(({ level }) => level !== 'low' && level !== 'high')
  const matches = (sample: number) =>
    channels.every(({ channel, level }) => {
      const now = channel.sample(sample)
      const before = sample > 0 ? channel.sample(sample - 1) : null
      switch (level) {
        case 'low':
          return !now
        case 'high':
          return now
        case 'rising':
          return before === false && now
        case 'falling':
          return before === true && !now
        default:
          return before !== null && before !== now
      }
    })
  const hit = (sample: number) => matches(sample) && (edges || sample === 0 || !matches(sample - 1))
  const candidates = [...new Set([0, ...channels.flatMap(({ channel }) => channel.edges)])]
    .filter((sample) => sample < length)
    .sort((a, b) => a - b)
  const start = Math.min(from, length - 1)
  if (forward) return candidates.find((sample) => sample >= start && hit(sample)) ?? null
  return candidates.filter((sample) => sample <= start && hit(sample)).at(-1) ?? null
}

interface Fixture {
  samplerate: number
  sampleCount: number
  triggerSample: number | null
  triggerTimeMs: number | null
  channels: { index: number; name: string; first: boolean; edges: number[] }[]
  decoders: unknown
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
  let device: MockDemoDevice | null = null
  const liveHandlers = new Set<(status: LiveStatus) => void>()
  const decodedHandlers = new Set<(event: DecodedEvent) => void>()
  let live: { status: LiveStatus; timers: ReturnType<typeof setTimeout>[] } | null = null

  const channel = (captureId: number, index: number): MockChannel => {
    if (!summary || summary.id !== captureId) throw uiError('error.session.staleCapture')
    const found = channels.get(index)
    if (!found) throw uiError('error.session.unknownChannel', { channel: index })
    return found
  }

  const setLength = (length: number) => {
    for (const data of channels.values()) data.length = length
    if (summary) summary = { ...summary, info: { ...summary.info, sampleCount: length } }
  }

  const emit = (patch: Partial<LiveStatus>) => {
    if (!live) return
    live.status = { ...live.status, ...patch }
    for (const handler of liveHandlers) handler(live.status)
  }

  const finish = (end: 'completed' | 'stopped') => {
    if (!live) return
    for (const timer of live.timers) clearTimeout(timer)
    emit({ phase: 'ended', end })
    const captureId = live.status.captureId
    live = null
    if (device) device.capturing = false
    // The decoders run once the capture is complete.
    setTimeout(() => {
      for (const { index } of mockStatuses(summary?.decoders, false)) {
        for (const handler of decodedHandlers) handler({ captureId, index })
      }
    }, 150)
  }

  const subscribe = <T>(handlers: Set<(value: T) => void>, handler: (value: T) => void) => {
    handlers.add(handler)
    return Promise.resolve<Unsubscribe>(() => handlers.delete(handler))
  }

  return {
    async captureSummary() {
      return Promise.resolve(summary)
    },

    async openDemo() {
      if (live) return Promise.reject(uiError('error.session.busy'))
      lastId += 1
      setLength(data.sampleCount)
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

    async saveFile() {
      return Promise.reject(uiError('error.app.unavailableInBrowser'))
    },

    async exportFile() {
      return Promise.reject(uiError('error.app.unavailableInBrowser'))
    },

    async cancelFile() {
      return Promise.resolve()
    },

    async saveText() {
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

    async clockPeriod(captureId, index) {
      return Promise.resolve(channel(captureId, index).clockPeriod())
    },

    async edge(captureId, index, sample, direction: Direction) {
      const data = channel(captureId, index)
      if (direction === 'next') return Promise.resolve(data.nextChange(sample))
      const prev = data.prevChange(sample)
      return Promise.resolve(prev === null ? null : prev + 1)
    },

    async decodeList(captureId: number) {
      if (!summary || summary.id !== captureId)
        return Promise.reject(uiError('error.session.staleCapture'))
      return Promise.resolve(mockStatuses(summary.decoders, live?.status.captureId === captureId))
    },

    async decodeView(captureId: number, start: number, end: number, samplesPerPixel: number) {
      if (!summary || summary.id !== captureId)
        return Promise.reject(uiError('error.session.staleCapture'))
      if (live?.status.captureId === captureId) return Promise.resolve([])
      return Promise.resolve(mockView(summary.decoders, start, end, samplesPerPixel))
    },

    async decodeStream(request: StreamRequest) {
      if (!summary || summary.id !== request.captureId)
        return Promise.reject(uiError('error.session.staleCapture'))
      if (live?.status.captureId === request.captureId) return Promise.resolve(null)
      return Promise.resolve(
        mockStream(
          summary.decoders,
          request.index,
          request.mode,
          request.segment,
          request.onlySegment,
          request.offset,
          request.count,
        ),
      )
    },

    async decodeRowAt(captureId, index, mode, sample) {
      if (!summary || summary.id !== captureId)
        return Promise.reject(uiError('error.session.staleCapture'))
      return Promise.resolve(mockRowAt(summary.decoders, index, mode, sample))
    },

    async decodeSearch(captureId, query, mode) {
      if (!summary || summary.id !== captureId)
        return Promise.reject(uiError('error.session.staleCapture'))
      const result = mockSearch(summary.decoders, query, mode)
      // Decoders of a capture still being taken have found nothing yet.
      const running = live?.status.captureId === captureId
      return Promise.resolve(result && running ? { hits: [], truncated: false } : result)
    },

    async edgeCounts(captureId, from, to, indices) {
      if (!summary || summary.id !== captureId)
        return Promise.reject(uiError('error.session.staleCapture'))
      const unknown = indices.find((index) => !channels.has(index))
      if (unknown !== undefined)
        return Promise.reject(uiError('error.session.unknownChannel', { channel: unknown }))
      const [low, high] = [Math.min(from, to), Math.max(from, to)]
      return Promise.resolve(
        indices.flatMap((index) => {
          const data = channels.get(index)
          if (!data) return []
          const inside = data.edges.filter((edge) => edge > low && edge <= high)
          const rising = inside.filter((edge) => data.sample(edge)).length
          return [{ channel: index, rising, falling: inside.length - rising }]
        }),
      )
    },

    async find(captureId, conditions: PatternCondition[], sample, direction) {
      if (!summary || summary.id !== captureId)
        return Promise.reject(uiError('error.session.staleCapture'))
      const resolved = conditions.flatMap(({ channel: index, level }) => {
        const found = channels.get(index)
        return found ? [{ channel: found, level }] : []
      })
      if (resolved.length < conditions.length) return Promise.resolve(null)
      const forward = direction === 'next'
      return Promise.resolve(findPattern(resolved, summary.info.sampleCount, sample, forward))
    },

    async decodeSet(captureId: number, decoders: unknown[]) {
      if (!summary || summary.id !== captureId)
        return Promise.reject(uiError('error.session.staleCapture'))
      if (live) return Promise.reject(uiError('error.session.busy'))
      summary = { ...summary, decoders: decoders.length > 0 ? decoders : null }
      const id = summary.id
      setTimeout(() => {
        for (const { index } of mockStatuses(summary?.decoders, false)) {
          for (const handler of decodedHandlers) handler({ captureId: id, index })
        }
      }, 50)
      return Promise.resolve(summary)
    },

    async deviceList(): Promise<DeviceSummary[]> {
      return Promise.resolve(MOCK_DEVICES)
    },

    async deviceOpen(id: string): Promise<DeviceState> {
      if (live) return Promise.reject(uiError('error.session.busy'))
      if (id !== DEMO_ID) return Promise.reject(uiError('error.device.unsupported'))
      device = new MockDemoDevice(data.samplerate, data.sampleCount)
      return Promise.resolve(device.state())
    },

    async deviceState() {
      return Promise.resolve(device?.state() ?? null)
    },

    async deviceSetOption(key: OptionKey, value: OptionValue) {
      if (!device) return Promise.reject(uiError('error.session.noDevice'))
      device.setOption(key, value)
      return Promise.resolve(device.state())
    },

    async deviceSetChannel(index: number, enabled: boolean) {
      if (!device) return Promise.reject(uiError('error.session.noDevice'))
      device.setChannel(index, enabled)
      return Promise.resolve(device.state())
    },

    async captureStart(request: CaptureRequest): Promise<CaptureSummary> {
      if (!device) return Promise.reject(uiError('error.session.noDevice'))
      if (live) return Promise.reject(uiError('error.session.busy'))
      const enabled = device.state().enabledChannels
      if (enabled.length === 0) return Promise.reject(uiError('error.device.invalidRequest'))
      const limit = Math.ceil(device.limit / 1024) * 1024
      const total = Math.min(limit, data.sampleCount)
      if (request.advanced && !device.state().trigger.advanced) {
        return Promise.reject(uiError('error.device.invalidRequest'))
      }
      const waits =
        !request.instant &&
        (request.advanced !== null ||
          request.triggers.some(([, condition]) => condition !== 'none'))
      lastId += 1
      device.capturing = true
      const names = new Map(data.channels.map(({ index, name }) => [index, name]))
      summary = {
        id: lastId,
        source: { kind: 'device', driver: 'virtual-demo', model: 'Demo' },
        info: {
          samplerate: device.samplerate,
          sampleCount: 0,
          triggerSample: null,
          triggerTimeMs: null,
          channels: enabled,
        },
        channels: enabled.map((index) => ({ index, name: names.get(index) ?? String(index) })),
        decoders: data.decoders,
      }
      setLength(0)
      live = {
        status: {
          captureId: lastId,
          phase: waits ? 'waiting' : 'capturing',
          samples: 0,
          limit,
          triggerSample: null,
          hardwareTriggered: false,
          hardwareSamples: null,
          overflow: false,
          end: null,
          error: null,
        },
        timers: [],
      }
      const timers = live.timers
      const begin = waits ? TRIGGER_WAIT_MS : 0
      if (waits) {
        const pre = Math.round((total * request.triggerPosition) / 100)
        timers.push(
          setTimeout(() => emit({ hardwareSamples: Math.round(pre / 2) }), TRIGGER_WAIT_MS / 2),
        )
        timers.push(
          setTimeout(() => {
            const trigger = data.triggerSample ?? pre
            if (summary) {
              summary = {
                ...summary,
                info: { ...summary.info, triggerSample: trigger, triggerTimeMs: Date.now() },
              }
            }
            emit({ phase: 'capturing', triggerSample: trigger })
          }, TRIGGER_WAIT_MS),
        )
      }
      for (let step = 1; step <= LIVE_STEPS; step++) {
        timers.push(
          setTimeout(
            () => {
              const samples = Math.round((total * step) / LIVE_STEPS)
              setLength(samples)
              emit({ phase: 'capturing', samples })
              if (step === LIVE_STEPS) finish('completed')
            },
            begin + step * LIVE_STEP_MS,
          ),
        )
      }
      queueMicrotask(() => emit({}))
      return Promise.resolve(summary)
    },

    async captureStop() {
      finish('stopped')
      return Promise.resolve()
    },

    onLive(handler) {
      return subscribe(liveHandlers, handler)
    },

    onDevices() {
      return Promise.resolve<Unsubscribe>(() => undefined)
    },

    onDeviceState() {
      return Promise.resolve<Unsubscribe>(() => undefined)
    },

    onDecoded(handler) {
      return subscribe(decodedHandlers, handler)
    },
  }
}
