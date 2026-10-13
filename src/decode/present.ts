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
const COUNTERS = new Set(['tx', 'index', 'op', 'seq', 'frame', 'packet', 'pkt', 'xact'])

/**
 * Fields that measure or count — ticks, bits, lengths, offsets, currents — and so read in
 * decimal whatever the number format, which is for what the bus carried (values, bytes,
 * addresses, identifiers).
 */
const QUANTITIES = new Set([
  ...COUNTERS,
  'offset',
  'length',
  'dlc',
  'bits',
  'ticks',
  'count',
  'slot',
  'slots',
  'leds',
  'position',
  'objects',
  'resolution',
  'revision',
  'milliamps',
  'millivolts',
  'milliwatts',
  'maximum',
  'operating',
  'sincePrevious',
  'channel',
  'endpoint',
  'clause',
])

/** Whether a field is a quantity rather than a word the bus carried. */
export function isQuantity(name: string): boolean {
  return QUANTITIES.has(name) || /(Ticks|Bits|Count|Number|Size|Millivolts|Milliamps)$/.test(name)
}

/**
 * The fields a block shows beside the kind, at most `limit` of them: the numbers and byte
 * arrays that are not counters (the two lines of an SPI word, the one value of a UART
 * frame), else the first counter, else the first string; nothing for an event of booleans
 * alone.
 */
export function primaryFields(payload: Record<string, unknown>, limit = 2): [string, unknown][] {
  const fields = payloadFields(payload)
  const numeric = fields.filter(([, value]) => typeof value === 'number' || isNumbers(value))
  const main = numeric.filter(([name]) => !COUNTERS.has(name)).slice(0, limit)
  if (main.length > 0) return main
  const fallback = numeric[0] ?? fields.find(([, value]) => typeof value === 'string')
  return fallback ? [fallback] : []
}

/** The first of [`primaryFields`], or nothing. */
export function primaryField(payload: Record<string, unknown>): [string, unknown] | null {
  return primaryFields(payload, 1)[0] ?? null
}

/** A field's value as shown. */
export function valueText(
  field: string,
  value: unknown,
  format: NumberFormat,
  wording: Wording,
): string {
  if (value === null || value === undefined) return wording.none
  if (typeof value === 'number') return formatNumber(value, isQuantity(field) ? 'dec' : format)
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
