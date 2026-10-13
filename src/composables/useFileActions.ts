import { useI18n } from 'vue-i18n'

import type { ExportKind } from '@/ipc/bindings/ExportKind'
import type { ExportRequest } from '@/ipc/bindings/ExportRequest'
import { pickFile, pickSavePath } from '@/ipc/dialogs'
import { SESSION_EXTENSION } from '@/session/session'
import { useCaptureStore } from '@/stores/capture'
import { useSessionStore } from '@/stores/session'

/** File name extension of each export format. */
export const EXTENSIONS: Record<ExportKind, string> = {
  hsync: 'hsync',
  dsl: 'dsl',
  csv: 'csv',
  vcd: 'vcd',
  fst: 'fst',
  sigrok: 'sr',
  binary: 'bin',
}

/** Extensions of the capture files HSync opens: its own, and DSView's. */
export const CAPTURE_EXTENSIONS = ['hsync', 'dsl'] as const

/** Extensions of the waveform files the import dialog reads. */
export const WAVEFORM_EXTENSIONS = ['vcd', 'fst', 'ghw'] as const

const WAVEFORM_SUFFIX = /\.(vcd|fst|ghw)$/i
const SOURCE_SUFFIX = /\.(hsync|dsl|vcd|fst|ghw)$/i

/** Whether `path` names a waveform file to import rather than a capture to open. */
export function isWaveformPath(path: string): boolean {
  return WAVEFORM_SUFFIX.test(path)
}

/** The `.hsync` name to suggest for the capture that came from `path`. */
export function suggestedCapturePath(path: string): string {
  return `${path.replace(SOURCE_SUFFIX, '')}.hsync`
}

/** A file name for a new capture: `HSync-20261010-142530.hsync`. */
export function defaultCaptureName(now = new Date(), extension = 'hsync'): string {
  const pad = (value: number) => String(value).padStart(2, '0')
  const date = `${now.getFullYear()}${pad(now.getMonth() + 1)}${pad(now.getDate())}`
  const time = `${pad(now.getHours())}${pad(now.getMinutes())}${pad(now.getSeconds())}`
  return `HSync-${date}-${time}.${extension}`
}

/** `path` with `extension` added unless it already ends with it. */
export function withExtension(path: string, extension: string): string {
  return path.toLowerCase().endsWith(`.${extension}`) ? path : `${path}.${extension}`
}

/** Opening, saving and exporting captures through the native dialogs. */
export function useFileActions() {
  const { t } = useI18n()
  const capture = useCaptureStore()
  const filters = () => [{ name: t('file.filter'), extensions: ['hsync'] }]
  const idle = () => !capture.busy && capture.job === null

  /**
   * Opens an `.hsync` or `.dsl` capture, or reads a waveform file for the import dialog. An
   * `.hsync` capture comes back with the colours and the view it was saved with.
   */
  async function open(): Promise<void> {
    if (!idle()) return
    const path = await pickFile([
      {
        name: t('file.filterAll'),
        extensions: [...CAPTURE_EXTENSIONS, ...WAVEFORM_EXTENSIONS],
      },
      ...filters(),
      { name: t('file.dslFilter'), extensions: ['dsl'] },
      { name: t('file.waveformFilter'), extensions: [...WAVEFORM_EXTENSIONS] },
    ])
    if (!path) return
    if (isWaveformPath(path)) {
      await capture.inspectWaveform(path)
      return
    }
    await capture.openFile(path)
    const opened = capture.summary
    if (opened?.source.kind === 'file' && opened.source.path === path && opened.session) {
      await useSessionStore().applyCaptureDocument(opened.session)
    }
  }

  /** Saves the open capture as an `.hsync` file, with the session as it is. */
  async function save(): Promise<void> {
    if (!capture.summary || !idle()) return
    const source = capture.summary.source
    const suggested =
      source.kind === 'file' ? suggestedCapturePath(source.path) : defaultCaptureName()
    const path = await pickSavePath(suggested, filters())
    if (!path) return
    await capture.saveFile(withExtension(path, 'hsync'), useSessionStore().captureDocument())
  }

  /** Asks where to write `request` and writes it there; `false` when cancelled or failed. */
  async function exportTo(request: ExportRequest): Promise<boolean> {
    if (!capture.summary || !idle()) return false
    const extension = EXTENSIONS[request.kind]
    const name = t(`exportDialog.formats.${request.kind}`)
    const path = await pickSavePath(defaultCaptureName(new Date(), extension), [
      { name, extensions: [extension] },
    ])
    if (!path) return false
    return capture.exportFile(withExtension(path, extension), request)
  }

  /** Saves the session: device settings, channels, triggers and decoders. */
  async function saveSession(): Promise<void> {
    if (!idle()) return
    const extension = SESSION_EXTENSION
    const path = await pickSavePath(defaultCaptureName(new Date(), extension), [
      { name: t('file.sessionFilter'), extensions: ['json'] },
    ])
    if (path) await useSessionStore().save(withExtension(path, extension))
  }

  /** Loads a session, HSync's own or DSView's `.dsc`. */
  async function loadSession(): Promise<void> {
    if (!idle()) return
    const path = await pickFile([
      { name: t('file.sessionFilter'), extensions: ['json'] },
      { name: t('file.dscFilter'), extensions: ['dsc'] },
    ])
    if (!path) return
    const failure = await useSessionStore().load(path)
    if (failure) capture.error = failure
  }

  return { open, save, exportTo, saveSession, loadSession }
}
