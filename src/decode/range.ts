/**
 * HSync's decode range in a DSView decoder entry: the first and last sample decoded, as DSView's
 * start and end cursors. The backend decodes those samples only; DSView ignores the key.
 */
export const RANGE_KEY = 'decode range'

/** The stored range as `[first, last]`, or `null` for the whole capture. */
export function entryRange(entry: unknown): [number, number] | null {
  const stored = (entry as Record<string, unknown> | null | undefined)?.[RANGE_KEY]
  if (!Array.isArray(stored) || stored.length !== 2) return null
  const [first, last] = stored as unknown[]
  if (typeof first !== 'number' || typeof last !== 'number') return null
  return [Math.min(first, last), Math.max(first, last)]
}

/** `entry` decoding `range` only, or all of the capture without one. */
export function withRange(
  entry: Record<string, unknown>,
  range: readonly [number, number] | null,
): Record<string, unknown> {
  const { [RANGE_KEY]: _, ...rest } = entry
  if (!range) return rest
  const [first, last] = range
  return {
    ...rest,
    [RANGE_KEY]: [Math.round(Math.min(first, last)), Math.round(Math.max(first, last))],
  }
}
