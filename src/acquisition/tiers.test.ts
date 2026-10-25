import { describe, expect, it } from 'vitest'

import type { RateTier } from '@/ipc/bindings/RateTier'

import {
  fits,
  keptChannels,
  leadingChannels,
  presetCounts,
  tierForChannels,
  tierForRate,
  tierRates,
} from './tiers'

const MHZ = 1_000_000

function tier(channels: number, leading: boolean, rates: number[], span = 16): RateTier {
  const samplerates = rates.map((mhz) => mhz * MHZ)
  return {
    channels,
    leading,
    span: leading ? channels : span,
    maxSamplerate: samplerates.at(-1) ?? 0,
    samplerates,
  }
}

/** An SLogic16U3 over USB 3.0: the highest enabled channel picks the tier. */
const LEADING = [
  tier(4, true, [5, 200, 800]),
  tier(8, true, [5, 200, 400]),
  tier(16, true, [5, 200]),
]
/** An ATK-Logic in Stream mode: the number of enabled channels does. */
const ANY = [tier(3, false, [1, 50, 100]), tier(6, false, [1, 50]), tier(16, false, [1, 20])]

describe('rate tiers', () => {
  it('fits the first channels or any of them', () => {
    expect(fits(LEADING[0]!, [0, 3])).toBe(true)
    expect(fits(LEADING[0]!, [15])).toBe(false)
    expect(fits(ANY[0]!, [7, 9, 15])).toBe(true)
    expect(fits(ANY[0]!, [0, 1, 2, 3])).toBe(false)
    // A DSLogic's 3-channel mode takes any 3 of D0–D7.
    expect(fits(tier(3, false, [1000], 8), [0, 4, 7])).toBe(true)
    expect(fits(tier(3, false, [1000], 8), [0, 9])).toBe(false)
  })

  it('finds the fastest tier that takes the channels', () => {
    expect(tierForChannels(LEADING, [0, 5])).toBe(1)
    // A lone D15 still needs all 16.
    expect(tierForChannels(LEADING, [15])).toBe(2)
    expect(tierForChannels(ANY, [10, 11, 12, 13])).toBe(1)
    expect(tierForChannels(ANY, leadingChannels(17))).toBe(-1)
  })

  it('finds the tier with the most channels that reaches a rate', () => {
    expect(tierForRate(LEADING, 200 * MHZ)).toBe(2)
    expect(tierForRate(LEADING, 400 * MHZ)).toBe(1)
    expect(tierForRate(LEADING, 800 * MHZ)).toBe(0)
    expect(tierForRate(LEADING, 1000 * MHZ)).toBe(-1)
  })

  it('keeps the enabled channels a tier allows, lowest first', () => {
    expect(keptChannels(LEADING[0]!, [1, 5, 9])).toEqual([1])
    expect(keptChannels(ANY[0]!, [2, 4, 6, 8])).toEqual([2, 4, 6])
    expect(keptChannels(tier(3, false, [1000], 8), [2, 9, 12])).toEqual([2])
    // None allowed: the tier's own first channels.
    expect(keptChannels(LEADING[0]!, [8, 9])).toEqual([0, 1, 2, 3])
    expect(keptChannels(ANY[0]!, [])).toEqual([0, 1, 2])
  })

  it('merges the rates of every tier', () => {
    expect(tierRates(LEADING)).toEqual([5, 200, 400, 800].map((mhz) => mhz * MHZ))
    expect(tierRates([])).toEqual([])
  })

  it('offers channel counts up to all of them', () => {
    expect(presetCounts(16)).toEqual([4, 8, 16])
    expect(presetCounts(32)).toEqual([4, 8, 16, 32])
    expect(presetCounts(3)).toEqual([3])
    expect(presetCounts(0)).toEqual([])
  })
})
