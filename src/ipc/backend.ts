import { getVersion } from '@tauri-apps/api/app'
import { Channel, invoke, isTauri } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { relaunch } from '@tauri-apps/plugin-process'
import { check, type Update } from '@tauri-apps/plugin-updater'

import type { BinsRequest } from './bindings/BinsRequest'
import type { CaptureRequest } from './bindings/CaptureRequest'
import type { CaptureSummary } from './bindings/CaptureSummary'
import type { ChannelEdges } from './bindings/ChannelEdges'
import type { CatalogEntry } from './bindings/CatalogEntry'
import type { DecoderLanes } from './bindings/DecoderLanes'
import type { DecoderStatus } from './bindings/DecoderStatus'
import type { DeviceState } from './bindings/DeviceState'
import type { DeviceSummary } from './bindings/DeviceSummary'
import type { Direction } from './bindings/Direction'
import type { ExportRequest } from './bindings/ExportRequest'
import type { ImportRequest } from './bindings/ImportRequest'
import type { LiveStatus } from './bindings/LiveStatus'
import type { ModuleInfo } from './bindings/ModuleInfo'
import type { OptionKey } from './bindings/OptionKey'
import type { OptionValue } from './bindings/OptionValue'
import type { PatternCondition } from './bindings/PatternCondition'
import type { PulseInfo } from './bindings/PulseInfo'
import type { RolePart } from './bindings/RolePart'
import type { SearchResult } from './bindings/SearchResult'
import type { StreamPage } from './bindings/StreamPage'
import type { WaveformSummary } from './bindings/WaveformSummary'
import { decodeAnalogBins, type AnalogBinsResult } from './analogBins'
import { decodeBins, type BinsResult } from './bins'
import type { UiError } from './errors'
import type { LogLevel } from './logLevels'

/** Stops a subscription. */
export type Unsubscribe = () => void

/** A page of a stream's listing in the data stream. */
export interface StreamRequest {
  captureId: number
  /** Position of the decoder in the capture's decoder list. */
  index: number
  /** Position of the stream among the decoder's. */
  stream: number
  /** The marked segment, to count what lies inside it. */
  segment: [number, number] | null
  /** List only rows overlapping the segment. */
  onlySegment: boolean
  offset: number
  count: number
}

/** A decoder of capture `captureId` finished, failed, or has more to show. */
export interface DecodedEvent {
  captureId: number
  index: number
}

/** A newer release than the one running, as the update server announces it. */
export interface AvailableUpdate {
  version: string
}

/** How much of an update has downloaded, and of how much when the server says. */
export interface UpdateProgress {
  done: number
  total: number | null
}

/** Everything the UI asks of the Rust side. Components reach it only through stores. */
export interface Backend {
  captureSummary(): Promise<CaptureSummary | null>
  /** Opens a capture file: HSync's own `.hsync`, or DSView's `.dsl`. */
  openFile(path: string): Promise<CaptureSummary>
  /**
   * Saves a capture as an `.hsync` file with `session`, the interface's session document; it
   * then counts as that file.
   */
  saveFile(
    captureId: number,
    path: string,
    session: unknown,
    onProgress: (done: number) => void,
  ): Promise<CaptureSummary>
  /** Writes a capture, or part of it, in another format. */
  exportFile(
    captureId: number,
    path: string,
    request: ExportRequest,
    onProgress: (done: number) => void,
  ): Promise<void>
  /** Stops the save, export or import under way; it fails as cancelled. */
  cancelFile(): Promise<void>
  /**
   * Reads a waveform file (VCD, FST or GHW) and lists what it holds; the backend keeps the
   * file for {@link waveformImport} until then or {@link waveformDiscard}. Reading cannot be
   * cancelled.
   */
  waveformInspect(path: string, onProgress: (done: number) => void): Promise<WaveformSummary>
  /** Lays the chosen signals of the waveform read last onto a capture, which becomes the open one. */
  waveformImport(
    request: ImportRequest,
    onProgress: (done: number) => void,
  ): Promise<CaptureSummary>
  /** Forgets the waveform read last. */
  waveformDiscard(): Promise<void>
  /** Writes text the UI composed, such as the data stream as listed. */
  saveText(path: string, text: string): Promise<void>
  /** Reads a small text file, such as a session. */
  readText(path: string): Promise<string>
  bins(request: BinsRequest): Promise<BinsResult>
  /** The lowest and highest volts of each column of analog channels. */
  analogBins(request: BinsRequest): Promise<AnalogBinsResult>
  pulse(captureId: number, channel: number, sample: number): Promise<PulseInfo | null>
  edge(
    captureId: number,
    channel: number,
    sample: number,
    direction: Direction,
  ): Promise<number | null>
  /** Median samples between rising edges: the period of the clock a channel carries. */
  clockPeriod(captureId: number, channel: number): Promise<number | null>
  /** Renames a channel of a capture; an empty name gives back its number. */
  renameChannel(captureId: number, channel: number, name: string): Promise<CaptureSummary>
  /** Rising and falling edges of channels from sample `from` to sample `to`. */
  edgeCounts(
    captureId: number,
    from: number,
    to: number,
    channels: number[],
  ): Promise<ChannelEdges[]>
  /** The nearest place from `sample` on (or back from it) where a level pattern holds. */
  find(
    captureId: number,
    conditions: PatternCondition[],
    sample: number,
    direction: Direction,
  ): Promise<number | null>

