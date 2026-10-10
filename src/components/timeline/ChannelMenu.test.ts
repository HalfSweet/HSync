import { mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { nextTick } from 'vue'

import { i18n } from '@/i18n'
import { useCaptureStore } from '@/stores/capture'
import { useChannelsStore } from '@/stores/channels'
import type { ChannelRef } from '@/timeline/groups'

import ChannelLabel from './ChannelLabel.vue'
import ChannelMenu from './ChannelMenu.vue'

vi.mock('@/ipc/backend', () => ({ getBackend: () => Promise.resolve({}) }))

const scl: ChannelRef = { index: 1, label: 'SCL', numeric: false, wire: 1, role: 'scl' }

function menu() {
  const capture = useCaptureStore()
  const renamed: [number, string][] = []
  capture.renameChannel = (index: number, name: string) => {
    renamed.push([index, name])
    return Promise.resolve()
  }
  const wrapper = mount(ChannelMenu, {
    props: { channel: scl, x: 10, y: 10 },
    global: { plugins: [i18n] },
    attachTo: document.body,
  })
  return { wrapper, renamed }
}

describe('channel menu', () => {
  beforeEach(() => setActivePinia(createPinia()))

  it('keeps the name typed when it closes, as by a click elsewhere', async () => {
    const { wrapper, renamed } = menu()
    await wrapper.find('input').setValue('Clock')
    wrapper.unmount()
    expect(renamed).toEqual([[1, 'Clock']])
  })

  it('drops the name on Escape', async () => {
    const { wrapper, renamed } = menu()
    await wrapper.find('input').setValue('Clock')
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' }))
    await nextTick()
    expect(wrapper.emitted('close')).toHaveLength(1)
    wrapper.unmount()
    expect(renamed).toEqual([])
  })

  it('closes on Enter and leaves an unchanged name alone', async () => {
    const { wrapper, renamed } = menu()
    await wrapper.find('input').trigger('keydown', { key: 'Enter' })
    expect(wrapper.emitted('close')).toHaveLength(1)
    wrapper.unmount()
    expect(renamed).toEqual([])
  })

  it('sets the colour at once', async () => {
    const { wrapper } = menu()
    await wrapper.findAll('[role="radio"]')[6]?.trigger('click')
    expect(useChannelsStore().colours[1]).toBe(6)
    wrapper.unmount()
  })
})

describe('channel label', () => {
  beforeEach(() => setActivePinia(createPinia()))

  it('opens the menu from the dot, a double-click on the name or a right-click', async () => {
    const wrapper = mount(ChannelLabel, {
      props: {
        channel: { ...scl, label: 'Clock', roleLabel: 'SCL' },
        condition: 'none',
        menuOpen: false,
      },
      global: { plugins: [i18n] },
    })
    expect(wrapper.text()).toContain('Clock')
    expect(wrapper.text()).toContain('SCL')
    await wrapper.find('button').trigger('click')
    await wrapper
      .findAll('span')
      .find((span) => span.text() === 'Clock')
      ?.trigger('dblclick')
    await wrapper.trigger('contextmenu')
    expect(wrapper.emitted('menu')).toHaveLength(3)
  })
})
