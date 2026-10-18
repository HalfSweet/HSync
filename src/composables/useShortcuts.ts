import { useEventListener } from '@vueuse/core'
import { useI18n } from 'vue-i18n'

import { WORK_ANYWHERE, type ShortcutId } from '@/shortcuts/actions'
import { chordOf, formatChord, hasCommand } from '@/shortcuts/chords'
import { useShortcutsStore } from '@/stores/shortcuts'

type Handler = (event: KeyboardEvent) => boolean | void

/** Whether a key press goes into a text field. */
export function inTextField(target: EventTarget | null): boolean {
  const element = target instanceof Element ? target : null
  return element?.closest('input, textarea, select, [contenteditable="true"]') != null
}

/** Whether a menu, dialog or list box is open, which has the keys. */
export function somethingOpen(root: ParentNode = document): boolean {
  return root.querySelector('[role="menu"], [role="dialog"], [role="listbox"]') !== null
}

/**
 * Runs `handlers` on their actions' chords (DESIGN.md §10). A chord without ⌘ or Ctrl leaves
 * text fields alone, actions other than the file ones and the settings leave the keys to an open
 * menu or dialog, and nothing runs while the settings record a chord. A handler that returns
 * `false` did nothing, and the key goes on to whatever else wants it.
 */
export function useShortcuts(handlers: Partial<Record<ShortcutId, Handler>>): void {
  const shortcuts = useShortcutsStore()
  useEventListener(document, 'keydown', (event: KeyboardEvent) => {
    if (event.defaultPrevented || shortcuts.recording) return
    const chord = chordOf(event)
    const id = chord === null ? null : shortcuts.actionFor(chord)
    const run = id === null ? undefined : handlers[id]
    if (chord === null || id === null || !run) return
    if (!hasCommand(chord) && inTextField(event.target)) return
    if (!WORK_ANYWHERE.has(id) && somethingOpen()) return
    if (run(event) !== false) event.preventDefault()
  })
}

/** How a chord reads in the interface's language. */
export function useChordLabel(): (chord: string) => string {
  const { t } = useI18n()
  return (chord) => formatChord(chord, t('shortcuts.space'))
}

/** How an action's chord reads, for menus and tips; `null` when it has none. */
export function useShortcutLabel(): (id: ShortcutId) => string | null {
  const shortcuts = useShortcutsStore()
  const label = useChordLabel()
  return (id) => {
    const chord = shortcuts.bindings[id]
    return chord === null ? null : label(chord)
  }
}

/** A tip that names an action's shortcut after its label, when it has one. */
export function useShortcutTip(): (label: string, id: ShortcutId) => string {
  const { t } = useI18n()
  const shortcut = useShortcutLabel()
  return (label, id) => {
    const keys = shortcut(id)
    return keys === null ? label : t('header.withShortcut', { label, shortcut: keys })
  }
}
