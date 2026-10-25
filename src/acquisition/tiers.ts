import type { RateTier } from '@/ipc/bindings/RateTier'

/**
 * How the acquisition menu trades channels for sample rate (DESIGN.md §4): a device reports,
 * for its current mode, tiers of "at most N channels, up to this rate", fewest channels first.
 * These helpers tell which tier a rate or a set of channels needs and what moving there keeps.
 */

/** Channels 0 to `count - 1`. */
export function leadingChannels(count: number): number[] {
  return Array.from({ length: count }, (_, index) => index)
}

/** Whether `channels` can be captured together in `tier`. */
export function fits(tier: RateTier, channels: readonly number[]): boolean {
  return channels.length <= tier.channels && channels.every((channel) => channel < tier.span)
}

/** Index of the tier with the fewest channels that `channels` fit, or -1. */
export function tierForChannels(tiers: readonly RateTier[], channels: readonly number[]): number {
  return tiers.findIndex((tier) => fits(tier, channels))
}

/** Index of the tier with the most channels that offers `rate`, or -1. */
export function tierForRate(tiers: readonly RateTier[], rate: number): number {
  return tiers.findLastIndex((tier) => tier.samplerates.includes(rate))
}

/**
 * The channels to enable when moving to `tier` for its rates: the enabled ones it allows,
 * lowest first, or its first channels when it allows none of them.
 */
export function keptChannels(tier: RateTier, enabled: readonly number[]): number[] {
  const kept = enabled.filter((channel) => channel < tier.span).slice(0, tier.channels)
  return kept.length > 0 ? kept : leadingChannels(tier.channels)
}

/** Every sample rate the tiers offer, ascending. */
export function tierRates(tiers: readonly RateTier[]): number[] {
  return [...new Set(tiers.flatMap((tier) => tier.samplerates))].sort((a, b) => a - b)
}

/**
 * Channel counts offered as shortcuts by a device whose channels do not bound its rate:
 * 4, 8, 16 … below its channel count, then all of them.
 */
export function presetCounts(channelCount: number): number[] {
  const counts: number[] = []
  for (let count = 4; count < channelCount; count *= 2) counts.push(count)
  if (channelCount > 0) counts.push(channelCount)
  return counts
}
