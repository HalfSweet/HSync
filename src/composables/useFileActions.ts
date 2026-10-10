import { useI18n } from 'vue-i18n'

import { pickFile, pickSavePath } from '@/ipc/dialogs'
import { useCaptureStore } from '@/stores/capture'

/** A file name for a new capture: `HSync-20261010-142530.dsl`. */
export function defaultCaptureName(now = new Date()): string {
  const pad = (value: number) => String(value).padStart(2, '0')
  const date = `${now.getFullYear()}${pad(now.getMonth() + 1)}${pad(now.getDate())}`
  const time = `${pad(now.getHours())}${pad(now.getMinutes())}${pad(now.getSeconds())}`
  return `HSync-${date}-${time}.dsl`
}

/** Opening and saving `.dsl` captures through the native dialogs. */
export function useFileActions() {
  const { t } = useI18n()
  const capture = useCaptureStore()
  const filters = () => [{ name: t('file.filter'), extensions: ['dsl'] }]

  async function open(): Promise<void> {
    if (capture.busy) return
    const path = await pickFile(filters())
    if (path) await capture.openFile(path)
  }

  async function save(): Promise<void> {
    if (!capture.summary || capture.busy) return
    const source = capture.summary.source
    const suggested = source.kind === 'file' ? source.path : defaultCaptureName()
    const path = await pickSavePath(suggested, filters())
    if (path) await capture.saveFile(path.endsWith('.dsl') ? path : `${path}.dsl`)
  }

  return { open, save }
}
