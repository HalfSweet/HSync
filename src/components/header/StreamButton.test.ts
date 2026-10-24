import { mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import { beforeEach, describe, expect, it } from 'vitest'

import { i18n, setLocale } from '@/i18n'
import { useStreamStore } from '@/stores/stream'

import StreamButton from './StreamButton.vue'

describe('data stream button', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
    setLocale('en-US')
  })

  it('opens and closes the data stream, and says whether it is open', async () => {
    const wrapper = mount(StreamButton, { global: { plugins: [i18n] } })
    const stream = useStreamStore()
    expect(wrapper.attributes('aria-label')).toBe('Data stream')
    expect(wrapper.attributes('aria-pressed')).toBe('false')
    await wrapper.trigger('click')
    expect(stream.open).toBe(true)
    expect(wrapper.attributes('aria-pressed')).toBe('true')
    await wrapper.trigger('click')
    expect(stream.open).toBe(false)
  })
})
