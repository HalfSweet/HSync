/**
 * Key chords (DESIGN.md §10, §11): a key press as a string such as `Mod+Shift+S`, `Shift+ArrowLeft`
 * or `+`, the same whatever the platform, and how each platform shows it.
 *
 * `Mod` is the platform's command key, ⌘ on Apple systems and Ctrl elsewhere; `Ctrl` is the
 * Control key of Apple systems. Letters and digits are kept by the key they are, symbols by the
 * character typed, which already tells whether Shift made it (`+` rather than `Shift+=`), named
 * keys by their name (`ArrowLeft`, `Home`, `F5`, `Space`).
 */

export const APPLE =
  typeof navigator !== 'undefined' &&
  /Mac|iPhone|iPad/.test(navigator.platform || navigator.userAgent)

type KeyPress = Pick<
  KeyboardEvent,
  'key' | 'code' | 'ctrlKey' | 'metaKey' | 'altKey' | 'shiftKey' | 'isComposing'
>

/** Keys that only modify others. */
const MODIFIERS = new Set([
  'Shift',
  'Control',
  'Alt',
  'AltGraph',
  'Meta',
  'OS',
  'Super',
  'Hyper',
  'Fn',
  'FnLock',
  'CapsLock',
  'NumLock',
  'ScrollLock',
])

/** The key a press names: a capital letter, a digit, a symbol or a key's name. */
function keyOf(event: KeyPress): string | null {
  if (/^[a-z]$/i.test(event.key)) return event.key.toUpperCase()
  // Layouts without Latin letters, ⌥ on Apple systems and dead keys type something else on a
  // letter key, and ⌥ on a digit.
  const letter = /^Key([A-Z])$/.exec(event.code)
  if (letter) return letter[1] ?? null
  const digit = /^Digit(\d)$/.exec(event.code)
  if (digit && (event.altKey || event.key === 'Dead')) return digit[1] ?? null
  if (event.key === 'Dead' || event.key === 'Unidentified' || event.key === 'Process') return null
  if (event.key === ' ') return 'Space'
  return event.key
}

/** Whether `key` is a symbol, typed with or without Shift. */
const isSymbol = (key: string) => key.length === 1 && !/[A-Z0-9]/.test(key)

/**
 * The chord of a key press, or `null` for a modifier alone, a key that types nothing yet, or the
 * Windows key, which belongs to the system.
 */
export function chordOf(event: KeyPress, apple = APPLE): string | null {
  if (event.isComposing || MODIFIERS.has(event.key)) return null
  if (!apple && event.metaKey) return null
  const key = keyOf(event)
  if (!key) return null
  const modifiers = modifiersOf(event, apple).filter((one) => one !== 'Shift' || !isSymbol(key))
  return [...modifiers, key].join('+')
}

/** The modifiers held in a key press, in the order chords name them. */
function modifiersOf(event: KeyPress, apple: boolean): string[] {
  const parts: string[] = []
  if (apple ? event.metaKey : event.ctrlKey) parts.push('Mod')
  if (apple && event.ctrlKey) parts.push('Ctrl')
  if (event.altKey) parts.push('Alt')
  if (event.shiftKey) parts.push('Shift')
  return parts
}

/** The most keys a chord is pressed with, its key included. */
export const MAX_KEYS = 4

/** How many keys the chord is pressed with. */
export function keyCount(chord: string): number {
  return partsOf(chord).modifiers.size + 1
}

/**
 * The modifiers held while a chord is being recorded, as they read (`⌘⇧…`, `Ctrl+Shift+…`), or
 * `null` when none is down.
 */
export function heldLabel(event: KeyPress, apple = APPLE): string | null {
  const modifiers = modifiersOf(event, apple)
  return modifiers.length === 0 ? null : formatChord([...modifiers, '…'].join('+'), '', apple)
}

/** The chord's modifiers and its key. */
function partsOf(chord: string): { modifiers: Set<string>; key: string } {
  // The key may itself be `+`: it is what follows the last modifier.
  const match = /^((?:(?:Mod|Ctrl|Alt|Shift)\+)*)(.+)$/.exec(chord)
  const modifiers = new Set((match?.[1] ?? '').split('+').filter(Boolean))
  return { modifiers, key: match?.[2] ?? chord }
}

/** Whether the chord holds the command or Control key, as shortcuts that work in text fields do. */
export function hasCommand(chord: string): boolean {
  const { modifiers } = partsOf(chord)
  return modifiers.has('Mod') || modifiers.has('Ctrl')
}

/** Keys that read otherwise than their names. */
const KEYCAPS: Record<string, string> = {
  ArrowLeft: '←',
  ArrowRight: '→',
  ArrowUp: '↑',
  ArrowDown: '↓',
  Escape: 'Esc',
  PageUp: 'PgUp',
  PageDown: 'PgDn',
}

/**
 * How the chord reads: `⌘⇧S` on Apple systems, `Ctrl+Shift+S` elsewhere. Named keys read as on
 * the keycap; `space` names the space bar in the interface's language.
 */
export function formatChord(chord: string, space: string, apple = APPLE): string {
  const { modifiers, key } = partsOf(chord)
  const name = key === 'Space' ? space : (KEYCAPS[key] ?? key)
  if (apple) {
    const symbols = [
      modifiers.has('Ctrl') && '⌃',
      modifiers.has('Alt') && '⌥',
      modifiers.has('Shift') && '⇧',
      modifiers.has('Mod') && '⌘',
    ]
    return `${symbols.filter(Boolean).join('')}${name}`
  }
  const words = [
    modifiers.has('Mod') && 'Ctrl',
    modifiers.has('Alt') && 'Alt',
    modifiers.has('Shift') && 'Shift',
  ]
  return [...words.filter(Boolean), name].join('+')
}
