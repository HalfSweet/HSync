import { flushPromises, mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { i18n } from '@/i18n'
import type { AvailableUpdate } from '@/ipc/backend'
import { useSettingsStore } from '@/stores/settings'
import { useUpdateStore } from '@/stores/update'

import SettingsPage from './SettingsPage.vue'

const backend = {
  appVersion: () => Promise.resolve('0.2.0'),
  logFile: () => Promise.resolve('/logs/hsync.log'),
  revealLog: vi.fn(() => Promise.resolve()),
  updateCheck: vi.fn<() => Promise<AvailableUpdate | null>>(() => Promise.resolve(null)),
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
  wrapper.findAll('[role="tab"]').find((one) => one.text().startsWith(name))

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

  it('takes focus without ringing a section, and shows the one chosen in the side bar', async () => {
    const { wrapper, settings } = await page()
    expect(document.activeElement?.getAttribute('role')).toBe('dialog')
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
    expect(settings.section).toBe('about')
    await tablist.trigger('keydown', { key: 'ArrowDown' })
    expect(settings.section).toBe('about')
    await tablist.trigger('keydown', { key: 'ArrowUp' })
    expect(settings.section).toBe('diagnostics')
    await tablist.trigger('keydown', { key: 'Home' })
    expect(settings.section).toBe('appearance')
    wrapper.unmount()
  })

  it('shows About as the last section, at the foot of the side bar', async () => {
    const { wrapper, settings } = await page()
    const about = tab(wrapper, 'About')
    expect(wrapper.findAll('[role="tab"]').at(-1)?.element).toBe(about?.element)
    expect(about?.classes()).toContain('mt-auto')

    await about?.trigger('click')
    await flushPromises()
    expect(settings.section).toBe('about')
    expect(wrapper.find('[role="tabpanel"]').text()).toContain('Version 0.2.0')
    expect(wrapper.find('[role="tabpanel"]').text()).toContain('Check for updates')
    wrapper.unmount()
  })

  it('marks About when a newer release is out', async () => {
    backend.updateCheck.mockResolvedValueOnce({ version: '0.3.0' })
    await useUpdateStore().check(true)
    const { wrapper } = await page()
    const about = tab(wrapper, 'About')
    expect(about?.attributes('title')).toBe('Version 0.3.0 available')
    expect(about?.text()).toContain('Version 0.3.0 available')
    expect(tab(wrapper, 'Appearance')?.attributes('title')).toBeUndefined()
    wrapper.unmount()
  })

  it('closes on Escape and when the blurred interface around it is clicked', async () => {
    const first = await page()
    escape()
    expect(first.settings.open).toBe(false)
    first.wrapper.unmount()

    const { wrapper, settings } = await page()
    await wrapper.find('[role="dialog"]').trigger('click')
    expect(settings.open).toBe(true)
    await wrapper.trigger('click')
    expect(settings.open).toBe(false)
    wrapper.unmount()
  })
})
