import { afterAll, beforeAll, describe, expect, it } from 'vitest'

import { holdInPlace } from './list'

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

describe('holdInPlace', () => {
  it('pins a leaving entry where it stands and marks it as leaving', () => {
    const entry = document.createElement('button')
    entry.dataset.box = '44,30,56,22'
    entry.style.marginLeft = '4px'
    holdInPlace(entry)
    expect(entry.dataset.leaving).toBe('')
    expect(entry.style.position).toBe('absolute')
    expect([entry.style.left, entry.style.top, entry.style.width, entry.style.height]).toEqual([
      '44px',
      '30px',
      '56px',
      '22px',
    ])
    expect(entry.style.marginLeft).toBe('0px')
  })

  it('holds each of several leaving entries where it stood before the first went', () => {
    const list = document.createElement('div')
    const entries = ['2,0,40,22', '44,0,40,22', '86,0,40,22'].map((box) => {
      const entry = document.createElement('span')
      entry.dataset.box = box
      list.append(entry)
      return entry
    })
    const [first, second, third] = entries
    if (!first || !second || !third) throw new Error('three entries')
    holdInPlace(second)
    // Out of the flow, the second lets the third move up to where it stood.
    third.dataset.box = '44,0,40,22'
    holdInPlace(third)
    expect([second.style.left, third.style.left]).toEqual(['44px', '86px'])
  })
})
