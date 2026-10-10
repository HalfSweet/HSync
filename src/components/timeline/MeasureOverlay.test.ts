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
  it('marks both edges and drops to the second row', () => {
    const wrapper = mount(MeasureOverlay, {
      props: {
        geometry: measureGeometry(end(100, 'd0'), end(300, 'd3'), view, rowOf),
        target: null,
      },
    })
    const dots = wrapper.findAll('[data-part="dot"]')
    expect(dots.map((dot) => dot.attributes('style'))).toEqual([
      'left: 10px; top: 55px;',
      'left: 30px; top: 109px;',
    ])
    expect(wrapper.find('.border-dashed').attributes('style')).toContain('height: 54px')
    expect(wrapper.find('[data-part="target"]').exists()).toBe(false)
    // The read-out is the status bar's, so nothing covers the rows.
    expect(wrapper.text()).toBe('')
  })

  it('draws a cursor for an end between edges', () => {
    const wrapper = mount(MeasureOverlay, {
      props: {
        geometry: measureGeometry(end(100, 'd0'), end(300, 'd0', false), view, rowOf),
        target: null,
      },
    })
    expect(wrapper.findAll('[data-part="dot"]')).toHaveLength(1)
    expect(wrapper.find('[data-part="cursor"]').attributes('style')).toContain('height: 24px')
  })

  it('rings the edge a click would start from', () => {
    const wrapper = mount(MeasureOverlay, {
      props: { geometry: null, target: { x: 12, y: 55 } },
    })
    expect(wrapper.find('[data-part="target"]').attributes('style')).toBe('left: 12px; top: 55px;')
  })
})
