import { describe, expect, it } from 'vitest'

import { summarizeTriggers } from './triggerSummary'

const armed = (count: number) =>
  Array.from({ length: count }, (_, index) => ({ index, condition: 'rise' }))

describe('summarizeTriggers', () => {
  it('lists up to three channels', () => {
    expect(summarizeTriggers(armed(0))).toEqual({ shown: [], more: 0 })
    expect(summarizeTriggers(armed(3)).shown).toHaveLength(3)
    expect(summarizeTriggers(armed(3)).more).toBe(0)
  })

  it('lists two and counts all beyond three', () => {
    const summary = summarizeTriggers(armed(5))
    expect(summary.shown.map((channel) => channel.index)).toEqual([0, 1])
    expect(summary.more).toBe(5)
  })
})
