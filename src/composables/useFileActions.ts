import { useI18n } from 'vue-i18n'

import type { ExportKind } from '@/ipc/bindings/ExportKind'
import type { ExportRequest } from '@/ipc/bindings/ExportRequest'
import { pickFile, pickSavePath } from '@/ipc/dialogs'
import { useCaptureStore } from '@/stores/capture'

/** File name extension of each export format. */
export const EXTENSIONS: Record<ExportKind, string> = {
  dsl: 'dsl',
  csv: 'csv',
  vcd: 'vcd',
  sigrok: 'sr',
  binary: 'bin',
}

/** A file name for a new capture: `HSync-20261010-142530.dsl`. */
export function defaultCaptureName(now = new Date(), extension = 'dsl'): string {
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
  const filters = () => [{ name: t('file.filter'), extensions: ['dsl'] }]
  const idle = () => !capture.busy && capture.job === null

  async function open(): Promise<void> {
    if (!idle()) return
    const path = await pickFile(filters())
    if (path) await capture.openFile(path)
  }

  async function save(): Promise<void> {
    if (!capture.summary || !idle()) return
    const source = capture.summary.source
    const suggested = source.kind === 'file' ? source.path : defaultCaptureName()
    const path = await pickSavePath(suggested, filters())
    if (path) await capture.saveFile(withExtension(path, 'dsl'))
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

  return { open, save, exportTo }
}
