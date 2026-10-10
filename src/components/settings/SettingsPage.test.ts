import { flushPromises, mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { i18n } from '@/i18n'
import { useSettingsStore } from '@/stores/settings'

import SettingsPage from './SettingsPage.vue'

const backend = {
  appVersion: () => Promise.resolve('0.2.0'),
  logFile: () => Promise.resolve('/logs/hsync.log'),
  revealLog: vi.fn(() => Promise.resolve()),
  updateCheck: vi.fn(() => Promise.resolve(null)),
  updateInstall: vi.fn(() => Promise.resolve()),
}
vi.mock('@/ipc/backend', () => ({ getBackend: () => Promise.resolve(backend) }))

async function page() {
  const settings = useSettingsStore()
  settings.open = true
  const wrapper = mount(SettingsPage, { global: { plugins: [i18n] }, attachTo: document.body })
  await flushPromises()
  return { wrapper, settings }
}

const tab = (wrapper: Awaited<ReturnType<typeof page>>['wrapper'], name: string) =>
  wrapper.findAll('[role="tab"]').find((one) => one.text() === name)

function escape(): void {
  window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' }))
}

describe('settings page', () => {
  beforeEach(() => {
    i18n.global.locale.value = 'en-US'
    setActivePinia(createPinia())
  })
  afterEach(() => {
    document.body.innerHTML = ''
  })

  it('shows the section chosen in the side bar, with focus on it', async () => {
    const { wrapper, settings } = await page()
    expect(document.activeElement?.textContent?.trim()).toBe('Appearance')
    expect(wrapper.find('[role="tabpanel"]').text()).toContain('Theme')

    await tab(wrapper, 'Behaviour')?.trigger('click')
    expect(settings.section).toBe('behaviour')
    expect(wrapper.findAll('[role="tabpanel"] input[type="checkbox"]')).toHaveLength(5)
    wrapper.unmount()
  })

  it('moves through the sections with the arrow keys, Home and End', async () => {
    const { wrapper, settings } = await page()
    const tablist = wrapper.find('[role="tablist"]')
    await tablist.trigger('keydown', { key: 'ArrowDown' })
    expect(settings.section).toBe('behaviour')
    await tablist.trigger('keydown', { key: 'End' })
    expect(settings.section).toBe('diagnostics')
    await tablist.trigger('keydown', { key: 'ArrowDown' })
    expect(settings.section).toBe('diagnostics')
    await tablist.trigger('keydown', { key: 'Home' })
    expect(settings.section).toBe('appearance')
    wrapper.unmount()
  })

  it('closes About first on Escape when it is open over the page', async () => {
    const { wrapper, settings } = await page()
    const about = wrapper.findAll('button').find((one) => one.text() === 'About HSync…')
    await about?.trigger('click')
    await flushPromises()
    expect(wrapper.text()).toContain('Version 0.2.0')

    escape()
    await flushPromises()
    expect(wrapper.text()).not.toContain('Version 0.2.0')
    expect(settings.open).toBe(true)

    escape()
    expect(settings.open).toBe(false)
    wrapper.unmount()
  })

  it('closes when the blurred interface around it is clicked', async () => {
    const { wrapper, settings } = await page()
    await wrapper.find('[role="dialog"]').trigger('click')
    expect(settings.open).toBe(true)
    await wrapper.trigger('click')
    expect(settings.open).toBe(false)
    wrapper.unmount()
  })
})
