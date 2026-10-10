/**
 * Level patterns in the search box (DESIGN.md §9.8), DSView's search dock as text: `D3=R D5=1`
 * finds where channel 3 rises while channel 5 is high.
 */
import type { PatternCondition } from '@/ipc/bindings/PatternCondition'
import type { PatternLevel } from '@/ipc/bindings/PatternLevel'

/** DSView's codes: low, high, rising, falling, either edge (`C`), and don't care (`X`). */
const LEVELS: Partial<Record<string, PatternLevel | null>> = {
  '0': 'low',
  '1': 'high',
  r: 'rising',
  f: 'falling',
  c: 'edge',
  x: null,
}

const TOKEN = /^([^=\s]+)=([01rfcx])$/i

/** A channel the pattern may name: by number (`D3`, `3`) or by any of its names. */
export interface PatternChannel {
  index: number
  names: string[]
}

export type ParsedPattern =
  { kind: 'conditions'; conditions: PatternCondition[] } | { kind: 'unknownChannel'; name: string }

function tokens(query: string): string[] {
  const trimmed = query.trim()
  return trimmed ? trimmed.split(/\s+/) : []
}

/** Whether the query is a level pattern rather than something to find in decoded data. */
export function isPattern(query: string): boolean {
  const all = tokens(query)
  return all.length > 0 && all.every((token) => TOKEN.test(token))
}

/** The channel a pattern names, or `null` when the capture has none by that name. */
export function resolveChannel(name: string, channels: readonly PatternChannel[]): number | null {
  const numbered = /^d?(\d+)$/i.exec(name)
  if (numbered) {
    const index = Number(numbered[1])
    return channels.some((channel) => channel.index === index) ? index : null
  }
  const wanted = name.toLowerCase()
  return (
    channels.find((channel) => channel.names.some((one) => one.toLowerCase() === wanted))?.index ??
    null
  )
}

/**
 * Reads a level pattern; `null` when the query is not one. A channel named twice keeps its last
 * condition; `X` conditions are dropped.
 */
export function parsePattern(
  query: string,
  channels: readonly PatternChannel[],
): ParsedPattern | null {
  if (!isPattern(query)) return null
  const conditions: PatternCondition[] = []
  for (const token of tokens(query)) {
    const [, name = '', code = ''] = TOKEN.exec(token) ?? []
    const channel = resolveChannel(name, channels)
    if (channel === null) return { kind: 'unknownChannel', name }
    const level = LEVELS[code.toLowerCase()]
    const others = conditions.filter((condition) => condition.channel !== channel)
    conditions.splice(0, conditions.length, ...others)
    if (level) conditions.push({ channel, level })
  }
  return { kind: 'conditions', conditions }
}
