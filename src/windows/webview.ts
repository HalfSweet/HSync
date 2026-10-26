import { isTauri } from '@tauri-apps/api/core'

import { APPLE, chordOf } from '@/shortcuts/chords'

/** Browser commands that must never fall through when an app action is unavailable or unbound. */
const BROWSER_CHORDS = new Set([
  'Mod+R',
  'Mod+Shift+R',
  'Mod+O',
  'Mod+S',
  'Mod+Shift+S',
  'Mod+P',
  'Mod+Shift+P',
  'Mod+F',
  'Mod+G',
  'Mod+Shift+G',
  'Mod++',
  'Mod+=',
  'Mod+-',
  'Mod+_',
  'Mod+0',
  'F3',
  'Shift+F3',
  'F6',
  'Shift+F6',
  'F7',
  'BrowserBack',
  'BrowserForward',
  'BrowserRefresh',
])

/** Native text controls keep their editing menu and navigation/deletion keys. */
function inTextControl(target: EventTarget | null): boolean {
  if (!(target instanceof Element)) return false
  if (target.closest('input, textarea, select')) return true
  // Use the nearest explicit setting: an editable ancestor can contain a noneditable child.
  const editable = target.closest('[contenteditable]')?.getAttribute('contenteditable')
  return editable === '' || editable === 'true' || editable === 'plaintext-only'
}

function isBrowserShortcut(event: KeyboardEvent, apple: boolean): boolean {
  if (event.isComposing) return false
  // F5, Shift+F5 and Ctrl+F5 reload on WebViews that support those accelerators.
  if (event.key === 'F5' && !event.altKey && !event.metaKey) return true
  const chord = chordOf(event, apple)
  if (chord === null) return false
  if (BROWSER_CHORDS.has(chord)) return true
  if (apple && chord === 'Mod+Alt+R') return true
  if (!apple && (chord === 'Alt+ArrowLeft' || chord === 'Alt+ArrowRight')) return true
  if (apple && (chord === 'Mod+[' || chord === 'Mod+]')) return true
  if (inTextControl(event.target)) return false
  return (
    chord === 'Backspace' ||
    chord === 'Shift+Backspace' ||
    (apple && (chord === 'Mod+ArrowLeft' || chord === 'Mod+ArrowRight'))
  )
}

/**
 * Keeps desktop windows from acting as browser pages (DESIGN.md §10). Install once at the
 * shared entry point so the main window and the detached data stream have the same defaults.
 * Returns cleanup for hot replacement; a regular browser preview keeps its browser controls.
 */
export function installWebviewGuards(apple = APPLE): () => void {
  if (!isTauri()) return () => {}

  const onKeydown = (event: KeyboardEvent) => {
    if (isBrowserShortcut(event, apple)) event.preventDefault()
  }
  const onContextMenu = (event: MouseEvent) => {
    if (!inTextControl(event.target)) event.preventDefault()
  }
  const onWheel = (event: WheelEvent) => {
    if (event.ctrlKey || event.metaKey) event.preventDefault()
  }

  // Bubble at window, after component and document handlers. Cancelling in capture would
  // make useShortcuts ignore the key as already handled, breaking Open/Save and custom chords.
  // Never stop propagation: waveform zoom and the app's context menus still receive events.
  window.addEventListener('keydown', onKeydown)
  window.addEventListener('contextmenu', onContextMenu)
  window.addEventListener('wheel', onWheel, { passive: false })
  return () => {
    window.removeEventListener('keydown', onKeydown)
    window.removeEventListener('contextmenu', onContextMenu)
    window.removeEventListener('wheel', onWheel)
  }
}
