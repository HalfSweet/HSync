import { defineStore } from 'pinia'
import { computed, shallowRef } from 'vue'

import { getBackend } from '@/ipc/backend'
import type { CaptureRequest } from '@/ipc/bindings/CaptureRequest'
import type { CaptureSummary } from '@/ipc/bindings/CaptureSummary'
import type { ExportRequest } from '@/ipc/bindings/ExportRequest'
import type { LiveStatus } from '@/ipc/bindings/LiveStatus'
import { toUiError, type UiError } from '@/ipc/errors'
import { groupChannels, type ChannelGroup } from '@/timeline/groups'

export type CaptureStatus = 'idle' | 'loading' | 'ready' | 'error'
/** Single captures, or another one a moment after each completes (DESIGN.md §9.4). */
export type CaptureMode = 'single' | 'repeat'

/** A save or export under way and the share of it done. */
export interface FileJob {
  kind: 'save' | 'export'
  progress: number
}

/** Pause between repeated captures, DSView's default interval. */
const REPEAT_INTERVAL_MS = 1000

/** The open capture: what it is, and how to place its samples in time. */
export const useCaptureStore = defineStore('capture', () => {
  const summary = shallowRef<CaptureSummary | null>(null)
  const status = shallowRef<CaptureStatus>('idle')
  const error = shallowRef<UiError | null>(null)
  /** The latest state of a capture taken with a device, kept after it ends. */
  const live = shallowRef<LiveStatus | null>(null)
  const mode = shallowRef<CaptureMode>('single')
  /** A repeated capture is due. */
  const repeatPending = shallowRef(false)
  /** Where the open capture was last saved or exported, until another capture replaces it. */
  const savedPath = shallowRef<string | null>(null)
  const savedKind = shallowRef<FileJob['kind']>('save')
  /** The save or export under way. */
  const job = shallowRef<FileJob | null>(null)
  let lastRequest: CaptureRequest | null = null
  let repeatTimer: ReturnType<typeof setTimeout> | null = null

  const samplerate = computed(() => summary.value?.info.samplerate ?? 0)
  const sampleCount = computed(() => summary.value?.info.sampleCount ?? 0)
  /** Whether a capture of the open capture's device is still running. */
  const capturing = computed(
    () =>
      live.value !== null &&
      live.value.phase !== 'ended' &&
      live.value.captureId === summary.value?.id,
  )
  /** A capture runs or is about to repeat: the capture button stops it. */
  const busy = computed(() => capturing.value || repeatPending.value)
  /** Samples the timeline spans: the capture, or all it will hold while it fills. */
  const extent = computed(() =>
    capturing.value && live.value
      ? Math.max(sampleCount.value, live.value.limit)
      : sampleCount.value,
  )
  /** Time zero of the timeline: the trigger, or the first sample without one. */
  const triggerSample = computed(() => summary.value?.info.triggerSample ?? 0)
  const triggered = computed(() => summary.value?.info.triggerSample != null)
  const duration = computed(() => (samplerate.value > 0 ? sampleCount.value / samplerate.value : 0))
  const channelNames = computed(
    () => new Map(summary.value?.channels.map((channel) => [channel.index, channel.name]) ?? []),
  )
  /** Channels grouped by decoder, in timeline order. */
  const groups = computed<ChannelGroup[]>(() =>
    summary.value ? groupChannels(summary.value.channels, summary.value.decoders) : [],
  )

  /** How the timeline labels a channel: its decoder role (SDA, RX…) or its own name. */
  function channelLabel(index: number): string {
    for (const group of groups.value) {
      const channel = group.channels.find((candidate) => candidate.index === index)
      if (channel) return channel.label
    }
    return channelNames.value.get(index) ?? `D${index}`
  }

  /** Seconds from the trigger to `sample`. */
  function timeOf(sample: number): number {
    return samplerate.value > 0 ? (sample - triggerSample.value) / samplerate.value : 0
  }

  /** Sample at `seconds` from the trigger. */
  function sampleAt(seconds: number): number {
    return triggerSample.value + seconds * samplerate.value
  }

  async function run(load: () => Promise<CaptureSummary | null>): Promise<void> {
    status.value = 'loading'
    error.value = null
    savedPath.value = null
    try {
      const loaded = await load()
      summary.value = loaded
      status.value = loaded ? 'ready' : 'idle'
    } catch (reason) {
      error.value = toUiError(reason)
      status.value = 'error'
    }
  }

  /** Shows the capture the backend already holds, or opens the demo capture. */
  async function init(): Promise<void> {
    const backend = await getBackend()
    await backend.onLive(receive)
    return run(async () => (await backend.captureSummary()) ?? (await backend.openDemo()))
  }

  /** Applies a status of a running capture to its summary. */
  function receive(next: LiveStatus): void {
    live.value = next
    const current = summary.value
    if (!current || current.id !== next.captureId) return
    summary.value = {
      ...current,
      info: {
        ...current.info,
        sampleCount: next.samples,
        triggerSample: next.triggerSample ?? current.info.triggerSample,
      },
    }
    if (next.phase === 'ended') {
      error.value = next.error
      void refreshSummary(next.captureId)
      if (next.end === 'completed' && mode.value === 'repeat' && lastRequest) {
        scheduleRepeat(lastRequest)
      }
    }
  }

  function scheduleRepeat(request: CaptureRequest): void {
    cancelRepeat()
    repeatPending.value = true
    repeatTimer = setTimeout(() => {
      repeatTimer = null
      repeatPending.value = false
      void start(request)
    }, REPEAT_INTERVAL_MS)
  }

  function cancelRepeat(): void {
    if (repeatTimer !== null) clearTimeout(repeatTimer)
    repeatTimer = null
    repeatPending.value = false
  }

  /** Takes the final summary of an ended capture, with its trigger time. */
  async function refreshSummary(id: number): Promise<void> {
    const latest = await (await getBackend()).captureSummary()
    if (latest && latest.id === id && summary.value?.id === id) summary.value = latest
  }

  /** Starts a capture on the open device. */
  async function start(request: CaptureRequest): Promise<void> {
    error.value = null
    savedPath.value = null
    lastRequest = request
    try {
      const started = await (await getBackend()).captureStart(request)
      summary.value = started
      status.value = 'ready'
      // A status may have arrived before the summary did.
      if (live.value?.captureId === started.id) receive(live.value)
    } catch (reason) {
      error.value = toUiError(reason)
    }
  }

  /** Takes a new summary of the open capture, such as one with other decoders. */
  function replaceSummary(next: CaptureSummary): void {
    if (summary.value?.id === next.id) summary.value = next
  }

  /** Stops the running capture, or the next repeat; what was captured so far stays. */
  async function stop(): Promise<void> {
    cancelRepeat()
    if (!capturing.value) return
    try {
      await (await getBackend()).captureStop()
    } catch (reason) {
      error.value = toUiError(reason)
    }
  }

  function openDemo(): Promise<void> {
    return run(async () => (await getBackend()).openDemo())
  }

  function openFile(path: string): Promise<void> {
    return run(async () => (await getBackend()).openFile(path))
  }

  /** Runs a save or export, following its progress; a cancelled one is no error. */
  async function fileJob(
    kind: FileJob['kind'],
    path: string,
    work: (id: number, onProgress: (done: number) => void) => Promise<void>,
  ): Promise<boolean> {
    const current = summary.value
    if (!current || job.value) return false
    error.value = null
    savedPath.value = null
    job.value = { kind, progress: 0 }
    try {
      await work(current.id, (done) => {
        if (job.value) job.value = { kind, progress: done }
      })
      savedPath.value = path
      savedKind.value = kind
      return true
    } catch (reason) {
      const failure = toUiError(reason)
      if (failure.code !== 'error.file.cancelled') error.value = failure
      return false
    } finally {
      job.value = null
    }
  }

  /** Writes the open capture to `path` as a `.dsl` file; it then counts as that file. */
  function saveFile(path: string): Promise<boolean> {
    return fileJob('save', path, async (id, onProgress) => {
      summary.value = await (await getBackend()).saveFile(id, path, onProgress)
    })
  }

  /** Writes the open capture, or part of it, to `path` in another format. */
  function exportFile(path: string, request: ExportRequest): Promise<boolean> {
    return fileJob('export', path, async (id, onProgress) => {
      await (await getBackend()).exportFile(id, path, request, onProgress)
    })
  }

  /** Writes text the UI composed to `path`, as an export. */
  function saveText(path: string, text: string): Promise<boolean> {
    return fileJob('export', path, async () => {
      await (await getBackend()).saveText(path, text)
    })
  }

  /** Stops the save or export under way. */
  async function cancelFile(): Promise<void> {
    if (job.value) await (await getBackend()).cancelFile()
  }

  return {
    summary,
    status,
    error,
    live,
    mode,
    repeatPending,
    savedPath,
    savedKind,
    job,
    capturing,
    busy,
    extent,
    samplerate,
    sampleCount,
    triggerSample,
    triggered,
    duration,
    channelNames,
    groups,
    channelLabel,
    timeOf,
    sampleAt,
    init,
    openDemo,
    openFile,
    saveFile,
    exportFile,
    saveText,
    cancelFile,
    start,
    stop,
    replaceSummary,
  }
})
