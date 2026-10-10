import { describe, expect, it } from 'vitest'

import { keyTakenElsewhere, timelineAction } from './keys'

const key = (key: string, more: Partial<KeyboardEvent> = {}) => ({
  key,
  ctrlKey: false,
  metaKey: false,
  altKey: false,
  shiftKey: false,
  ...more,
})

describe('timeline keys', () => {
  it('zoom, pan, fit and clear', () => {
    expect(timelineAction(key('+'))).toEqual({ kind: 'zoom', factor: 2 })
    expect(timelineAction(key('='))).toEqual({ kind: 'zoom', factor: 2 })
    expect(timelineAction(key('-'))).toEqual({ kind: 'zoom', factor: 0.5 })
    expect(timelineAction(key('ArrowLeft'))).toEqual({ kind: 'pan', share: -0.1 })
    expect(timelineAction(key('ArrowRight', { shiftKey: true }))).toEqual({
      kind: 'pan',
      share: 0.5,
    })
    expect(timelineAction(key('Home'))).toEqual({ kind: 'fit' })
    expect(timelineAction(key('Escape'))).toEqual({ kind: 'clear' })
  })

  it('leave other keys and shortcuts alone', () => {
    expect(timelineAction(key('a'))).toBeNull()
    expect(timelineAction(key('-', { metaKey: true }))).toBeNull()
    expect(timelineAction(key('+', { ctrlKey: true }))).toBeNull()
  })

  it('give way to text fields and open menus', () => {
    const page = document.createElement('div')
    const field = document.createElement('input')
    page.append(field)
    expect(keyTakenElsewhere(field, page)).toBe(true)
    expect(keyTakenElsewhere(page, page)).toBe(false)
    const menu = document.createElement('div')
    menu.setAttribute('role', 'menu')
    page.append(menu)
    expect(keyTakenElsewhere(page, page)).toBe(true)
  })
})
