import { createPinia, setActivePinia } from 'pinia'
import { beforeEach, describe, expect, it } from 'vitest'

import { usePreferencesStore } from './preferences'

describe('preferences store', () => {
  beforeEach(() => {
    localStorage.clear()
    setActivePinia(createPinia())
  })

  it('defaults to the system theme, comfortable density and system language', () => {
    const store = usePreferencesStore()
    expect(store.theme).toBe('system')
    expect(store.density).toBe('comfortable')
    expect(store.locale).toBe('system')
  })

  it('leaves the wheel alone scrolling the rows until asked to zoom with it', () => {
    expect(usePreferencesStore().wheelZoom).toBe(false)
  })

  it('resolves explicit choices', () => {
    const store = usePreferencesStore()
    store.theme = 'dark'
    store.locale = 'en-US'
    expect(store.resolvedTheme).toBe('dark')
    expect(store.resolvedLocale).toBe('en-US')
  })

  it('ignores invalid stored values', () => {
    localStorage.setItem('hsync.theme', 'neon')
    localStorage.setItem('hsync.density', 'cramped')
    const store = usePreferencesStore()
    expect(store.theme).toBe('system')
    expect(store.density).toBe('comfortable')
  })
})
