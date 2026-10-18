/**
 * What the keyboard can do and which chord does it (DESIGN.md §10, §11): each action has one
 * chord at most, its default unless the user changed it, and a chord belongs to one action at
 * most, so giving it to one takes it from the other.
 */

export const SHORTCUT_GROUPS = {
  file: ['open', 'save', 'export', 'settings'],
  view: [
    'zoomIn',
    'zoomOut',
    'fit',
    'panLeft',
    'panRight',
    'panLeftFar',
    'panRightFar',
    'clear',
    'search',
    'dataStream',
  ],
  capture: ['capture', 'trigger'],
} as const

export type ShortcutGroup = keyof typeof SHORTCUT_GROUPS
export type ShortcutId = (typeof SHORTCUT_GROUPS)[ShortcutGroup][number]

export const SHORTCUT_IDS: ShortcutId[] = Object.values(SHORTCUT_GROUPS).flat()

/** The chord each action starts with; the ones added for HSync's own actions start unbound. */
export const DEFAULT_CHORDS: Record<ShortcutId, string | null> = {
  open: 'Mod+O',
  save: 'Mod+S',
  export: null,
  settings: 'Mod+,',
  zoomIn: '+',
  zoomOut: '-',
  fit: 'Home',
  panLeft: 'ArrowLeft',
  panRight: 'ArrowRight',
  panLeftFar: 'Shift+ArrowLeft',
  panRightFar: 'Shift+ArrowRight',
  clear: 'Escape',
  search: '/',
  dataStream: null,
  capture: null,
  trigger: null,
}

/**
 * Keys that act as another while no action has them: `=` and `_` share their keys with `+` and
 * `-`, so zooming works with Shift or without.
 */
const ALIASES: Record<string, string> = { '=': '+', _: '-' }

/**
 * Actions that work while a menu or dialog is open: the file actions and the settings, which
 * the same chord closes again. The others leave the keys to what is open.
 */
export const WORK_ANYWHERE: ReadonlySet<ShortcutId> = new Set(['open', 'save', 'settings'])

export type Bindings = Record<ShortcutId, string | null>

/** The chords the user changed, by action (`null`: none); the others keep their defaults. */
export type Overrides = Partial<Record<ShortcutId, string | null>>

const isId = (id: string): id is ShortcutId => (SHORTCUT_IDS as string[]).includes(id)

/**
 * Every action's chord: the defaults with the user's changes, read defensively since they come
 * from storage. A chord stays with the first action that has it, the changed ones first.
 */
export function resolveBindings(overrides: unknown): Bindings {
  const changed: Overrides = {}
  if (overrides && typeof overrides === 'object') {
    for (const [id, chord] of Object.entries(overrides)) {
      if (isId(id) && (chord === null || (typeof chord === 'string' && chord !== ''))) {
        changed[id] = chord
      }
    }
  }
  const taken = new Set<string>()
  const bindings = {} as Bindings
  // A default chord the user gave to another action is no longer free.
  const order = [
    ...SHORTCUT_IDS.filter((id) => id in changed),
    ...SHORTCUT_IDS.filter((id) => !(id in changed)),
  ]
  for (const id of order) {
    const chord = id in changed ? (changed[id] ?? null) : DEFAULT_CHORDS[id]
    bindings[id] = chord !== null && !taken.has(chord) ? chord : null
    if (chord !== null) taken.add(chord)
  }
  return bindings
}

/** The action a chord runs, if any; `=` and `_` run what `+` and `-` do unless they have one. */
export function actionOf(bindings: Bindings, chord: string): ShortcutId | null {
  const owner = (one: string) => SHORTCUT_IDS.find((id) => bindings[id] === one) ?? null
  const alias = ALIASES[chord]
  return owner(chord) ?? (alias === undefined ? null : owner(alias))
}

/** The overrides that make `bindings` from the defaults: only the actions that differ. */
export function overridesOf(bindings: Bindings): Overrides {
  const overrides: Overrides = {}
  for (const id of SHORTCUT_IDS) {
    if (bindings[id] !== DEFAULT_CHORDS[id]) overrides[id] = bindings[id]
  }
  return overrides
}

/** Gives `chord` to action `id`; another action that had it loses it, and `from` names it. */
export function assignChord(
  bindings: Bindings,
  id: ShortcutId,
  chord: string,
): { bindings: Bindings; from: ShortcutId | null } {
  const owner = SHORTCUT_IDS.find((one) => bindings[one] === chord) ?? null
  const from = owner !== null && owner !== id ? owner : null
  const next = { ...bindings, [id]: chord }
  if (from) next[from] = null
  return { bindings: next, from }
}

/** Leaves action `id` without a chord. */
export function clearChord(bindings: Bindings, id: ShortcutId): Bindings {
  return { ...bindings, [id]: null }
}

/** Gives action `id` its default chord again, taking it back from the action that had it. */
export function resetChord(
  bindings: Bindings,
  id: ShortcutId,
): { bindings: Bindings; from: ShortcutId | null } {
  const chord = DEFAULT_CHORDS[id]
  return chord === null
    ? { bindings: clearChord(bindings, id), from: null }
    : assignChord(bindings, id, chord)
}

/** Whether action `id` has another chord than its default. */
export function isChanged(bindings: Bindings, id: ShortcutId): boolean {
  return bindings[id] !== DEFAULT_CHORDS[id]
}
