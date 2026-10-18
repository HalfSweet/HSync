import { describe, expect, it } from 'vitest'

import {
  actionOf,
  assignChord,
  clearChord,
  DEFAULT_CHORDS,
  isChanged,
  overridesOf,
  resetChord,
  resolveBindings,
  SHORTCUT_IDS,
} from './actions'

const defaults = resolveBindings({})

describe('shortcut bindings', () => {
  it('start from the defaults, each chord with one action', () => {
    expect(defaults.save).toBe('Mod+S')
    expect(defaults.zoomIn).toBe('+')
    expect(defaults.capture).toBeNull()
    const chords = SHORTCUT_IDS.flatMap((id) => DEFAULT_CHORDS[id] ?? [])
    expect(new Set(chords).size).toBe(chords.length)
    expect(actionOf(defaults, 'Shift+ArrowLeft')).toBe('panLeftFar')
    expect(actionOf(defaults, 'Mod+K')).toBeNull()
  })

  it('let = and _ zoom as + and - do, unless an action has them', () => {
    expect(actionOf(defaults, '=')).toBe('zoomIn')
    expect(actionOf(defaults, '_')).toBe('zoomOut')
    const given = assignChord(defaults, 'fit', '=').bindings
    expect(actionOf(given, '=')).toBe('fit')
    // The alias follows `+` wherever it goes.
    expect(actionOf(assignChord(defaults, 'search', '+').bindings, '=')).toBe('search')
  })

  it('read stored changes defensively', () => {
    const read = resolveBindings({
      capture: 'Space',
      export: '',
      nothing: 'Mod+Q',
      search: 42,
      fit: null,
    })
    expect(read.capture).toBe('Space')
    expect(read.export).toBeNull()
    expect(read.search).toBe('/')
    expect(read.fit).toBeNull()
    expect(resolveBindings('garbage')).toEqual(defaults)
    expect(resolveBindings(null)).toEqual(defaults)
  })

  it('give a changed action the chord before the action it was the default of', () => {
    const read = resolveBindings({ export: 'Mod+S' })
    expect(read.export).toBe('Mod+S')
    expect(read.save).toBeNull()
  })

  it('take a chord from the action that had it', () => {
    const given = assignChord(defaults, 'export', 'Mod+S')
    expect(given.from).toBe('save')
    expect(given.bindings.export).toBe('Mod+S')
    expect(given.bindings.save).toBeNull()
    expect(overridesOf(given.bindings)).toEqual({ export: 'Mod+S', save: null })
    expect(assignChord(defaults, 'save', 'Mod+S').from).toBeNull()
  })

  it('clear a chord and tell changed actions', () => {
    const cleared = clearChord(defaults, 'zoomOut')
    expect(cleared.zoomOut).toBeNull()
    expect(isChanged(cleared, 'zoomOut')).toBe(true)
    expect(isChanged(cleared, 'zoomIn')).toBe(false)
  })

  it('take the default back on a reset', () => {
    const changed = assignChord(defaults, 'export', 'Mod+S').bindings
    const reset = resetChord(changed, 'save')
    expect(reset.bindings.save).toBe('Mod+S')
    expect(reset.bindings.export).toBeNull()
    expect(reset.from).toBe('export')
    expect(overridesOf(reset.bindings)).toEqual({})
    // An action without a default is left without one.
    const bound = assignChord(defaults, 'capture', 'Space').bindings
    expect(resetChord(bound, 'capture')).toEqual({ bindings: defaults, from: null })
  })
})
