import { mount } from '@vue/test-utils'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { nextTick } from 'vue'

import { i18n } from '@/i18n'

import TraceGrip from './TraceGrip.vue'

function grip() {
  const wrapper = mount(TraceGrip, {
    props: { channel: 'SDA', height: 30 },
    global: { plugins: [i18n] },
  })
  // jsdom has no pointer capture.
  ;(wrapper.element as HTMLElement).setPointerCapture = vi.fn()
  return wrapper
}

/** Sends a pointer event; Vue Test Utils cannot set a mouse event's button or position. */
async function send(
  wrapper: ReturnType<typeof grip>,
  type: string,
  init: PointerEventInit,
): Promise<void> {
  wrapper.element.dispatchEvent(new PointerEvent(type, { bubbles: true, pointerId: 1, ...init }))
  await nextTick()
}

describe('trace grip', () => {
  beforeEach(() => {
    i18n.global.locale.value = 'en-US'
  })

  it('names the channel whose row it resizes', () => {
    const wrapper = grip()
    expect(wrapper.attributes('role')).toBe('separator')
    expect(wrapper.attributes('aria-label')).toBe('Drag to resize the waveform of SDA')
  })

  it('asks for the height the row had plus how far the pointer moved', async () => {
    const wrapper = grip()
    await send(wrapper, 'pointermove', { clientY: 140 })
    expect(wrapper.emitted('resize')).toBeUndefined()

    await send(wrapper, 'pointerdown', { button: 0, clientY: 100 })
    await send(wrapper, 'pointermove', { clientY: 124 })
    await send(wrapper, 'pointermove', { clientY: 90 })
    expect(wrapper.emitted('resize')).toEqual([[54], [20]])

    await send(wrapper, 'pointerup', { clientY: 90 })
    await send(wrapper, 'pointermove', { clientY: 160 })
    expect(wrapper.emitted('resize')).toHaveLength(2)
  })

  it('says when a drag ends, and only then', async () => {
    const wrapper = grip()
    await send(wrapper, 'pointerup', { clientY: 100 })
    expect(wrapper.emitted('end')).toBeUndefined()

    await send(wrapper, 'pointerdown', { button: 0, clientY: 100 })
    await send(wrapper, 'pointermove', { clientY: 130 })
    await send(wrapper, 'pointerup', { clientY: 130 })
    await send(wrapper, 'pointercancel', { clientY: 130 })
    expect(wrapper.emitted('end')).toHaveLength(1)
  })

  it('ignores other buttons', async () => {
    const wrapper = grip()
    await send(wrapper, 'pointerdown', { button: 2, clientY: 100 })
    await send(wrapper, 'pointermove', { clientY: 130 })
    expect(wrapper.emitted('resize')).toBeUndefined()
  })

  it('asks for the default height on a double-click', async () => {
    const wrapper = grip()
    await wrapper.trigger('dblclick')
    expect(wrapper.emitted('reset')).toHaveLength(1)
  })
})
