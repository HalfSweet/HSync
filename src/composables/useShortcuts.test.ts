import { mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { defineComponent, h } from 'vue'

import { i18n } from '@/i18n'
import { APPLE } from '@/shortcuts/chords'
import { useShortcutsStore } from '@/stores/shortcuts'

import { useShortcutLabel, useShortcuts, useShortcutTip } from './useShortcuts'

/** A key press on `target`, as the platform's command key + `key` when `command` is set. */
function press(key: string, code: string, target: EventTarget = document.body, command = false) {
  const event = new KeyboardEvent('keydown', {
    key,
    code,
    bubbles: true,
    cancelable: true,
    metaKey: command && APPLE,
    ctrlKey: command && !APPLE,
  })
  target.dispatchEvent(event)
  return event
}

/** A component that runs `setup` in its own setup, with a text field to type in. */
const Host = defineComponent({
  props: { setup: { type: Function, required: true } },
  setup(props) {
    ;(props.setup as () => void)()
    return () => h('input')
  },
})

const host = (handlers: Parameters<typeof useShortcuts>[0]) =>
  mount(Host, {
    props: { setup: () => useShortcuts(handlers) },
    attachTo: document.body,
    global: { plugins: [i18n] },
  })

describe('useShortcuts', () => {
  beforeEach(() => {
    localStorage.clear()
    setActivePinia(createPinia())
    i18n.global.locale.value = 'en-US'
  })
  afterEach(() => {
    document.body.innerHTML = ''
  })

  it("runs an action's handler on its chords and takes the key", () => {
    const zoomIn = vi.fn()
    const wrapper = host({ zoomIn })
    expect(press('+', 'Equal').defaultPrevented).toBe(true)
    press('=', 'Equal')
    expect(zoomIn).toHaveBeenCalledTimes(2)
    expect(press('x', 'KeyX').defaultPrevented).toBe(false)
    wrapper.unmount()
  })

  it('follows the chords the user gave', () => {
    const capture = vi.fn()
    const wrapper = host({ capture })
    press(' ', 'Space')
    expect(capture).not.toHaveBeenCalled()
    useShortcutsStore().assign('capture', 'Space')
    press(' ', 'Space')
    expect(capture).toHaveBeenCalledOnce()
    wrapper.unmount()
  })

  it('leaves text fields their keys, but not command chords', () => {
    const search = vi.fn()
    const save = vi.fn()
    const wrapper = host({ search, save })
    const field = wrapper.find('input').element
    press('/', 'Slash', field)
    expect(search).not.toHaveBeenCalled()
    press('s', 'KeyS', field, true)
    expect(save).toHaveBeenCalledOnce()
    wrapper.unmount()
  })

  it('leaves the keys to an open menu or dialog, except for the file actions and settings', () => {
    const fit = vi.fn()
    const settings = vi.fn()
    const wrapper = host({ fit, settings })
    const dialog = document.createElement('div')
    dialog.setAttribute('role', 'dialog')
    document.body.append(dialog)
    press('Home', 'Home')
    expect(fit).not.toHaveBeenCalled()
    press(',', 'Comma', document.body, true)
    expect(settings).toHaveBeenCalledOnce()
    wrapper.unmount()
  })

  it('runs nothing while the settings record a chord', () => {
    const fit = vi.fn()
    const wrapper = host({ fit })
    useShortcutsStore().recording = true
    press('Home', 'Home')
    expect(fit).not.toHaveBeenCalled()
    wrapper.unmount()
  })

  it('lets the key go on when the handler did nothing', () => {
    const wrapper = host({ clear: () => false })
    expect(press('Escape', 'Escape').defaultPrevented).toBe(false)
    wrapper.unmount()
  })

  it("names an action's chord in menus and tips", () => {
    const shown = { label: null as string | null, tip: '' }
    const wrapper = mount(Host, {
      props: {
        setup: () => {
          shown.label = useShortcutLabel()('capture')
          shown.tip = useShortcutTip()('Zoom in', 'zoomIn')
        },
      },
      global: { plugins: [i18n] },
    })
    expect(shown.label).toBeNull()
    expect(shown.tip).toBe('Zoom in (+)')
    wrapper.unmount()
  })
})
