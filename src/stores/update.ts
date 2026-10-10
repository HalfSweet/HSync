import { defineStore } from 'pinia'
import { shallowRef } from 'vue'

import { getBackend, type AvailableUpdate, type UpdateProgress } from '@/ipc/backend'
import { toUiError, type UiError } from '@/ipc/errors'
import { usePreferencesStore } from '@/stores/preferences'

/**
 * Where updating stands: not asked yet, asking, this is the latest release, a newer one is out
 * (`available`), downloading it (`progress`), restarting into it, or the last step failed
 * (`error`; `available` still names the release when installing it failed).
 */
export type UpdateState =
  'idle' | 'checking' | 'latest' | 'available' | 'downloading' | 'restarting' | 'failed'

/** Updates of the app itself (DESIGN.md §9.11), from the GitHub releases. */
export const useUpdateStore = defineStore('update', () => {
  const preferences = usePreferencesStore()
  const state = shallowRef<UpdateState>('idle')
  const available = shallowRef<AvailableUpdate | null>(null)
  const progress = shallowRef<UpdateProgress | null>(null)
  const error = shallowRef<UiError | null>(null)

  const busy = () => ['checking', 'downloading', 'restarting'].includes(state.value)

  /** Asks for a newer release. A quiet check, the one at startup, only logs a failure. */
  async function check(quiet = false): Promise<void> {
    if (busy()) return
    state.value = 'checking'
    error.value = null
    try {
      available.value = await (await getBackend()).updateCheck()
      state.value = available.value ? 'available' : 'latest'
    } catch (caught) {
      if (quiet) {
        console.warn('update check failed', caught)
        state.value = 'idle'
      } else {
        error.value = toUiError(caught)
        state.value = 'failed'
      }
    }
  }

  /** Downloads and installs the release found; the app then restarts into it. */
  async function install(): Promise<void> {
    if (!available.value || busy()) return
    state.value = 'downloading'
    progress.value = { done: 0, total: null }
    error.value = null
    try {
      await (await getBackend()).updateInstall((next) => (progress.value = next))
      state.value = 'restarting'
    } catch (caught) {
      error.value = toUiError(caught)
      state.value = 'failed'
    }
  }

  /** The check at startup, unless the settings turn it off; development builds never check. */
  function init(): Promise<void> {
    return preferences.checkUpdates && !import.meta.env.DEV ? check(true) : Promise.resolve()
  }

  return { state, available, progress, error, check, install, init }
})
