import { invoke, isTauri } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'

import type { BinsRequest } from './bindings/BinsRequest'
import type { CaptureRequest } from './bindings/CaptureRequest'
import type { CaptureSummary } from './bindings/CaptureSummary'
import type { DecoderLanes } from './bindings/DecoderLanes'
import type { DecoderStatus } from './bindings/DecoderStatus'
import type { DeviceState } from './bindings/DeviceState'
import type { DeviceSummary } from './bindings/DeviceSummary'
import type { Direction } from './bindings/Direction'
import type { LiveStatus } from './bindings/LiveStatus'
import type { OptionKey } from './bindings/OptionKey'
import type { OptionValue } from './bindings/OptionValue'
import type { PulseInfo } from './bindings/PulseInfo'
import type { StreamMode } from './bindings/StreamMode'
import type { StreamPage } from './bindings/StreamPage'
import { decodeBins, type BinsResult } from './bins'

/** Stops a subscription. */
export type Unsubscribe = () => void

/** A page of a decoder's data stream. */
export interface StreamRequest {
  captureId: number
  index: number
  mode: StreamMode
  /** The marked segment, to count what lies inside it. */
  segment: [number, number] | null
  /** List only rows overlapping the segment. */
  onlySegment: boolean
  offset: number
  count: number
}

/** A decoder of capture `captureId` finished. */
export interface DecodedEvent {
  captureId: number
  index: number
}

/** Everything the UI asks of the Rust side. Components reach it only through stores. */
export interface Backend {
  captureSummary(): Promise<CaptureSummary | null>
  openDemo(): Promise<CaptureSummary>
  openFile(path: string): Promise<CaptureSummary>
  /** Saves a capture as a `.dsl` file; it then counts as that file. */
  saveFile(captureId: number, path: string): Promise<CaptureSummary>
  bins(request: BinsRequest): Promise<BinsResult>
  pulse(captureId: number, channel: number, sample: number): Promise<PulseInfo | null>
  edge(
    captureId: number,
    channel: number,
    sample: number,
    direction: Direction,
  ): Promise<number | null>
  /** Median samples between rising edges: the period of the clock a channel carries. */
  clockPeriod(captureId: number, channel: number): Promise<number | null>

  /** Scans for devices. */
  deviceList(): Promise<DeviceSummary[]>
  /** Opens a device; one that needs firmware gets it first, which takes a few seconds. */
  deviceOpen(id: string): Promise<DeviceState>
  deviceState(): Promise<DeviceState | null>
  deviceSetOption(key: OptionKey, value: OptionValue): Promise<DeviceState>
  deviceSetChannel(channel: number, enabled: boolean): Promise<DeviceState>
  /** Starts a capture on the open device; the returned capture fills as it runs. */
  captureStart(request: CaptureRequest): Promise<CaptureSummary>
  captureStop(): Promise<void>

  /** The decoders of a capture and how far they are. */
  decodeList(captureId: number): Promise<DecoderStatus[]>
  /** Decode lanes of finished decoders over `start..end`. */
  decodeView(
    captureId: number,
    start: number,
    end: number,
    samplesPerPixel: number,
  ): Promise<DecoderLanes[]>
  decodeStream(request: StreamRequest): Promise<StreamPage | null>
  /** Replaces the capture's decoders (DSView's `decoders` list) and decodes again. */
  decodeSet(captureId: number, decoders: unknown[]): Promise<CaptureSummary>
  /** Position in a decoder's data stream of the first row ending after `sample`. */
  decodeRowAt(
    captureId: number,
    index: number,
    mode: StreamMode,
    sample: number,
  ): Promise<number | null>

  /** Progress and end of captures taken with a device. */
  onLive(handler: (status: LiveStatus) => void): Promise<Unsubscribe>
  /** The device list after devices came or went. */
  onDevices(handler: (devices: DeviceSummary[]) => void): Promise<Unsubscribe>
  /** The open device, when it changed by itself (it was unplugged). */
  onDeviceState(handler: (state: DeviceState | null) => void): Promise<Unsubscribe>
  /** A decoder finished. */
  onDecoded(handler: (event: DecodedEvent) => void): Promise<Unsubscribe>
}

