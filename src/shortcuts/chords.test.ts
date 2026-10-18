import { describe, expect, it } from 'vitest'

import { chordOf, formatChord, hasCommand, heldLabel, keyCount } from './chords'

const press = (key: string, code: string, more: Partial<KeyboardEventInit> = {}) =>
  new KeyboardEvent('keydown', { key, code, ...more })

describe('chordOf', () => {
  it('takes ⌘ on Apple systems and Ctrl elsewhere as the command key', () => {
    expect(chordOf(press('s', 'KeyS', { metaKey: true }), true)).toBe('Mod+S')
    expect(chordOf(press('s', 'KeyS', { ctrlKey: true }), false)).toBe('Mod+S')
    // Control on an Apple system is a key of its own.
    expect(chordOf(press('s', 'KeyS', { ctrlKey: true }), true)).toBe('Ctrl+S')
    expect(chordOf(press('S', 'KeyS', { metaKey: true, shiftKey: true }), true)).toBe('Mod+Shift+S')
  })

  it('leaves the Windows key to the system', () => {
    expect(chordOf(press('s', 'KeyS', { metaKey: true }), false)).toBeNull()
  })

  it('keeps symbols as typed, which tells whether Shift made them', () => {
    expect(chordOf(press('+', 'Equal', { shiftKey: true }), false)).toBe('+')
    expect(chordOf(press('=', 'Equal'), false)).toBe('=')
    expect(chordOf(press('+', 'NumpadAdd'), false)).toBe('+')
    expect(chordOf(press(',', 'Comma', { metaKey: true }), true)).toBe('Mod+,')
  })

  it('keeps Shift with named keys and letters', () => {
    expect(chordOf(press('ArrowLeft', 'ArrowLeft', { shiftKey: true }), false)).toBe(
      'Shift+ArrowLeft',
    )
    expect(chordOf(press('Home', 'Home'), false)).toBe('Home')
    expect(chordOf(press(' ', 'Space'), false)).toBe('Space')
    expect(chordOf(press('F5', 'F5', { altKey: true }), false)).toBe('Alt+F5')
  })

  it('names a letter key by its letter on any layout', () => {
    // A Russian layout types Cyrillic on the O key; ⌥ on an Apple system types ø.
    expect(chordOf(press('щ', 'KeyO', { ctrlKey: true }), false)).toBe('Mod+O')
    expect(chordOf(press('ø', 'KeyO', { altKey: true }), true)).toBe('Alt+O')
    expect(chordOf(press('Dead', 'KeyE', { altKey: true }), true)).toBe('Alt+E')
    expect(chordOf(press('¡', 'Digit1', { altKey: true }), true)).toBe('Alt+1')
    // Layouts with other Latin letters keep the letter typed.
    expect(chordOf(press('a', 'KeyQ'), false)).toBe('A')
  })

  it('waits for a key while only modifiers are down or text is being composed', () => {
    expect(chordOf(press('Shift', 'ShiftLeft', { shiftKey: true }), false)).toBeNull()
    expect(chordOf(press('Meta', 'MetaLeft', { metaKey: true }), true)).toBeNull()
    expect(chordOf(press('Dead', 'Quote'), false)).toBeNull()
    expect(chordOf(press('n', 'KeyN', { isComposing: true }), false)).toBeNull()
  })
})

describe('formatChord', () => {
  it('reads with symbols on Apple systems, in Apple order', () => {
    expect(formatChord('Mod+Shift+S', 'Space', true)).toBe('⇧⌘S')
    expect(formatChord('Mod+Ctrl+Alt+K', 'Space', true)).toBe('⌃⌥⌘K')
    expect(formatChord('Shift+ArrowLeft', 'Space', true)).toBe('⇧←')
  })

  it('reads with words elsewhere', () => {
    expect(formatChord('Mod+Shift+S', 'Space', false)).toBe('Ctrl+Shift+S')
    expect(formatChord('Mod+,', 'Space', false)).toBe('Ctrl+,')
    expect(formatChord('Alt+Escape', 'Space', false)).toBe('Alt+Esc')
    expect(formatChord('PageDown', 'Space', false)).toBe('PgDn')
  })

  it('names the space bar in the interface language and keeps a + key', () => {
    expect(formatChord('Space', '空格', false)).toBe('空格')
    expect(formatChord('+', 'Space', true)).toBe('+')
    expect(formatChord('Mod++', 'Space', false)).toBe('Ctrl++')
  })
})

describe('hasCommand', () => {
  it('tells chords that work in text fields', () => {
    expect(hasCommand('Mod+S')).toBe(true)
    expect(hasCommand('Ctrl+K')).toBe(true)
    expect(hasCommand('Shift+ArrowLeft')).toBe(false)
    expect(hasCommand('+')).toBe(false)
  })
})

describe('recording a chord', () => {
  it('shows the modifiers held so far', () => {
    expect(heldLabel(press('Meta', 'MetaLeft', { metaKey: true, shiftKey: true }), true)).toBe(
      '⇧⌘…',
    )
    expect(heldLabel(press('Control', 'ControlLeft', { ctrlKey: true }), false)).toBe('Ctrl+…')
    expect(heldLabel(press('a', 'KeyA'), false)).toBeNull()
  })

  it('counts the keys a chord is pressed with', () => {
    expect(keyCount('Home')).toBe(1)
    expect(keyCount('Mod+Shift+S')).toBe(3)
    expect(keyCount('Mod+Ctrl+Alt+Shift+K')).toBe(5)
    expect(keyCount('Mod++')).toBe(2)
  })
})
