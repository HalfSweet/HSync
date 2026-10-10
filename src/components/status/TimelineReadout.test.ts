import { mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { nextTick } from 'vue'

import { useViewStore } from '@/stores/view'

import TimelineReadout from './TimelineReadout.vue'

vi.mock('@/ipc/backend', () => ({ getBackend: () => Promise.resolve({}) }))

const d5 = { index: 5, label: 'D5', numeric: true, wire: 5 }

describe('timeline read-out in the status bar', () => {
  beforeEach(() => setActivePinia(createPinia()))

  it('names the channel of the pulse under the pointer', async () => {
    const wrapper = mount(TimelineReadout)
    expect(wrapper.text()).toBe('')
    useViewStore().setReadouts({
      pulse: { channel: d5, primary: 'Low 23.0 µs', secondary: 'Period 53.8 µs' },
      measure: null,
    })
    await nextTick()
    const pulse = wrapper.find('[data-part="pulse"]')
    expect(pulse.classes()).toContain('wire-5')
    expect(pulse.text()).toBe('D5Low 23.0 µsPeriod 53.8 µs')
    expect(pulse.find('.font-mono').text()).toBe('D5')
  })

  it('announces the measurement once both edges are chosen, before the pulse', async () => {
    const view = useViewStore()
    view.setReadouts({
      pulse: null,
      measure: { primary: null, secondary: 'Click another edge', done: false },
    })
    const wrapper = mount(TimelineReadout)
    expect(wrapper.find('[role="status"]').exists()).toBe(false)
    expect(wrapper.text()).toBe('Click another edge')
    view.setReadouts({
      pulse: { channel: d5, primary: 'High 9.8 µs', secondary: null },
      measure: { primary: 'Δt 9.80 µs', secondary: '245 samples', done: true },
    })
    await nextTick()
    expect(wrapper.find('[role="status"]').text()).toBe('Δt 9.80 µs245 samples')
    const parts = wrapper.findAll('[data-part]').map((part) => part.attributes('data-part'))
    expect(parts).toEqual(['measure', 'pulse'])
  })
})
