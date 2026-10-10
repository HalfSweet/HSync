import { afterEach, describe, expect, it } from 'vitest'

import { motionTiming, parseDuration } from './timing'

describe('motion timing', () => {
  afterEach(() => document.documentElement.removeAttribute('style'))

  it('reads durations as written or as the build shortens them', () => {
    expect(parseDuration('200ms')).toBe(200)
    expect(parseDuration(' .26s')).toBe(260)
    expect(parseDuration('0s')).toBe(0)
    expect(parseDuration('')).toBe(0)
  })

  it('reads a token pair from the root', () => {
    document.documentElement.style.setProperty('--motion-enter', '200ms')
    document.documentElement.style.setProperty('--ease-enter', 'cubic-bezier(0.16, 1, 0.3, 1)')
    expect(motionTiming('enter')).toEqual({
      duration: 200,
      easing: 'cubic-bezier(0.16, 1, 0.3, 1)',
    })
  })
})
