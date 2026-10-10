import { watch, watchEffect } from 'vue'

import { setLocale } from '@/i18n'
import { getBackend } from '@/ipc/backend'
import { usePreferencesStore } from '@/stores/preferences'

/** Reflects theme, density and language preferences on the document root, and hands the
 * diagnostics settings to the backend. */
export function useRootPreferences(): void {
  const preferences = usePreferencesStore()

  watchEffect(() => {
    const root = document.documentElement
    root.classList.toggle('theme-night', preferences.resolvedTheme === 'night')
    root.classList.toggle('density-compact', preferences.density === 'compact')
  })

  watchEffect(() => setLocale(preferences.resolvedLocale))

  watch(
    () => [preferences.logLevel, preferences.logToFile] as const,
    async ([level, file]) => {
      try {
        await (await getBackend()).configureLog(level, file)
      } catch (error) {
        console.error('log settings failed', error)
      }
    },
    { immediate: true },
  )
}
