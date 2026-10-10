/** Text of decoded data, shared by the decode lanes and the data stream (DESIGN.md §6.3, §9). */

/** Upper-case hex with leading zeros. */
export function hex(value: number, digits = 2): string {
  return value.toString(16).toUpperCase().padStart(digits, '0')
}

/** Bytes as spaced hex, e.g. `32 BB`. */
export function hexBytes(values: readonly number[]): string {
  return values.map((value) => hex(value)).join(' ')
}

/** A piece of listed text: plain characters or an escape such as `\r`. */
export interface TextPiece {
  text: string
  escape: boolean
}

const ESCAPES: Record<string, string> = { '\r': '\\r', '\n': '\\n', '\t': '\\t' }

/** Splits text into plain runs and escapes; other control characters become `\xNN`. */
export function escapePieces(text: string): TextPiece[] {
  const pieces: TextPiece[] = []
  let plain = ''
  for (const ch of text) {
    const code = ch.codePointAt(0) ?? 0
    const escape = ESCAPES[ch] ?? (code < 0x20 || code === 0x7f ? `\\x${hex(code)}` : null)
    if (escape === null) {
      plain += ch
      continue
    }
    if (plain) pieces.push({ text: plain, escape: false })
    plain = ''
    pieces.push({ text: escape, escape: true })
  }
  if (plain) pieces.push({ text: plain, escape: false })
  return pieces
}
