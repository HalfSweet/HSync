import { mount } from '@vue/test-utils'
import { describe, expect, it } from 'vitest'

import { measureGeometry, type MeasureEnd } from '@/timeline/measure'

import MeasureOverlay from './MeasureOverlay.vue'

const view = { start: 0, samplesPerPixel: 10, width: 100 }
const rows: Record<string, { top: number; height: number }> = {
  d0: { top: 40, height: 30 },
  d3: { top: 100, height: 18 },
}
const rowOf = (key: string) => rows[key] ?? null
const end = (sample: number, rowKey: string, edge = true): MeasureEnd => ({ sample, rowKey, edge })

describe('edge measurement overlay', () => {
  it('marks both edges, drops to the second row and announces the read-out', () => {
    const wrapper = mount(MeasureOverlay, {
      props: {
        geometry: measureGeometry(end(100, 'd0'), end(300, 'd3'), view, rowOf),
        primary: 'Δt 9.80 µs',
        secondary: '245 samples',
        done: true,
        target: null,
      },
    })
    const dots = wrapper.findAll('[data-part="dot"]')
    expect(dots.map((dot) => dot.attributes('style'))).toEqual([
      'left: 10px; top: 55px;',
      'left: 30px; top: 109px;',
    ])
    expect(wrapper.find('.border-dashed').attributes('style')).toContain('height: 54px')
    const readout = wrapper.find('[role="status"]')
    expect(readout.text()).toBe('Δt 9.80 µs245 samples')
    expect(wrapper.find('[data-part="target"]').exists()).toBe(false)
  })

  it('draws a cursor for an end between edges and only hints until the measurement is done', () => {
    const wrapper = mount(MeasureOverlay, {
      props: {
        geometry: measureGeometry(end(100, 'd0'), end(300, 'd0', false), view, rowOf),
        primary: 'Δt 8.00 µs',
        secondary: 'Click another edge',
        done: false,
        target: null,
      },
    })
    expect(wrapper.findAll('[data-part="dot"]')).toHaveLength(1)
    expect(wrapper.find('[data-part="cursor"]').attributes('style')).toContain('height: 24px')
    expect(wrapper.find('[role="status"]').exists()).toBe(false)
    expect(wrapper.text()).toContain('Click another edge')
  })

  it('rings the edge a click would start from', () => {
    const wrapper = mount(MeasureOverlay, {
      props: {
        geometry: null,
        primary: null,
        secondary: null,
        done: false,
        target: { x: 12, y: 55 },
      },
    })
    expect(wrapper.find('[data-part="target"]').attributes('style')).toBe('left: 12px; top: 55px;')
  })
})
