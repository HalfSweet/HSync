/**
 * Multi-stage and serial triggers as the trigger panel edits them (DESIGN.md §2.2), and the
 * requests they turn into. The fields and their defaults follow DSView's trigger dock.
 */
import type { AdvancedTrigger } from '@/ipc/bindings/AdvancedTrigger'
import type { TriggerCondition } from '@/ipc/bindings/TriggerCondition'
import type { TriggerStageSpec } from '@/ipc/bindings/TriggerStageSpec'

/** Conditions by channel; channels left out take no part. */
export type Pattern = Partial<Record<number, TriggerCondition>>

export interface StageDraft {
  pattern0: Pattern
  pattern1: Pattern
  /** Both patterns must hold, rather than either. */
  both: boolean
  invert0: boolean
  invert1: boolean
  count: number
  contiguous: boolean
}

export interface SerialDraft {
  start: Pattern
  stop: Pattern
  clock: Pattern
  dataChannel: number
  bits: number
  /** The data bits, bit 15 first: `0`, `1` or `X` for any. */
  value: string
}

export const MAX_STAGES = 16
export const MAX_SERIAL_BITS = 16
const VALUE_LENGTH = 16

/** A stage as DSView's dock starts it: patterns that match anything, combined with "and". */
export function newStage(): StageDraft {
  return {
    pattern0: {},
    pattern1: {},
    both: true,
    invert0: false,
    invert1: false,
    count: 1,
    contiguous: false,
  }
}

export function newSerial(): SerialDraft {
  return {
    start: {},
    stop: {},
    clock: {},
    dataChannel: 0,
    bits: 8,
    value: 'X'.repeat(VALUE_LENGTH),
  }
}

/** The conditions a pattern sets on the device's channels, ascending. */
export function patternList(pattern: Pattern, channels: number): [number, TriggerCondition][] {
  return Object.entries(pattern)
    .map(([index, condition]) => [Number(index), condition ?? 'none'] as [number, TriggerCondition])
    .filter(([index, condition]) => condition !== 'none' && index < channels)
    .sort((a, b) => a[0] - b[0])
}

/** Whether a pattern sets any condition. */
export function patternUsed(pattern: Pattern, channels: number): boolean {
  return patternList(pattern, channels).length > 0
}

export function stageSpec(stage: StageDraft, channels: number): TriggerStageSpec {
  return {
    pattern0: patternList(stage.pattern0, channels),
    pattern1: patternList(stage.pattern1, channels),
    both: stage.both,
    invert0: stage.invert0,
    invert1: stage.invert1,
    count: Math.max(1, Math.round(stage.count)),
    contiguous: stage.contiguous,
  }
}

/** A data value as typed: 16 positions of `0`, `1` and `X`; spaces are ignored and missing high
 * positions match anything. `null` when something else was typed. */
export function normalizeValue(text: string): string | null {
  const bits = text.replace(/\s+/g, '').toUpperCase()
  if (bits.length > VALUE_LENGTH || /[^01X]/.test(bits)) return null
  return bits.padStart(VALUE_LENGTH, 'X')
}

/** The value and the mask of the bits that count. */
export function valueBits(value: string): { value: number; mask: number } {
  let bits = 0
  let mask = 0
  for (const [position, char] of [...value].entries()) {
    const bit = VALUE_LENGTH - 1 - position
    if (char === 'X') continue
    mask |= 1 << bit
    if (char === '1') bits |= 1 << bit
  }
  return { value: bits, mask }
}

/** The low `bits` bits in hex when each of them counts; `null` otherwise. */
export function valueHex(value: string, bits: number): string | null {
  const { value: number, mask } = valueBits(value)
  const used = bits >= VALUE_LENGTH ? 0xffff : (1 << bits) - 1
  if ((mask & used) !== used) return null
  return (number & used)
    .toString(16)
    .toUpperCase()
    .padStart(Math.ceil(bits / 4), '0')
}

/** The positions for a value typed in hex: its low `bits` bits, the others any. `null` for
 * anything but 1–4 hex digits. */
export function valueFromHex(hex: string, bits: number): string | null {
  if (!/^[0-9a-f]{1,4}$/i.test(hex.trim())) return null
  const width = Math.min(Math.max(1, bits), VALUE_LENGTH)
  const binary = (parseInt(hex.trim(), 16) & ((1 << width) - 1)).toString(2).padStart(width, '0')
  return binary.padStart(VALUE_LENGTH, 'X')
}

/** The cycle of conditions in a pattern cell: the trigger menu's order. */
const ORDER: TriggerCondition[] = ['none', 'rise', 'fall', 'both', 'high', 'low']

export function nextCondition(condition: TriggerCondition, backwards = false): TriggerCondition {
  const at = ORDER.indexOf(condition)
  return ORDER[(at + (backwards ? ORDER.length - 1 : 1)) % ORDER.length] ?? 'none'
}

/** DSView's letters for conditions: `X 0 1 R F C`. */
const LETTERS: Record<string, TriggerCondition> = {
  x: 'none',
  '0': 'low',
  '1': 'high',
  r: 'rise',
  f: 'fall',
  c: 'both',
}

export function conditionForKey(key: string): TriggerCondition | null {
  return LETTERS[key.toLowerCase()] ?? null
}

export type TriggerMode = 'simple' | 'stages' | 'serial'

/** What a capture sends besides the simple trigger. */
export function advancedTrigger(
  mode: TriggerMode,
  stages: readonly StageDraft[],
  stageCount: number,
  serial: SerialDraft,
  channels: number,
): AdvancedTrigger | null {
  if (mode === 'stages') {
    const count = Math.min(Math.max(1, stageCount), MAX_STAGES)
    return {
      mode: 'stages',
      stages: stages.slice(0, count).map((stage) => stageSpec(stage, channels)),
    }
  }
  if (mode === 'serial') {
    const { value, mask } = valueBits(normalizeValue(serial.value) ?? newSerial().value)
    return {
      mode: 'serial',
      start: patternList(serial.start, channels),
      stop: patternList(serial.stop, channels),
      clock: patternList(serial.clock, channels),
      dataChannel: Math.min(serial.dataChannel, channels - 1),
      bits: Math.min(Math.max(1, serial.bits), MAX_SERIAL_BITS),
      value,
      mask,
    }
  }
  return null
}
