import { mount } from '@vue/test-utils'
import { beforeEach, describe, expect, it } from 'vitest'

import { i18n, setLocale } from '@/i18n'
import type { LaneBlock } from '@/ipc/bindings/LaneBlock'

import DecodeLane from './DecodeLane.vue'

function lane(blocks: LaneBlock[]) {
  return mount(DecodeLane, {
    global: { plugins: [i18n] },
    props: {
      kind: 'jtag',
      view: { blocks, truncated: false },
      viewport: { start: 0, samplesPerPixel: 3, width: 1000 },
      decoder: 0,
      stream: 0,
      selected: null,
      timeOf: (sample) => sample / 50_000_000,
      format: 'hex',
    },
  })
}

describe('decode lane', () => {
  beforeEach(() => setLocale('en-US'))

  it('shows JTAG data rather than scan identifiers and offsets', () => {
    const wrapper = lane([
      {
        type: 'event',
        index: 3,
        start: 600,
        end: 1500,
        kind: 'shift',
        role: 'data',
        payload: { kind: 'shift', scan: 2, offset: 0, tdi: [0xa5], tdo: [0x3c], bits: 8 },
      },
    ])
    expect(wrapper.find('.val').text()).toBe('A5 · 3C')
    expect(wrapper.find('button').attributes('title')).toContain('Scan: 2')
  })

  it('keeps single-sample and zero-duration events visible with accessible details', () => {
    const wrapper = lane([
      {
        type: 'event',
        index: 0,
        start: 300,
        end: 301,
        kind: 'state',
        role: 'data',
        payload: { kind: 'state', state: 'shift-dr' },
      },
      {
        type: 'event',
        index: 1,
        start: 600,
        end: 600,
        kind: 'scan-end',
        role: 'framing',
        payload: { kind: 'scan-end', scan: 2, bits: 8, reason: 'aborted' },
      },
    ])
    for (const button of wrapper.findAll('button')) {
      expect(button.attributes('style')).toContain('width: 2px')
      expect(button.attributes('aria-label')).toBe(button.attributes('title'))
      expect(button.text()).toBe('')
    }
  })

  it('keeps every coincident event in the marker details through updates', async () => {
    const state: LaneBlock = {
      type: 'event',
      index: 0,
      start: 300,
      end: 301,
      kind: 'state',
      role: 'data',
      payload: { kind: 'state', state: 'shift-dr' },
    }
    const begin: LaneBlock = {
      type: 'event',
      index: 1,
      start: 300,
      end: 301,
      kind: 'scan-begin',
      role: 'framing',
      payload: { kind: 'scan-begin', scan: 2, register: 'dr' },
    }
    const wrapper = lane([state, begin])
    expect(wrapper.findAll('button')).toHaveLength(1)
    const marker = wrapper.find('button')
    expect(marker.attributes('title')).toContain('Shift-DR')
    expect(marker.attributes('title')).toContain('Scan: 2')
    await marker.trigger('click')
    expect(wrapper.emitted('pick')?.[0]?.[0]).toMatchObject({ start: 300, end: 301 })

    await wrapper.setProps({ view: { blocks: [begin], truncated: false } })
    expect(wrapper.findAll('button')).toHaveLength(1)
    expect(wrapper.find('button').attributes('title')).not.toContain('Shift-DR')
    expect(wrapper.find('button').attributes('title')).toContain('Scan: 2')
  })

  it('labels a state interval and selects its whole duration independently of data', async () => {
    const wrapper = lane([
      {
        type: 'event',
        index: 0,
        start: 300,
        end: 1500,
        kind: 'state',
        role: 'command',
        payload: { kind: 'state', state: 'shift-ir' },
      },
    ])
    await wrapper.setProps({ annotation: 'states' })
    expect(wrapper.find('button').text()).toBe('Shift-IR')
    expect(wrapper.find('button').attributes('style')).toContain('width: 399px')
    expect(wrapper.find('button').attributes('title')).toContain('duration 24.0 µs')
    await wrapper.find('button').trigger('click')
    expect(wrapper.emitted('pick')?.[0]?.[0]).toEqual({
      id: '0:0:states:event:300:1500',
      start: 300,
      end: 1500,
    })
    await wrapper.setProps({ selected: '0:0:event:300:1500' })
    expect(wrapper.find('button').attributes('aria-pressed')).toBe('false')
    await wrapper.setProps({ selected: '0:0:states:event:300:1500' })
    expect(wrapper.find('button').attributes('aria-pressed')).toBe('true')
    setLocale('zh-CN')
    await wrapper.vm.$nextTick()
    expect(wrapper.find('button').text()).toBe('指令移位')
    expect(wrapper.find('button').attributes('title')).not.toContain('shift-ir')
  })

  it('keeps an abbreviated state name visible in a narrow interval', async () => {
    const wrapper = lane([
      {
        type: 'event',
        index: 0,
        start: 300,
        end: 435,
        kind: 'state',
        role: 'command',
        payload: { kind: 'state', state: 'capture-ir' },
      },
    ])
    await wrapper.setProps({ annotation: 'states' })
    expect(wrapper.find('button').classes()).not.toContain('blk-mark')
    expect(wrapper.find('.val').text()).toBe('Capture-IR')
    expect(wrapper.find('button').attributes('aria-label')).toContain('Capture-IR')
  })

  it.each(['cjtag', 'jtag-chain'])('localizes shared TAP state intervals for %s', async (kind) => {
    const wrapper = lane([
      {
        type: 'event',
        index: 0,
        start: 0,
        end: 600,
        kind: 'state',
        role: 'command',
        payload: { kind: 'state', state: 'shift-ir' },
      },
    ])
    await wrapper.setProps({ kind, annotation: 'states' })
    expect(wrapper.find('button').text()).toBe('Shift-IR')
    setLocale('zh-CN')
    await wrapper.vm.$nextTick()
    expect(wrapper.find('button').text()).toBe('指令移位')
    expect(wrapper.find('button').attributes('title')).not.toContain('shift-ir')
  })

  it('colours blocks by role and shows text wherever it fits', async () => {
    const wrapper = lane([
      {
        type: 'event',
        index: 0,
        start: 0,
        end: 210,
        kind: 'shift',
        role: 'command',
        payload: { kind: 'shift', scan: 1, offset: 0, tdi: [0x0e], tdo: [0x01], bits: 4 },
      },
      {
        type: 'event',
        index: 1,
        start: 300,
        end: 330,
        kind: 'shift',
        role: 'data',
        payload: { kind: 'shift', scan: 2, offset: 0, tdi: [0xff], tdo: [0x00], bits: 8 },
      },
    ])
    // 69 px is less than 3.2% of a 3000 px lane, and still room for both values.
    await wrapper.setProps({ viewport: { start: 0, samplesPerPixel: 3, width: 3000 } })
    const [wide, narrow] = wrapper.findAll('button')
    expect(wide?.classes()).toContain('role-command')
    expect(wide?.classes()).not.toContain('blk-mark')
    expect(wide?.find('.val').text()).toBe('0E · 01')
    expect(narrow?.classes()).toEqual(expect.arrayContaining(['role-data', 'blk-mark']))
    expect(narrow?.text()).toBe('')
  })

  it('marks an error across the lane at every width', () => {
    const wrapper = lane([
      {
        type: 'event',
        index: 0,
        start: 300,
        end: 301,
        kind: 'scan-end',
        role: 'error',
        payload: { kind: 'scan-end', scan: 2, bits: 8, reason: 'aborted' },
      },
    ])
    const mark = wrapper.find('button')
    expect(mark.classes()).toEqual(expect.arrayContaining(['role-error', 'blk-alarm']))
    expect(mark.attributes('style')).toContain('width: 3px')
  })

  it('paints a dense block by the role of each stretch', () => {
    const wrapper = lane([
      {
        type: 'dense',
        start: 0,
        end: 300,
        count: 40,
        parts: [
          { start: 0, end: 60, role: 'address' },
          { start: 60, end: 240, role: 'data' },
          { start: 240, end: 246, role: 'error' },
        ],
      },
    ])
    const parts = wrapper.findAll('.part')
    expect(parts.map((part) => part.classes().find((name) => name.startsWith('role-')))).toEqual([
      'role-address',
      'role-data',
      'role-error',
    ])
    expect(parts[1]?.attributes('style')).toContain('left: 20px; width: 60px')
    expect(parts[2]?.classes()).toContain('part-alarm')
    expect(wrapper.find('.dense').attributes('title')).toContain('40')
  })

  it('fills a shift state deeper than the states around it', async () => {
    const state = (index: number, start: number, end: number, name: string): LaneBlock => ({
      type: 'event',
      index,
      start,
      end,
      kind: 'state',
      role: 'data',
      payload: { kind: 'state', state: name },
    })
    const wrapper = lane([state(0, 0, 300, 'capture-dr'), state(1, 300, 900, 'shift-dr')])
    await wrapper.setProps({ annotation: 'states' })
    const [capture, shift] = wrapper.findAll('button')
    expect(capture?.classes()).not.toContain('blk-deep')
    expect(shift?.classes()).toContain('blk-deep')
  })
})
