import { usePreferredDark, useStorage } from '@vueuse/core'
import { defineStore } from 'pinia'
import { computed, shallowRef } from 'vue'

import { detectLocale, SUPPORTED_LOCALES, type AppLocale } from '@/i18n'
import { LOG_LEVELS, type LogLevel } from '@/ipc/logLevels'

export const THEMES = ['system', 'light', 'dark'] as const
export type ThemePreference = (typeof THEMES)[number]
export type Theme = Exclude<ThemePreference, 'system'>

export const DENSITIES = ['comfortable', 'compact'] as const
export type Density = (typeof DENSITIES)[number]

/** Waveform drawing: WebGPU where it works (else Canvas2D), or Canvas2D always. */
export const GRAPHICS = ['auto', 'off'] as const
export type GraphicsPreference = (typeof GRAPHICS)[number]

/** Where the pulse and edge measurement read-outs go: the status bar, or over the waveforms. */
export const READOUT_PLACES = ['status', 'float'] as const
export type ReadoutPlace = (typeof READOUT_PLACES)[number]

export const LOCALE_PREFERENCES = ['system', ...SUPPORTED_LOCALES] as const
export type LocalePreference = (typeof LOCALE_PREFERENCES)[number]

function oneOf<T extends string>(allowed: readonly T[], value: string, fallback: T): T {
  return (allowed as readonly string[]).includes(value) ? (value as T) : fallback
}

/** Interface preferences, kept in the WebView's local storage (DESIGN.md §11). */
export const usePreferencesStore = defineStore('preferences', () => {
  const storedTheme = useStorage<string>('hsync.theme', 'system')
  const storedDensity = useStorage<string>('hsync.density', 'comfortable')
  const storedLocale = useStorage<string>('hsync.locale', 'system')
  const storedGraphics = useStorage<string>('hsync.graphics', 'auto')
  const storedReadoutPlace = useStorage<string>('hsync.readoutPlace', 'status')
  /** The renderer the timeline ended up with. */
  const activeRenderer = shallowRef<'webgpu' | 'canvas2d' | null>(null)

  // Behaviour, DSView's display options with its defaults.
  /** Centre the timeline on the trigger when a capture finds it, unless fitted to it all. */
  const centerTrigger = useStorage('hsync.centerTrigger', true)
  /** Keep the newest samples in view while a capture streams in. */
  const followLatest = useStorage('hsync.followLatest', true)
  /** A quick drag on the tracks keeps the waveform moving after release. */
  const quickScroll = useStorage('hsync.quickScroll', true)
  /** The wheel alone zooms the waveforms, as Ctrl/⌘ + wheel does; over the labels it scrolls. */
  const wheelZoom = useStorage('hsync.wheelZoom', false)
  /** A repeat stopped early shows what it caught instead of the last complete capture. */
  const keepStopped = useStorage('hsync.keepStopped', false)

  /** Look for a newer release in the background at startup. */
  const checkUpdates = useStorage('hsync.checkUpdates', true)

  // Diagnostics.
  const storedLogLevel = useStorage<string>('hsync.logLevel', 'info')
  /** Also write the log to the log file. */
  const logToFile = useStorage('hsync.logToFile', true)
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

  const graphics = computed<GraphicsPreference>({
    get: () => oneOf(GRAPHICS, storedGraphics.value, 'auto'),
    set: (value) => (storedGraphics.value = value),
  })

  const readoutPlace = computed<ReadoutPlace>({
    get: () => oneOf(READOUT_PLACES, storedReadoutPlace.value, 'status'),
    set: (value) => (storedReadoutPlace.value = value),
  })

  const logLevel = computed<LogLevel>({
    get: () => oneOf(LOG_LEVELS, storedLogLevel.value, 'info'),
    set: (value) => (storedLogLevel.value = value),
  })

  const resolvedTheme = computed<Theme>(() =>
    theme.value === 'system' ? (prefersDark.value ? 'dark' : 'light') : theme.value,
  )
  const resolvedLocale = computed<AppLocale>(() =>
    locale.value === 'system' ? detectLocale(navigator.languages) : locale.value,
  )

  return {
    theme,
    density,
    locale,
    graphics,
    activeRenderer,
    readoutPlace,
    centerTrigger,
    followLatest,
    quickScroll,
    wheelZoom,
    keepStopped,
    checkUpdates,
    logLevel,
    logToFile,
    resolvedTheme,
    resolvedLocale,
  }
})
