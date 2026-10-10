import { watch } from 'vue'

import { getBackend } from '@/ipc/backend'
import { useCaptureStore } from '@/stores/capture'

const APP = 'HSync'

/** The window title for a capture: the file's name when one is open, as DSView shows it. */
export function windowTitle(source: { kind: string; path?: string } | undefined): string {
  if (source?.kind !== 'file' || !source.path) return APP
  const name = source.path.split(/[\\/]/).pop() ?? source.path
  return `${name} — ${APP}`
}

/** Keeps the window title on the open capture. */
export function useWindowTitle(): void {
  const capture = useCaptureStore()
  watch(
    () => windowTitle(capture.summary?.source),
    async (title) => {
      try {
        await (await getBackend()).setTitle(title)
      } catch (error) {
        console.error('window title failed', error)
      }
    },
    { immediate: true },
  )
}
