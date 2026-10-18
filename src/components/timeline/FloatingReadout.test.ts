import { mount } from '@vue/test-utils'
import { describe, expect, it } from 'vitest'

import FloatingReadout from './FloatingReadout.vue'

const props = {
  anchor: { x: 300, align: 'beside', top: 200, bottom: 230, side: 'above' },
  bounds: { left: 0, right: 800, top: 38, bottom: 600 },
} as const

describe('floating read-out', () => {
  it('reads out beside the pointer on a see-through fill', () => {
    const wrapper = mount(FloatingReadout, {
      props: { ...props, primary: 'High 9.8 µs', secondary: 'Period 20.0 µs' },
    })
    expect(wrapper.text()).toBe('High 9.8 µsPeriod 20.0 µs')
    expect(wrapper.classes()).toContain('bg-float')
    expect(wrapper.classes()).toContain('pointer-events-none')
    expect(wrapper.attributes('style')).toBe('left: 312px; top: 200px;')
    expect(wrapper.attributes('role')).toBeUndefined()
  })

  it('announces a finished measurement, and shows the hint alone in full ink', async () => {
    const wrapper = mount(FloatingReadout, {
      props: { ...props, primary: null, secondary: 'Click another edge' },
    })
    expect(wrapper.find('span').classes()).toContain('text-ink')
    await wrapper.setProps({ primary: 'Δt 9.80 µs', secondary: '245 samples', announce: true })
    expect(wrapper.attributes('role')).toBe('status')
    expect(wrapper.findAll('span').map((one) => one.classes())[1]).toContain('text-ink-2')
  })
})