  /** Scans for devices. */
  deviceList(): Promise<DeviceSummary[]>
  /** Opens a device; one that needs firmware gets it first, which takes a few seconds. */
  deviceOpen(id: string): Promise<DeviceState>
  deviceState(): Promise<DeviceState | null>
  deviceSetOption(key: OptionKey, value: OptionValue): Promise<DeviceState>
  deviceSetChannel(channel: number, enabled: boolean): Promise<DeviceState>
  /** Enables exactly `channels`. */
  deviceSetChannels(channels: number[]): Promise<DeviceState>
  /** Moves to rate tier `index` of the open device with exactly `channels` enabled. */
  deviceSelectRateTier(index: number, channels: number[]): Promise<DeviceState>
  /** Starts a capture on the open device; the returned capture fills as it runs. */
  captureStart(request: CaptureRequest): Promise<CaptureSummary>
  captureStop(): Promise<void>

  /** Every decoder there is, built in or loaded, with its configuration fields and event kinds. */
  decodeCatalog(): Promise<CatalogEntry[]>
  /** The decoder modules loaded, in the order they were. */
  decodeModules(): Promise<ModuleInfo[]>
  /** Loads a decoder module (a decoder written in the DSL, as its IR file) under its name, in
   * place of a decoder of that name built in; the capture's decoders naming it decode again. */
  decodeLoadModule(path: string): Promise<ModuleInfo>
  /** Forgets a decoder module; the decoder built in takes over. Whether there was one. */
  decodeUnloadModule(name: string): Promise<boolean>

  /** Which messages the app logs, and whether to the log file as well. */
  configureLog(level: LogLevel, file: boolean): Promise<void>
  /** Where the log file is, or `null` without one (the browser preview). */
  logFile(): Promise<string | null>
  /** Shows the log file in the system's file manager. */
  revealLog(): Promise<void>
  /** Titles the window. */
  setTitle(title: string): Promise<void>
  /** Physical memory in bytes, or `null` when unknown. */
  systemMemory(): Promise<number | null>
  /** The app's version. */
  appVersion(): Promise<string>
  /** Asks the update server for a newer release; `null` when this one is the latest. */
  updateCheck(): Promise<AvailableUpdate | null>
  /** Downloads and installs the release the last check found, then restarts the app. */
  updateInstall(onProgress: (progress: UpdateProgress) => void): Promise<void>
  /** The decoders of a capture, their streams and how far they are. */
  decodeList(captureId: number): Promise<DecoderStatus[]>
  /** The lanes of every decoder's streams over `start..end`. */
  decodeView(
    captureId: number,
    start: number,
    end: number,
    samplesPerPixel: number,
  ): Promise<DecoderLanes[]>
  decodeStream(request: StreamRequest): Promise<StreamPage | null>
  /** Replaces the capture's decoders (its entries) and decodes again, while capturing too. */
  decodeSet(captureId: number, decoders: unknown[]): Promise<CaptureSummary>
  /** Position in a stream's listing of the first event reaching past `sample`. */
  decodeRowAt(
    captureId: number,
    index: number,
    stream: number,
    sample: number,
  ): Promise<number | null>
  /** Events matching `query` in every decoder's streams; `null` for a blank query. */
  decodeSearch(captureId: number, query: string): Promise<SearchResult | null>
  /** Where decoder `index` has events within `start..end`, in columns of `resolution` samples
   * each in the role covering most of it, errors last; `null` for a decoder that is not there. */
  decodeCoverage(
    captureId: number,
    index: number,
    start: number,
    end: number,
    resolution: number,
  ): Promise<RolePart[] | null>

