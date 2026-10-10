import { invoke, isTauri } from '@tauri-apps/api/core'

import type { BinsRequest } from './bindings/BinsRequest'
import type { CaptureSummary } from './bindings/CaptureSummary'
import type { Direction } from './bindings/Direction'
import type { PulseInfo } from './bindings/PulseInfo'
import { decodeBins, type BinsResult } from './bins'

/** Everything the UI asks of the Rust side. Components reach it only through stores. */
export interface Backend {
  captureSummary(): Promise<CaptureSummary | null>
  openDemo(): Promise<CaptureSummary>
  openFile(path: string): Promise<CaptureSummary>
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
