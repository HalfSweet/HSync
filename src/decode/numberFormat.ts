/**
 * Numbers in the annotations of DSView's decoders: a decoder gives a value in
 * hex digits and texts where `{$}` stands for it; the user picks how values show, as DSView's
 * protocol dock does (`annotationrestable.cpp`).
 */

export type NumberFormat = 'hex' | 'dec' | 'oct' | 'bin' | 'ascii'

export const NUMBER_FORMATS: readonly NumberFormat[] = ['hex', 'dec', 'oct', 'bin', 'ascii']

/** The key HSync stores a decoder entry's format under; DSView ignores it. */
export const FORMAT_KEY = 'number format'

function bits(hex: string): string {
  return [...hex].map((digit) => parseInt(digit, 16).toString(2).padStart(4, '0')).join('')
}

/** One run of hex digits in `format`, as DSView converts it. */
function convert(hex: string, format: NumberFormat): string {
  switch (format) {
    case 'hex':
      return hex
    case 'bin':
      return bits(hex)
    case 'oct': {
      // Groups of three bits from the right; leading zeros stay, as in DSView.
      const binary = bits(hex)
      const padded = binary.padStart(Math.ceil(binary.length / 3) * 3, '0')
      let out = ''
      for (let at = 0; at < padded.length; at += 3) out += parseInt(padded.slice(at, at + 3), 2)
      return out
    }
    case 'dec':
      return hex.length <= 16 ? BigInt(`0x${hex}`).toString() : hex
    case 'ascii': {
      if (hex.length >= 27) return hex
      if (hex.length === 2) {
        const code = parseInt(hex, 16)
        if (code >= 33 && code <= 126) return String.fromCharCode(code)
      }
      return `[${hex}]`
    }
  }
}

/** A value in `format`; runs of hex digits apart from other characters convert one by one. */
export function formatValue(value: string, format: NumberFormat): string {
  return value.replace(/[0-9a-fA-F]+/g, (run) => convert(run, format))
}

/** An annotation's texts as shown: `{$}` replaced by the value, or the value alone. */
export function shownTexts(
  texts: readonly string[],
  value: string | null,
  format: NumberFormat,
): string[] {
  if (value === null) return [...texts]
  const shown = formatValue(value, format)
  return texts.length > 0 ? texts.map((text) => text.replaceAll('{$}', shown)) : [shown]
}

/** A decoder entry's number format; hex unless it says otherwise. */
export function entryFormat(entry: unknown): NumberFormat {
  const stored = (entry as Record<string, unknown> | undefined)?.[FORMAT_KEY]
  return NUMBER_FORMATS.find((format) => format === stored) ?? 'hex'
}