  /** Progress and end of captures taken with a device. */
  onLive(handler: (status: LiveStatus) => void): Promise<Unsubscribe>
  /** The device list after devices came or went. */
  onDevices(handler: (devices: DeviceSummary[]) => void): Promise<Unsubscribe>
  /** The open device, when it changed by itself (it was unplugged). */
  onDeviceState(handler: (state: DeviceState | null) => void): Promise<Unsubscribe>
  /** A decoder finished or failed. */
  onDecoded(handler: (event: DecodedEvent) => void): Promise<Unsubscribe>
  /** A decoder has more to show. */
  onDecodeProgress(handler: (event: DecodedEvent) => void): Promise<Unsubscribe>
  /** Decoder modules came with a capture file: the modules and the catalog changed. */
  onModulesChanged(handler: () => void): Promise<Unsubscribe>
}

/** A channel the backend reports the share done through. */
function progress(handler: (done: number) => void): Channel<number> {
  const channel = new Channel<number>()
  channel.onmessage = handler
  return channel
}

/** An updater failure carrying a {@link UiError}; the plugin reports them as English text. */
function updateError(code: string, error: unknown): Error & UiError {
  const detail = error instanceof Error ? error.message : String(error)
  return Object.assign(new Error(detail), { code, params: { detail } })
}

class TauriBackend implements Backend {
  /** The release the last update check found. */
  private update: Update | null = null

  captureSummary() {
    return invoke<CaptureSummary | null>('capture_summary')
  }

  openFile(path: string) {
    return invoke<CaptureSummary>('capture_open_file', { path })
  }

  saveFile(captureId: number, path: string, session: unknown, onProgress: (done: number) => void) {
    return invoke<CaptureSummary>('capture_save', {
      captureId,
      path,
      session,
      onProgress: progress(onProgress),
    })
  }

  exportFile(
    captureId: number,
    path: string,
    request: ExportRequest,
    onProgress: (done: number) => void,
  ) {
    return invoke<void>('capture_export', {
      captureId,
      path,
      request,
      onProgress: progress(onProgress),
    })
  }

  cancelFile() {
    return invoke<void>('file_cancel')
  }

  waveformInspect(path: string, onProgress: (done: number) => void) {
    return invoke<WaveformSummary>('waveform_inspect', {
      path,
      onProgress: progress(onProgress),
    })
  }

  waveformImport(request: ImportRequest, onProgress: (done: number) => void) {
    return invoke<CaptureSummary>('waveform_import', {
      request,
      onProgress: progress(onProgress),
    })
  }

  waveformDiscard() {
    return invoke<void>('waveform_discard')
  }

  saveText(path: string, text: string) {
    return invoke<void>('text_save', { path, text })
  }

  readText(path: string) {
    return invoke<string>('text_read', { path })
  }

  async bins(request: BinsRequest) {
    return decodeBins(await invoke<ArrayBuffer>('capture_bins', { request }))
  }

