import { describe, expect, it } from 'vitest'

import { commandLabel, isCommand } from './keys'

const press = (key: string, init: KeyboardEventInit) =>
  new KeyboardEvent('keydown', { key, ...init })

describe('command shortcuts', () => {
  it('uses ⌘ on Apple systems and Ctrl elsewhere', () => {
    expect(isCommand(press('s', { metaKey: true }), 's', true)).toBe(true)
    expect(isCommand(press('s', { ctrlKey: true }), 's', true)).toBe(false)
    expect(isCommand(press('s', { ctrlKey: true }), 's', false)).toBe(true)
    expect(isCommand(press('S', { ctrlKey: true }), 's', false)).toBe(true)
  })

  it('ignores other modifiers', () => {
    expect(isCommand(press('s', { ctrlKey: true, shiftKey: true }), 's', false)).toBe(false)
    expect(isCommand(press('o', { metaKey: true, altKey: true }), 'o', true)).toBe(false)
  })

  it('labels the shortcut', () => {
    expect(commandLabel('s', true)).toBe('⌘S')
    expect(commandLabel('o', false)).toBe('Ctrl+O')
  })
})