class TauriBackend implements Backend {
  captureSummary() {
    return invoke<CaptureSummary | null>('capture_summary')
  }

  openDemo() {
    return invoke<CaptureSummary>('capture_open_demo')
  }

  openFile(path: string) {
    return invoke<CaptureSummary>('capture_open_file', { path })
  }

  saveFile(captureId: number, path: string) {
    return invoke<CaptureSummary>('capture_save', { captureId, path })
  }

  async bins(request: BinsRequest) {
    return decodeBins(await invoke<ArrayBuffer>('capture_bins', { request }))
  }

  pulse(captureId: number, channel: number, sample: number) {
    return invoke<PulseInfo | null>('capture_pulse', { captureId, channel, sample })
  }

  edge(captureId: number, channel: number, sample: number, direction: Direction) {
    return invoke<number | null>('capture_edge', { captureId, channel, sample, direction })
  }

  clockPeriod(captureId: number, channel: number) {
    return invoke<number | null>('capture_clock_period', { captureId, channel })
  }

  deviceList() {
    return invoke<DeviceSummary[]>('device_list')
  }

  deviceOpen(id: string) {
    return invoke<DeviceState>('device_open', { id })
  }

  deviceState() {
    return invoke<DeviceState | null>('device_state')
  }

  deviceSetOption(key: OptionKey, value: OptionValue) {
    return invoke<DeviceState>('device_set_option', { key, value })
  }

  deviceSetChannel(channel: number, enabled: boolean) {
    return invoke<DeviceState>('device_set_channel', { channel, enabled })
  }

  captureStart(request: CaptureRequest) {
    return invoke<CaptureSummary>('capture_start', { request })
  }

  captureStop() {
    return invoke<void>('capture_stop')
  }

  decodeList(captureId: number) {
    return invoke<DecoderStatus[]>('decode_list', { captureId })
  }

  decodeView(captureId: number, start: number, end: number, samplesPerPixel: number) {
    return invoke<DecoderLanes[]>('decode_view', { captureId, start, end, samplesPerPixel })
  }

  decodeStream(request: StreamRequest) {
    return invoke<StreamPage | null>('decode_stream', { ...request })
  }

  decodeSet(captureId: number, decoders: unknown[]) {
    return invoke<CaptureSummary>('decode_set', { captureId, decoders })
  }

  decodeRowAt(captureId: number, index: number, mode: StreamMode, sample: number) {
    return invoke<number | null>('decode_row_at', { captureId, index, mode, sample })
  }

  onLive(handler: (status: LiveStatus) => void) {
    return listen<LiveStatus>('capture://live', (event) => handler(event.payload))
  }

  onDecoded(handler: (event: DecodedEvent) => void) {
    return listen<DecodedEvent>('decode://done', (event) => handler(event.payload))
  }

  onDevices(handler: (devices: DeviceSummary[]) => void) {
    return listen<DeviceSummary[]>('device://list', (event) => handler(event.payload))
  }

  onDeviceState(handler: (state: DeviceState | null) => void) {
    return listen<DeviceState | null>('device://state', (event) => handler(event.payload))
  }
}

let backend: Promise<Backend> | undefined

/**
 * The Tauri backend inside the app. Outside Tauri, development builds fall back to an
 * in-browser mock that replays the demo capture so the UI can be worked on in a browser.
 */
export function getBackend(): Promise<Backend> {
  backend ??= isTauri()
    ? Promise.resolve(new TauriBackend())
    : import.meta.env.DEV
      ? import('./mock/mockBackend').then((module) => module.createMockBackend())
      : Promise.reject(new Error('HSync must run inside its desktop shell'))
  return backend
}
