import { flushPromises, mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { activeCatalog, DEFAULT_CATALOG } from '@/decode/catalog'
import { i18n } from '@/i18n'
import type { AvailableUpdate } from '@/ipc/backend'
import type { CatalogEntry } from '@/ipc/bindings/CatalogEntry'
import type { ModuleInfo } from '@/ipc/bindings/ModuleInfo'
import { useDecodeStore } from '@/stores/decode'
import { usePreferencesStore } from '@/stores/preferences'
import { useSettingsStore } from '@/stores/settings'
import { useUpdateStore } from '@/stores/update'

import SettingsPage from './SettingsPage.vue'

const backend = {
  appVersion: () => Promise.resolve('0.2.0'),
  logFile: () => Promise.resolve('/logs/hsync.log'),
  revealLog: vi.fn(() => Promise.resolve()),
  updateCheck: vi.fn<() => Promise<AvailableUpdate | null>>(() => Promise.resolve(null)),
  updateInstall: vi.fn(() => Promise.resolve()),
  decodeCatalog: vi.fn<() => Promise<CatalogEntry[]>>(() => Promise.resolve([])),
  decodeLoadModule: vi.fn<(path: string) => Promise<ModuleInfo>>((path) =>
    Promise.resolve({ name: 'uart', path, category: 'bus', shadows: true }),
  ),
  decodeUnloadModule: vi.fn(() => Promise.resolve(true)),
}
vi.mock('@/ipc/backend', () => ({ getBackend: () => Promise.resolve(backend) }))
const picked = vi.fn<() => Promise<string | null>>(() => Promise.resolve(null))
vi.mock('@/ipc/dialogs', () => ({ pickFile: () => picked() }))

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
    activeCatalog.value = DEFAULT_CATALOG
  })

  it('takes focus without ringing a section, and shows the one chosen in the side bar', async () => {
    const { wrapper, settings } = await page()
    expect(document.activeElement?.getAttribute('role')).toBe('dialog')
    expect(wrapper.find('[role="tabpanel"]').text()).toContain('Theme')

    await tab(wrapper, 'Behaviour')?.trigger('click')
    expect(settings.section).toBe('behaviour')
    expect(wrapper.findAll('[role="tabpanel"] input[type="checkbox"]')).toHaveLength(6)
    wrapper.unmount()
  })

  it('turns on zooming with the wheel alone from Behaviour', async () => {
    localStorage.clear()
    const { wrapper, settings } = await page()
    settings.section = 'behaviour'
    await flushPromises()
    const option = wrapper
      .findAll('[role="tabpanel"] label')
      .find((one) => one.text() === 'Zoom the waveform with the wheel alone')
    expect(usePreferencesStore().wheelZoom).toBe(false)
    await option?.find('input').setValue(true)
    expect(usePreferencesStore().wheelZoom).toBe(true)
    wrapper.unmount()
  })

  it('floats the read-outs over the waveforms from Appearance', async () => {
    localStorage.clear()
    const { wrapper } = await page()
    const group = wrapper.find('[role="group"][aria-label="Readout position"]')
    const buttons = group.findAll('button')
    expect(buttons.map((one) => one.text())).toEqual(['Status bar', 'Floating'])
    expect(buttons[0]?.attributes('aria-pressed')).toBe('true')
    await buttons[1]?.trigger('click')
    expect(usePreferencesStore().readoutPlace).toBe('float')
    expect(buttons[1]?.attributes('aria-pressed')).toBe('true')
    wrapper.unmount()
  })

  it('loads decoder modules from Decoders, remembers them and lets them go', async () => {
    localStorage.clear()
    const { wrapper, settings } = await page()
    await tab(wrapper, 'Decoders')?.trigger('click')
    expect(settings.section).toBe('decoders')
    const panel = () => wrapper.find('[role="tabpanel"]')
    expect(panel().text()).toContain('No decoder module is loaded')

    picked.mockResolvedValueOnce('/decoders/uart.json')
    const load = panel()
      .findAll('button')
      .find((one) => one.text() === 'Load module…')
    await load?.trigger('click')
    await flushPromises()
    expect(backend.decodeLoadModule).toHaveBeenCalledWith('/decoders/uart.json')
    expect(panel().text()).toContain('UART')
    expect(panel().text()).toContain('Stands in for the built-in UART decoder')
    expect(usePreferencesStore().decoderModules).toEqual(['/decoders/uart.json'])
    expect(useDecodeStore().modules.map((entry) => entry.info?.name)).toEqual(['uart'])

    backend.decodeLoadModule.mockRejectedValueOnce({
      code: 'error.decode.module',
      params: { path: '/decoders/bad.json', detail: 'the module does not parse' },
    })
    picked.mockResolvedValueOnce('/decoders/bad.json')
    await load?.trigger('click')
    await flushPromises()
    expect(panel().find('[role="alert"]').text()).toBe(
      'The decoder module cannot be loaded: the module does not parse',
    )
    expect(usePreferencesStore().decoderModules).toEqual(['/decoders/uart.json'])

    const remove = panel()
      .findAll('button')
      .filter((one) => one.text() === 'Remove')
    expect(remove).toHaveLength(2)
    await remove[0]?.trigger('click')
    await flushPromises()
    expect(backend.decodeUnloadModule).toHaveBeenCalledWith('uart')
    expect(usePreferencesStore().decoderModules).toEqual([])
    expect(panel().text()).not.toContain('UART')
    wrapper.unmount()
  })

  it('names a loaded module by the copy it carries', async () => {
    localStorage.clear()
    const sensor: CatalogEntry = {
      ...(DEFAULT_CATALOG[0] as CatalogEntry),
      name: 'my-sensor',
      module: '/decoders/my-sensor.json',
      locales: {
        'en-US': { name: 'My sensor', fields: {}, values: {}, events: {}, eventFields: {} },
      },
    }
    backend.decodeCatalog.mockResolvedValueOnce([...DEFAULT_CATALOG, sensor])
    backend.decodeLoadModule.mockResolvedValueOnce({
      name: 'my-sensor',
      path: '/decoders/my-sensor.json',
      category: 'line',
      shadows: false,
    })
    const { wrapper, settings } = await page()
    settings.section = 'decoders'
    await flushPromises()
    picked.mockResolvedValueOnce('/decoders/my-sensor.json')
    await wrapper
      .findAll('[role="tabpanel"] button')
      .find((one) => one.text() === 'Load module…')
      ?.trigger('click')
    await flushPromises()
    expect(wrapper.find('[role="tabpanel"]').text()).toContain('My sensor')
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
