/**
 * How decoded events show, whatever the protocol (DESIGN.md §6.3, §9): an event is its kind
 * and its fields as the contract names them; a number shows in the decoder's number format, a
 * byte array as its bytes, an enumerated value by its name.
 */
import { formatNumber, type NumberFormat } from './numberFormat'

/** Words the presentation needs translated. */
export interface Wording {
  /** The name of an enumerated value `value` of `field`, or the value itself. */
  enumValue: (field: string, value: string) => string
  yes: string
  no: string
  /** For a field that holds nothing (`null`). */
  none: string
}

export interface FieldText {
  name: string
  text: string
}

/** The fields of a payload other than its kind, in the contract's order. */
export function payloadFields(payload: Record<string, unknown>): [string, unknown][] {
  return Object.entries(payload).filter(([name]) => name !== 'kind')
}

const isNumbers = (value: unknown): value is number[] =>
  Array.isArray(value) && value.every((item) => typeof item === 'number')

/** Fields that count or number things rather than carry what the bus said. */
const COUNTERS = new Set(['tx', 'index', 'op', 'seq', 'frame', 'packet', 'pkt'])

/**
 * The field a block shows beside the kind: the first number or byte array that is not a
 * counter, else any number or byte array, else the first string; nothing for an event of
 * booleans alone.
 */
export function primaryField(payload: Record<string, unknown>): [string, unknown] | null {
  const fields = payloadFields(payload)
  const numeric = fields.filter(([, value]) => typeof value === 'number' || isNumbers(value))
  return (
    numeric.find(([name]) => !COUNTERS.has(name)) ??
    numeric[0] ??
    fields.find(([, value]) => typeof value === 'string') ??
    null
  )
}

/** A field's value as shown. */
export function valueText(
  field: string,
  value: unknown,
  format: NumberFormat,
  wording: Wording,
): string {
  if (value === null || value === undefined) return wording.none
  if (typeof value === 'number') return formatNumber(value, format)
  if (typeof value === 'boolean') return value ? wording.yes : wording.no
  if (typeof value === 'string') return wording.enumValue(field, value)
  if (isNumbers(value)) return value.map((item) => formatNumber(item, format)).join(' ')
  if (Array.isArray(value))
    return value.map((item) => valueText(field, item, format, wording)).join(' ')
  return JSON.stringify(value)
}

/** Every field of a payload as shown, in the contract's order. */
export function fieldTexts(
  payload: Record<string, unknown>,
  format: NumberFormat,
  wording: Wording,
): FieldText[] {
  return payloadFields(payload).map(([name, value]) => ({
    name,
    text: valueText(name, value, format, wording),
  }))
}
