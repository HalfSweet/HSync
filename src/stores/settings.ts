import { defineStore } from 'pinia'
import { shallowRef } from 'vue'

/** The settings page's sections, in the order its side bar lists them (DESIGN.md §11). */
export const SETTINGS_SECTIONS = ['appearance', 'behaviour', 'diagnostics'] as const
export type SettingsSection = (typeof SETTINGS_SECTIONS)[number]

/** Whether the settings page is open, and the section it shows, kept for the next opening. */
export const useSettingsStore = defineStore('settings', () => {
  const open = shallowRef(false)
  const section = shallowRef<SettingsSection>('appearance')

  function toggle(): void {
    open.value = !open.value
  }

  function close(): void {
    open.value = false
  }

  return { open, section, toggle, close }
})
