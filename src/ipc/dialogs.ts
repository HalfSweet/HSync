/**
 * Native file dialogs through Tauri's dialog plugin. Outside the desktop shell there is no file
 * system to pick from, so they report a cancelled dialog.
 */
import { isTauri } from '@tauri-apps/api/core'

export interface FileFilter {
  name: string
  extensions: string[]
}

/** Asks for a file to open; `null` when cancelled. */
export async function pickFile(filters: FileFilter[]): Promise<string | null> {
  if (!isTauri()) return null
  const { open } = await import('@tauri-apps/plugin-dialog')
  const picked = await open({ multiple: false, directory: false, filters })
  return typeof picked === 'string' ? picked : null
}

/** Asks where to save a file; `null` when cancelled. */
export async function pickSavePath(
  defaultPath: string,
  filters: FileFilter[],
): Promise<string | null> {
  if (!isTauri()) return null
  const { save } = await import('@tauri-apps/plugin-dialog')
  return save({ defaultPath, filters })
}
