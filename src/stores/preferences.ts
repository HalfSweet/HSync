import { usePreferredDark, useStorage } from '@vueuse/core'
import { defineStore } from 'pinia'
import { computed } from 'vue'

import { detectLocale, SUPPORTED_LOCALES, type AppLocale } from '@/i18n'

export const THEMES = ['system', 'paper', 'night'] as const
export type ThemePreference = (typeof THEMES)[number]
export type Theme = Exclude<ThemePreference, 'system'>

export const DENSITIES = ['comfortable', 'compact'] as const
export type Density = (typeof DENSITIES)[number]

export const LOCALE_PREFERENCES = ['system', ...SUPPORTED_LOCALES] as const
export type LocalePreference = (typeof LOCALE_PREFERENCES)[number]

function oneOf<T extends string>(allowed: readonly T[], value: string, fallback: T): T {
  return (allowed as readonly string[]).includes(value) ? (value as T) : fallback
}

/** Interface preferences, kept in the WebView's local storage (DESIGN.md §2.3). */
export const usePreferencesStore = defineStore('preferences', () => {
  const storedTheme = useStorage<string>('hsync.theme', 'system')
  const storedDensity = useStorage<string>('hsync.density', 'comfortable')
  const storedLocale = useStorage<string>('hsync.locale', 'system')
  const prefersDark = usePreferredDark()

  const theme = computed<ThemePreference>({
    get: () => oneOf(THEMES, storedTheme.value, 'system'),
    set: (value) => (storedTheme.value = value),
  })
  const density = computed<Density>({
    get: () => oneOf(DENSITIES, storedDensity.value, 'comfortable'),
    set: (value) => (storedDensity.value = value),
  })
  const locale = computed<LocalePreference>({
    get: () => oneOf(LOCALE_PREFERENCES, storedLocale.value, 'system'),
    set: (value) => (storedLocale.value = value),
  })

  const resolvedTheme = computed<Theme>(() =>
    theme.value === 'system' ? (prefersDark.value ? 'night' : 'paper') : theme.value,
  )
  const resolvedLocale = computed<AppLocale>(() =>
    locale.value === 'system' ? detectLocale(navigator.languages) : locale.value,
  )

  return { theme, density, locale, resolvedTheme, resolvedLocale }
})
