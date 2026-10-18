import { useStorage } from '@vueuse/core'
import { defineStore } from 'pinia'
import { computed, shallowRef } from 'vue'

import {
  actionOf,
  assignChord,
  clearChord,
  isChanged,
  overridesOf,
  resetChord,
  resolveBindings,
  type Bindings,
  type Overrides,
  type ShortcutId,
} from '@/shortcuts/actions'

/**
 * The keyboard shortcuts (DESIGN.md §10, §11): the chords the user changed, kept in the
 * WebView's local storage, over the defaults.
 */
export const useShortcutsStore = defineStore('shortcuts', () => {
  const stored = useStorage<Overrides>('hsync.shortcuts', {})
  const bindings = computed(() => resolveBindings(stored.value))
  /** The settings are recording a chord: no shortcut runs meanwhile. */
  const recording = shallowRef(false)

  function keep(next: Bindings): void {
    stored.value = overridesOf(next)
  }

  /** Gives `chord` to `id`; returns the action it was taken from. */
  function assign(id: ShortcutId, chord: string): ShortcutId | null {
    const given = assignChord(bindings.value, id, chord)
    keep(given.bindings)
    return given.from
  }

  function clear(id: ShortcutId): void {
    keep(clearChord(bindings.value, id))
  }

  /** Gives `id` its default again; returns the action it was taken back from. */
  function reset(id: ShortcutId): ShortcutId | null {
    const given = resetChord(bindings.value, id)
    keep(given.bindings)
    return given.from
  }

  function resetAll(): void {
    stored.value = {}
  }

  const changed = (id: ShortcutId) => isChanged(bindings.value, id)
  const anyChanged = computed(() => Object.keys(overridesOf(bindings.value)).length > 0)
  const actionFor = (chord: string) => actionOf(bindings.value, chord)

  return { bindings, recording, assign, clear, reset, resetAll, changed, anyChanged, actionFor }
})
