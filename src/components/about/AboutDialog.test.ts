import { flushPromises, mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import { i18n } from '@/i18n'
import type { AvailableUpdate, UpdateProgress } from '@/ipc/backend'
import { useUpdateStore } from '@/stores/update'

import AboutDialog from './AboutDialog.vue'

const backend = {
  appVersion: () => Promise.resolve('0.2.0'),
  updateCheck: vi.fn<() => Promise<AvailableUpdate | null>>(),
  updateInstall: vi.fn<(onProgress: (progress: UpdateProgress) => void) => Promise<void>>(),
}
vi.mock('@/ipc/backend', () => ({ getBackend: () => Promise.resolve(backend) }))

async function dialog() {
  const wrapper = mount(AboutDialog, { global: { plugins: [i18n] } })
  await flushPromises()
  return wrapper
}

describe('about dialog', () => {
  beforeEach(() => {
    i18n.global.locale.value = 'en-US'
    setActivePinia(createPinia())
    backend.updateCheck.mockReset()
    backend.updateInstall.mockReset()
  })

  it('checks for updates on request and says when this is the latest', async () => {
    const wrapper = await dialog()
    expect(wrapper.text()).toContain('Version 0.2.0')
    backend.updateCheck.mockResolvedValueOnce(null)
    const button = wrapper.findAll('button').find((one) => one.text() === 'Check for updates')
    await button?.trigger('click')
    await flushPromises()
    expect(backend.updateCheck).toHaveBeenCalledTimes(1)
    expect(wrapper.find('[role="status"]').text()).toBe('HSync is up to date')
  })

  it('offers the release found and shows the download', async () => {
    backend.updateCheck.mockResolvedValueOnce({ version: '0.3.0' })
    await useUpdateStore().check(true)
    const wrapper = await dialog()
    expect(wrapper.text()).toContain('Version 0.3.0 is available')
    let report: (progress: UpdateProgress) => void = () => {}
    backend.updateInstall.mockImplementationOnce(
      (onProgress) =>
        new Promise(() => {
          report = onProgress
        }),
    )
    const install = wrapper.findAll('button').find((one) => one.text() === 'Download and restart')
    await install?.trigger('click')
    await flushPromises()
    expect(wrapper.text()).toContain('Downloading…')
    report({ done: 512, total: 2048 })
    await flushPromises()
    expect(wrapper.text()).toContain('Downloading 25%')
    expect(wrapper.text()).not.toContain('Download and restart')
  })

  it('shows why installing failed and offers it again', async () => {
    backend.updateCheck.mockResolvedValueOnce({ version: '0.3.0' })
    await useUpdateStore().check(true)
    backend.updateInstall.mockRejectedValueOnce({
      code: 'error.update.install',
      params: { detail: 'bad signature' },
    })
    const wrapper = await dialog()
    const install = wrapper.findAll('button').find((one) => one.text() === 'Download and restart')
    await install?.trigger('click')
    await flushPromises()
    expect(wrapper.find('[role="alert"]').text()).toBe('Could not update: bad signature')
    expect(wrapper.text()).toContain('Download and restart')
  })
})
