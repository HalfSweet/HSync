import { mount } from '@vue/test-utils'
import { afterAll, beforeAll, describe, expect, it } from 'vitest'
import { defineComponent, h, nextTick, shallowRef, withDirectives } from 'vue'

import { vIndicator } from './indicator'

// jsdom lays nothing out: an element reports the box in its `data-box`, as "left,top,width,height".
const BOX = ['offsetLeft', 'offsetTop', 'offsetWidth', 'offsetHeight'] as const
const original = BOX.map((name) => Object.getOwnPropertyDescriptor(HTMLElement.prototype, name))
beforeAll(() => {
  BOX.forEach((name, at) =>
    Object.defineProperty(HTMLElement.prototype, name, {
      configurable: true,
      get(this: HTMLElement) {
        return Number((this.dataset.box ?? '0,0,0,0').split(',')[at])
      },
    }),
  )
})
afterAll(() => {
  BOX.forEach((name, at) => {
    const descriptor = original[at]
    if (descriptor) Object.defineProperty(HTMLElement.prototype, name, descriptor)
  })
})

/** Two options, A and B, in a control marked by `v-indicator` (with `arg`, `v-indicator:arg`). */
function control(arg?: string, attribute = 'aria-pressed') {
  const chosen = shallowRef<'a' | 'b' | null>('a')
  const Control = defineComponent(
    () => () =>
      withDirectives(
        h('div', { role: 'group' }, [
          h('button', { [attribute]: chosen.value === 'a', 'data-box': '2,2,40,22' }, 'A'),
          h('button', { [attribute]: chosen.value === 'b', 'data-box': '44,2,56,22' }, 'B'),
        ]),
        [[vIndicator, undefined, arg]],
      ),
  )
  const wrapper = mount(Control, { attachTo: document.body })
  return { chosen, wrapper, container: wrapper.element as HTMLElement }
}

/** Lets Vue patch the DOM and the mutation observer answer. */
async function settle(): Promise<void> {
  await nextTick()
  await new Promise((resolve) => setTimeout(resolve, 0))
}

const mark = (container: HTMLElement) =>
  ['x', 'y', 'w', 'h'].map((part) => container.style.getPropertyValue(`--mark-${part}`))

describe('v-indicator', () => {
  it('puts the mark under the selected option', () => {
    const { container, wrapper } = control()
    expect(container.dataset.indicator).toBe('face')
    expect(container.dataset.marked).toBe('')
    expect(mark(container)).toEqual(['2px', '2px', '40px', '22px'])
    wrapper.unmount()
  })

  it('moves the mark to the next selection', async () => {
    const { chosen, container, wrapper } = control()
    chosen.value = 'b'
    await settle()
    expect(mark(container)).toEqual(['44px', '2px', '56px', '22px'])
    wrapper.unmount()
  })

  it('takes the mark away while nothing is selected, and brings it back', async () => {
    const { chosen, container, wrapper } = control()
    chosen.value = null
    await settle()
    expect(container.dataset.marked).toBeUndefined()
    chosen.value = 'b'
    await settle()
    expect(container.dataset.marked).toBe('')
    expect(mark(container)[0]).toBe('44px')
    wrapper.unmount()
  })

  it('draws a line under the selected tab of a tab bar', async () => {
    const { chosen, container, wrapper } = control('line', 'aria-selected')
    expect(container.dataset.indicator).toBe('line')
    expect(mark(container)[0]).toBe('2px')
    chosen.value = 'b'
    await settle()
    expect(mark(container)[0]).toBe('44px')
    wrapper.unmount()
  })
})
