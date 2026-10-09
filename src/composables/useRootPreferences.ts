import { watchEffect } from 'vue'

import { setLocale } from '@/i18n'
import { usePreferencesStore } from '@/stores/preferences'

/** Reflects theme, density and language preferences on the document root. */
export function useRootPreferences(): void {
  const preferences = usePreferencesStore()

  watchEffect(() => {
    const root = document.documentElement
    root.classList.toggle('theme-night', preferences.resolvedTheme === 'night')
    root.classList.toggle('density-compact', preferences.density === 'compact')
  })

  watchEffect(() => setLocale(preferences.resolvedLocale))
}
