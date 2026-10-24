import { afterEach, describe, expect, it, vi } from 'vitest'

import { drawerEnter, drawerLeave } from './drawer'

/** A panel whose animations are recorded, finishing when `finish` is called. */
function panel(width: number) {
  const element = document.createElement('aside')
  Object.defineProperty(element, 'offsetWidth', { value: width })
  element.getBoundingClientRect = () => ({ width }) as DOMRect
  const calls: { frames: Keyframe[]; options: KeyframeAnimationOptions }[] = []
  let finish = () => undefined as void
  element.animate = vi.fn((frames: Keyframe[], options: KeyframeAnimationOptions) => {
    calls.push({ frames, options })
    const finished = new Promise<Animation>((resolve) => {
      finish = () => resolve({} as Animation)
    })
    return { finished, cancel: vi.fn() } as unknown as Animation
  })
  return { element, calls, finish: () => finish() }
}

describe('data stream panel motion', () => {
  afterEach(() => {
    document.documentElement.style.removeProperty('--motion-enter')
    document.documentElement.style.removeProperty('--motion-exit')
  })

  it('grows from nothing to its width as it opens, and shrinks back as it closes', async () => {
    document.documentElement.style.setProperty('--motion-enter', '200ms')
    document.documentElement.style.setProperty('--motion-exit', '120ms')
    const { element, calls, finish } = panel(400)
    const opened = vi.fn()
    drawerEnter(element, opened)
    expect(calls[0]?.frames).toEqual([{ width: '0px' }, { width: '400px' }])
    expect(calls[0]?.options.duration).toBe(200)
    finish()
    await Promise.resolve()
    await Promise.resolve()
    expect(opened).toHaveBeenCalledOnce()

    const closed = vi.fn()
    drawerLeave(element, closed)
    expect(element.style.pointerEvents).toBe('none')
    expect(calls[1]?.frames).toEqual([{ width: '400px' }, { width: '0px' }])
    expect(calls[1]?.options).toMatchObject({ duration: 120, fill: 'forwards' })
  })

  it('changes at once with reduced motion', () => {
    const { element, calls } = panel(400)
    const opened = vi.fn()
    drawerEnter(element, opened)
    expect(opened).toHaveBeenCalledOnce()
    expect(calls).toHaveLength(0)
  })
})
