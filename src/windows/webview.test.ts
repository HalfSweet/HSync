import { isTauri } from '@tauri-apps/api/core'
import { flushPromises, mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { defineComponent, h } from 'vue'

import ShortcutsSection from '@/components/settings/ShortcutsSection.vue'
import { useShortcuts } from '@/composables/useShortcuts'
import { i18n } from '@/i18n'
import { APPLE } from '@/shortcuts/chords'
import { useShortcutsStore } from '@/stores/shortcuts'

import { installWebviewGuards } from './webview'

vi.mock('@tauri-apps/api/core', () => ({ isTauri: vi.fn(() => true) }))

function press(key: string, more: KeyboardEventInit = {}, target: EventTarget = document.body) {
  const event = new KeyboardEvent('keydown', { key, bubbles: true, cancelable: true, ...more })
  target.dispatchEvent(event)
  return event
}

function dispatch(type: string, target: EventTarget = document.body) {
  const event = new Event(type, { bubbles: true, cancelable: true })
  target.dispatchEvent(event)
  return event
}

const command = APPLE ? { metaKey: true } : { ctrlKey: true }

describe('desktop WebView defaults', () => {
  let remove: () => void

  beforeEach(() => {
    vi.mocked(isTauri).mockReturnValue(true)
    localStorage.clear()
    setActivePinia(createPinia())
    remove = installWebviewGuards()
  })

  afterEach(() => {
    remove()
    document.body.replaceChildren()
  })

  it.each([
    ['F5', {}],
    ['F5', { ctrlKey: true }],
    ['F5', { shiftKey: true }],
    ['r', command],
    ['R', { ...command, shiftKey: true }],
    ['s', command],
    ['S', { ...command, shiftKey: true }],
    ['o', command],
    ['p', command],
    ['f', command],
    ['g', command],
    ['F3', {}],
    ['F7', {}],
    ['+', command],
    ['-', command],
    ['0', command],
    ['Backspace', {}],
    ['BrowserBack', {}],
  ] satisfies [string, KeyboardEventInit][])(
    'prevents the browser default for %s with %j even without any app handlers',
    (key, modifiers) => {
      expect(press(key, modifiers).defaultPrevented).toBe(true)
    },
  )

  it.each([false, true])('blocks platform shortcuts with apple=%s', (apple) => {
    remove()
    remove = installWebviewGuards(apple)
    const mod = apple ? { metaKey: true } : { ctrlKey: true }
    expect(press('r', mod).defaultPrevented).toBe(true)
    expect(press('s', mod).defaultPrevented).toBe(true)
    expect(press('=', mod).defaultPrevented).toBe(true)
    if (apple) expect(press('r', { ...mod, altKey: true }).defaultPrevented).toBe(true)
    expect(press(apple ? '[' : 'ArrowLeft', apple ? mod : { altKey: true }).defaultPrevented).toBe(
      true,
    )
  })

  it('still blocks refresh and save while an input in a dialog has focus', () => {
    document.body.innerHTML = '<div role="dialog"><input></div>'
    const input = document.querySelector('input')!
    input.focus()
    expect(press('F5', {}, input).defaultPrevented).toBe(true)
    expect(press('s', command, input).defaultPrevented).toBe(true)
    expect(press('Backspace', {}, input).defaultPrevented).toBe(false)
  })

  it('lets application shortcuts run before suppressing unhandled browser actions', () => {
    const save = vi.fn()
    const capture = vi.fn(() => false)
    const wrapper = mount(
      defineComponent({
        setup() {
          useShortcuts({ save, capture })
          return () => h('input')
        },
      }),
      { attachTo: document.body },
    )
    try {
      expect(press('s', command, wrapper.get('input').element).defaultPrevented).toBe(true)
      expect(save).toHaveBeenCalledOnce()

      useShortcutsStore().clear('save')
      expect(press('s', command).defaultPrevented).toBe(true)
      expect(save).toHaveBeenCalledOnce()

      useShortcutsStore().assign('capture', 'F5')
      expect(press('F5').defaultPrevented).toBe(true)
      expect(capture).toHaveBeenCalledOnce()

      useShortcutsStore().assign('capture', 'Mod+R')
      expect(press('r', command).defaultPrevented).toBe(true)
      expect(capture).toHaveBeenCalledTimes(2)
    } finally {
      wrapper.unmount()
    }
  })

  it('keeps browser chords recordable as application shortcuts', async () => {
    const wrapper = mount(ShortcutsSection, {
      attachTo: document.body,
      global: { plugins: [i18n] },
    })
    try {
      const box = wrapper.get('[data-shortcut="capture"]')
      await box.trigger('click')
      await flushPromises()
      expect(press('F5', {}, box.element).defaultPrevented).toBe(true)
      expect(useShortcutsStore().bindings.capture).toBe('F5')
      expect(useShortcutsStore().recording).toBe(false)
    } finally {
      wrapper.unmount()
    }
  })

  it('preserves text editing, focus navigation, IME input and system window shortcuts', () => {
    for (const key of ['a', 'c', 'v', 'x', 'z']) {
      expect(press(key, command).defaultPrevented).toBe(false)
    }
    expect(press('Tab').defaultPrevented).toBe(false)
    expect(press('Escape').defaultPrevented).toBe(false)
    expect(press('F4', { altKey: true }).defaultPrevented).toBe(false)
    expect(press('r', { ...command, isComposing: true }).defaultPrevented).toBe(false)
    expect(press('s', { ctrlKey: true, altKey: true }).defaultPrevented).toBe(false)

    remove()
    remove = installWebviewGuards(true)
    document.body.innerHTML = '<textarea></textarea>'
    const field = document.querySelector('textarea')!
    for (const key of ['a', 'd', 'h', 'k']) {
      expect(press(key, { ctrlKey: true }, field).defaultPrevented).toBe(false)
    }
    expect(press('ArrowLeft', { metaKey: true }, field).defaultPrevented).toBe(false)
  })

  it('suppresses the page context menu without swallowing application context menus', () => {
    const channel = document.createElement('button')
    document.body.append(channel)
    const openMenu = vi.fn()
    channel.addEventListener('contextmenu', openMenu)
    expect(dispatch('contextmenu', channel).defaultPrevented).toBe(true)
    expect(openMenu).toHaveBeenCalledOnce()
  })

  it('preserves native editing menus and deletion, including inherited editable content', () => {
    document.body.innerHTML = `
      <input><textarea></textarea><select><option>One</option></select>
      <div contenteditable><span data-editable></span></div>
      <div contenteditable="plaintext-only"><span data-editable></span></div>
      <div contenteditable="true"><span contenteditable="false" id="readonly"></span></div>
    `
    for (const field of document.querySelectorAll('input, textarea, select, [data-editable]')) {
      expect(dispatch('contextmenu', field).defaultPrevented).toBe(false)
      expect(press('Backspace', {}, field).defaultPrevented).toBe(false)
    }
    expect(dispatch('contextmenu', document.getElementById('readonly')!).defaultPrevented).toBe(
      true,
    )
  })

  it('prevents page zoom without stopping waveform wheel handlers or ordinary scrolling', () => {
    const timeline = document.createElement('div')
    document.body.append(timeline)
    const zoom = vi.fn()
    timeline.addEventListener('wheel', zoom)
    for (const modifiers of [{ ctrlKey: true }, { metaKey: true }]) {
      const wheel = new WheelEvent('wheel', {
        ...modifiers,
        deltaY: -40,
        bubbles: true,
        cancelable: true,
      })
      timeline.dispatchEvent(wheel)
      expect(wheel.defaultPrevented).toBe(true)
    }
    expect(zoom).toHaveBeenCalledTimes(2)
    expect(dispatch('wheel').defaultPrevented).toBe(false)
  })

  it('removes its listeners and leaves the browser development preview alone', () => {
    remove()
    expect(press('F5').defaultPrevented).toBe(false)
    expect(dispatch('contextmenu').defaultPrevented).toBe(false)
    vi.mocked(isTauri).mockReturnValue(false)
    remove = installWebviewGuards()
    expect(press('F5').defaultPrevented).toBe(false)
    expect(press('s', command).defaultPrevented).toBe(false)
    expect(dispatch('contextmenu').defaultPrevented).toBe(false)
  })
})
