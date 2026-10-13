/** How the numbers of decoded events show: the user picks a base per decoder. */

export type NumberFormat = 'hex' | 'dec' | 'oct' | 'bin' | 'ascii'

export const NUMBER_FORMATS: readonly NumberFormat[] = ['hex', 'dec', 'oct', 'bin', 'ascii']

/** Digits of `value` in `base`, padded to whole bytes' worth: 2 hex digits, 8 bits. */
function padded(value: number, base: 2 | 16): string {
  const digits = value.toString(base).toUpperCase()
  const perByte = base === 16 ? 2 : 8
  return digits.padStart(Math.ceil(digits.length / perByte) * perByte, '0')
}

/** A number in `format`; integers only convert, anything else shows as it is. */
export function formatNumber(value: number, format: NumberFormat): string {
  if (!Number.isInteger(value)) return String(value)
  const sign = value < 0 ? '-' : ''
  const magnitude = Math.abs(value)
  switch (format) {
    case 'hex':
      return sign + padded(magnitude, 16)
    case 'bin':
      return sign + padded(magnitude, 2)
    case 'oct':
      return sign + magnitude.toString(8)
    case 'dec':
      return String(value)
    case 'ascii':
      if (value >= 0x21 && value <= 0x7e) return String.fromCharCode(value)
      return `[${padded(magnitude, 16)}]`
  }
}

/** The number format a decoder entry asks for; hex unless it says otherwise. */
export function entryFormat(entry: { show?: { numbers?: unknown } } | undefined): NumberFormat {
  const stored = entry?.show?.numbers
  return NUMBER_FORMATS.find((format) => format === stored) ?? 'hex'
}
