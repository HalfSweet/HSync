import { flushPromises, mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { i18n } from '@/i18n'
import { APPLE, formatChord } from '@/shortcuts/chords'
import { useShortcutsStore } from '@/stores/shortcuts'

import ShortcutsSection from './ShortcutsSection.vue'

const shown = (chord: string) => formatChord(chord, 'Space')

async function section() {
  const wrapper = mount(ShortcutsSection, { global: { plugins: [i18n] }, attachTo: document.body })
  await flushPromises()
  return wrapper
}

type Wrapper = Awaited<ReturnType<typeof section>>

const box = (wrapper: Wrapper, id: string) => wrapper.find(`[data-shortcut="${id}"]`)
const row = (wrapper: Wrapper, id: string) => box(wrapper, id).element.closest('li') as HTMLElement

/** Clicks the action's box to record a chord in it. */
async function record(wrapper: Wrapper, id: string) {
  await box(wrapper, id).trigger('click')
  await flushPromises()
  expect(document.activeElement).toBe(box(wrapper, id).element)
}

/** Presses keys on the action's box; `command` adds the platform's command key. */
async function press(
  wrapper: Wrapper,
  id: string,
  key: string,
  code: string,
  more: { command?: boolean; shift?: boolean; type?: 'keydown' | 'keyup' } = {},
) {
  await box(wrapper, id).trigger(more.type ?? 'keydown', {
    key,
    code,
    metaKey: !!more.command && APPLE,
    ctrlKey: !!more.command && !APPLE,
    shiftKey: !!more.shift,
  })
  await flushPromises()
}

describe('shortcuts section', () => {
  beforeEach(() => {
    localStorage.clear()
    setActivePinia(createPinia())
    i18n.global.locale.value = 'en-US'
  })
  afterEach(() => {
    document.body.innerHTML = ''
  })

  it('lists the actions by group, each with its shortcut in one box', async () => {
    const wrapper = await section()
    expect(wrapper.findAll('h3').map((one) => one.text())).toEqual(['File', 'View', 'Capture'])
    expect(box(wrapper, 'save').text()).toBe(shown('Mod+S'))
    expect(box(wrapper, 'zoomIn').text()).toBe('+')
    expect(box(wrapper, 'capture').text()).toBe('None')
    expect(wrapper.findAll('[data-shortcut]')).toHaveLength(16)
    wrapper.unmount()
  })

  it('records the keys pressed together, showing the modifiers as they go down', async () => {
    const wrapper = await section()
    await record(wrapper, 'capture')
    expect(useShortcutsStore().recording).toBe(true)
    expect(box(wrapper, 'capture').text()).toBe('Press a shortcut…')

    await press(wrapper, 'capture', 'Shift', 'ShiftLeft', { command: true, shift: true })
    expect(box(wrapper, 'capture').text()).toBe(APPLE ? '⇧⌘…' : 'Ctrl+Shift+…')
    await press(wrapper, 'capture', 'R', 'KeyR', { command: true, shift: true })
    expect(box(wrapper, 'capture').text()).toBe(shown('Mod+Shift+R'))
    expect(useShortcutsStore().bindings.capture).toBe('Mod+Shift+R')
    expect(useShortcutsStore().recording).toBe(false)
    wrapper.unmount()
  })

  it('takes a chord from the action that had it, and says so', async () => {
    const wrapper = await section()
    await record(wrapper, 'export')
    await press(wrapper, 'export', 's', 'KeyS', { command: true })
    expect(box(wrapper, 'export').text()).toBe(shown('Mod+S'))
    expect(box(wrapper, 'save').text()).toBe('None')
    expect(row(wrapper, 'export').querySelector('[role="status"]')?.textContent?.trim()).toBe(
      `${shown('Mod+S')} was taken from “Save the capture”`,
    )

    // Restoring Save's default takes the chord back.
    const reset = row(wrapper, 'save').querySelector<HTMLElement>(
      '[aria-label="Restore the default shortcut of “Save the capture”"]',
    )
    reset?.click()
    await flushPromises()
    expect(box(wrapper, 'save').text()).toBe(shown('Mod+S'))
    expect(box(wrapper, 'export').text()).toBe('None')
    expect(row(wrapper, 'save').querySelector('[role="status"]')?.textContent?.trim()).toBe(
      'The default was taken back from “Export”',
    )
    wrapper.unmount()
  })

  it('cancels on Esc without closing the settings, and lets Space end a recording', async () => {
    const closed = vi.fn()
    window.addEventListener('keydown', closed)
    const wrapper = await section()
    await record(wrapper, 'fit')
    await press(wrapper, 'fit', 'Escape', 'Escape')
    expect(closed).not.toHaveBeenCalled()
    expect(box(wrapper, 'fit').text()).toBe('Home')
    expect(useShortcutsStore().recording).toBe(false)

    await record(wrapper, 'fit')
    await press(wrapper, 'fit', ' ', 'Space')
    // Letting Space go does not press the box again.
    const release = new KeyboardEvent('keyup', { key: ' ', code: 'Space', cancelable: true })
    box(wrapper, 'fit').element.dispatchEvent(release)
    expect(release.defaultPrevented).toBe(true)
    expect(box(wrapper, 'fit').text()).toBe('Space')
    window.removeEventListener('keydown', closed)
    wrapper.unmount()
  })

  it('lets Tab move on rather than record it', async () => {
    const wrapper = await section()
    await record(wrapper, 'search')
    const tab = new KeyboardEvent('keydown', { key: 'Tab', code: 'Tab', cancelable: true })
    box(wrapper, 'search').element.dispatchEvent(tab)
    await flushPromises()
    expect(tab.defaultPrevented).toBe(false)
    expect(useShortcutsStore().recording).toBe(false)
    expect(box(wrapper, 'search').text()).toBe('/')
    wrapper.unmount()
  })

  it('clears a shortcut and restores every default', async () => {
    const wrapper = await section()
    const clear = row(wrapper, 'zoomOut').querySelector<HTMLElement>(
      '[aria-label="Clear the shortcut of “Zoom out”"]',
    )
    clear?.click()
    await flushPromises()
    expect(box(wrapper, 'zoomOut').text()).toBe('None')

    const resetAll = wrapper.findAll('button').find((one) => one.text() === 'Restore all defaults')
    expect(resetAll?.attributes('disabled')).toBeUndefined()
    await resetAll?.trigger('click')
    expect(box(wrapper, 'zoomOut').text()).toBe('-')
    expect(resetAll?.attributes('disabled')).toBeDefined()
    wrapper.unmount()
  })
})