  async analogBins(request: BinsRequest) {
    return decodeAnalogBins(await invoke<ArrayBuffer>('capture_analog_bins', { request }))
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

  renameChannel(captureId: number, channel: number, name: string) {
    return invoke<CaptureSummary>('capture_rename_channel', { captureId, channel, name })
  }

  edgeCounts(captureId: number, from: number, to: number, channels: number[]) {
    return invoke<ChannelEdges[]>('capture_edges', { captureId, from, to, channels })
  }

  find(captureId: number, conditions: PatternCondition[], sample: number, direction: Direction) {
    return invoke<number | null>('capture_find', { captureId, conditions, sample, direction })
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

  deviceSetChannels(channels: number[]) {
    return invoke<DeviceState>('device_set_channels', { channels })
  }

  deviceSelectRateTier(index: number, channels: number[]) {
    return invoke<DeviceState>('device_select_rate_tier', { index, channels })
  }

  captureStart(request: CaptureRequest) {
    return invoke<CaptureSummary>('capture_start', { request })
  }

  captureStop() {
    return invoke<void>('capture_stop')
  }

  decodeCatalog() {
    return invoke<CatalogEntry[]>('decode_catalog')
  }

  decodeModules() {
    return invoke<ModuleInfo[]>('decode_modules')
  }

  decodeLoadModule(path: string) {
    return invoke<ModuleInfo>('decode_load_module', { path })
  }

  decodeUnloadModule(name: string) {
    return invoke<boolean>('decode_unload_module', { name })
  }

  configureLog(level: LogLevel, file: boolean) {
    return invoke<void>('log_configure', { level, file })
  }

  logFile() {
    return invoke<string>('log_file')
  }

  revealLog() {
    return invoke<void>('log_reveal')
  }

  setTitle(title: string) {
    return getCurrentWindow().setTitle(title)
  }

  systemMemory() {
    return invoke<number>('system_memory')
  }

  appVersion() {
    return getVersion()
  }

  async updateCheck() {
    let found: Update | null
    try {
      found = await check()
    } catch (error) {
      throw updateError('error.update.check', error)
    }
    await this.update?.close()
    this.update = found
    return found && { version: found.version }
  }

  async updateInstall(onProgress: (progress: UpdateProgress) => void) {
    const update = this.update
    if (!update) throw updateError('error.update.install', 'no update to install')
    const progress: UpdateProgress = { done: 0, total: null }
    try {
      // Windows quits here to run the installer, which starts the new version.
      await update.downloadAndInstall((event) => {
        if (event.event === 'Started') progress.total = event.data.contentLength ?? null
        else if (event.event === 'Progress') progress.done += event.data.chunkLength
        onProgress({ ...progress })
      })
    } catch (error) {
      throw updateError('error.update.install', error)
    }
    await relaunch()
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

  decodeRowAt(captureId: number, index: number, stream: number, sample: number) {
    return invoke<number | null>('decode_row_at', { captureId, index, stream, sample })
  }

  decodeSearch(captureId: number, query: string) {
    return invoke<SearchResult | null>('decode_search', { captureId, query })
  }

  decodeCoverage(captureId: number, index: number, start: number, end: number, resolution: number) {
    return invoke<RolePart[] | null>('decode_coverage', {
      captureId,
      index,
      start,
      end,
      // Overview pixels can span a fraction of a sample; the command takes a u64.
      resolution: Math.max(1, Math.ceil(resolution)),
    })
  }

  onLive(handler: (status: LiveStatus) => void) {
    return listen<LiveStatus>('capture://live', (event) => handler(event.payload))
  }

  onDecoded(handler: (event: DecodedEvent) => void) {
    return listen<DecodedEvent>('decode://done', (event) => handler(event.payload))
  }

  onDecodeProgress(handler: (event: DecodedEvent) => void) {
    return listen<DecodedEvent>('decode://progress', (event) => handler(event.payload))
  }

  onModulesChanged(handler: () => void) {
    return listen<null>('decode://modules', () => handler())
  }

  onDevices(handler: (devices: DeviceSummary[]) => void) {
    return listen<DeviceSummary[]>('device://list', (event) => handler(event.payload))
  }

  onDeviceState(handler: (state: DeviceState | null) => void) {
    return listen<DeviceState | null>('device://state', (event) => handler(event.payload))
  }
}

let backend: Promise<Backend> | undefined

/** A development window's mock, which a pop-up it opened uses too. */
interface MockHolder {
  hsyncMock?: Promise<Backend>
}

/** The mock of the window that opened this one: the data stream's pop-up shows its capture. */
function openerMock(): Promise<Backend> | undefined {
  try {
    return (window.opener as MockHolder | null)?.hsyncMock
  } catch {
    return undefined
  }
}

/**
 * The Tauri backend inside the app. Outside Tauri, development builds fall back to an
 * in-browser mock that replays the demo capture so the UI can be worked on in a browser.
 */
export function getBackend(): Promise<Backend> {
  if (backend) return backend
  if (isTauri()) backend = Promise.resolve(new TauriBackend())
  else if (import.meta.env.DEV) {
    backend =
      openerMock() ?? import('./mock/mockBackend').then((module) => module.createMockBackend())
    ;(window as MockHolder).hsyncMock = backend
  } else backend = Promise.reject(new Error('HSync must run inside its desktop shell'))
  return backend
}
