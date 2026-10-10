import { mount } from '@vue/test-utils'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { h } from 'vue'

import AnimatedHeight from './AnimatedHeight.vue'

// jsdom has no layout, resize observer or Web Animations. Here the content (what holds the
// slot's <p>) is `contentHeight` tall and the box as tall as its `data-h`; the observer is
// called by hand, and `animate` records what it is asked to play.
let contentHeight = 100
let notify: () => void = () => undefined
class FakeResizeObserver {
  constructor(callback: () => void) {
    notify = callback
  }
  observe(): void {}
  disconnect(): void {}
}

interface Played {
  keyframes: Keyframe[]
  options: KeyframeAnimationOptions
  cancel: ReturnType<typeof vi.fn>
  finish: () => void
}
let played: Played[] = []
const offsetHeight = Object.getOwnPropertyDescriptor(HTMLElement.prototype, 'offsetHeight')

beforeEach(() => {
  contentHeight = 100
  played = []
  vi.stubGlobal('ResizeObserver', FakeResizeObserver)
  Object.defineProperty(HTMLElement.prototype, 'offsetHeight', {
    configurable: true,
    get(this: HTMLElement) {
      if (this.firstElementChild?.tagName === 'P') return contentHeight
      return Number(this.dataset.h ?? 0)
    },
  })
  HTMLElement.prototype.animate = function (keyframes, options) {
    let finish: () => void = () => undefined
    const cancel = vi.fn()
    const finished = new Promise((resolve, reject) => {
      finish = () => resolve(undefined)
      cancel.mockImplementation(() => reject(new DOMException('cancelled', 'AbortError')))
    })
    played.push({
      keyframes: keyframes as Keyframe[],
      options: options as KeyframeAnimationOptions,
      cancel,
      finish,
    })
    return { cancel, finished } as unknown as Animation
  }
  document.documentElement.style.setProperty('--motion-enter', '200ms')
  document.documentElement.style.setProperty('--ease-enter', 'ease-out')
})

afterEach(() => {
  vi.unstubAllGlobals()
  delete (HTMLElement.prototype as Partial<HTMLElement>).animate
  if (offsetHeight) Object.defineProperty(HTMLElement.prototype, 'offsetHeight', offsetHeight)
  document.documentElement.removeAttribute('style')
})

function box() {
  const wrapper = mount(AnimatedHeight, { slots: { default: () => h('p', 'content') } })
  return { wrapper, outer: wrapper.element as HTMLElement }
}

describe('animated height', () => {
  it('eases from the old height to the new one, clipping meanwhile', async () => {
    const { wrapper, outer } = box()
    contentHeight = 160
    notify()
    expect(played).toHaveLength(1)
    expect(played[0]?.keyframes).toEqual([{ height: '100px' }, { height: '160px' }])
    expect(played[0]?.options).toEqual({ duration: 200, easing: 'ease-out' })
    expect(outer.style.overflow).toBe('clip')
    played[0]?.finish()
    await Promise.resolve()
    expect(outer.style.overflow).toBe('')
    wrapper.unmount()
  })

  it('leaves the box alone while the content keeps its height', () => {
    const { wrapper } = box()
    notify()
    expect(played).toHaveLength(0)
    wrapper.unmount()
  })

  it('starts from wherever the box has got to when the content changes again', () => {
    const { wrapper, outer } = box()
    contentHeight = 160
    notify()
    outer.dataset.h = '130'
    contentHeight = 80
    notify()
    expect(played[0]?.cancel).toHaveBeenCalled()
    expect(played[1]?.keyframes).toEqual([{ height: '130px' }, { height: '80px' }])
    wrapper.unmount()
  })

  it('changes at once with reduced motion', () => {
    document.documentElement.style.setProperty('--motion-enter', '0s')
    const { wrapper, outer } = box()
    contentHeight = 160
    notify()
    expect(played).toHaveLength(0)
    expect(outer.style.overflow).toBe('')
    wrapper.unmount()
  })
})
