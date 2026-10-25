/**
 * Decoding of the binary viewport format (`hsync_session::encode_bins`).
 */

/** Level of the first sample in the bin. */
export const CELL_FIRST_HIGH = 1 << 0
/** Level of the last sample in the bin. */
export const CELL_LAST_HIGH = 1 << 1
/** The bin contains at least one transition. */
export const CELL_TRANSITION = 1 << 2
/** The bin lies beyond the captured samples. */
export const CELL_EMPTY = 1 << 3

export const BINS_FORMAT_VERSION = 1
const HEADER_BYTES = 32

/** Columns of several channels over one sample range. */
export interface BinsResult {
  /** First sample of the range. */
  start: number
  /** Sample after the last one. */
  end: number
  /** Columns per channel; column `i` covers `start + ⌊i·n/bins⌋ .. start + ⌊(i+1)·n/bins⌋`. */
  bins: number
  /** Cells by channel index. */
  channels: Map<number, Uint8Array>
}

/** First sample of column `column`, of logic or analog columns alike. */
export function binStart(
  result: Pick<BinsResult, 'start' | 'end' | 'bins'>,
  column: number,
): number {
  return result.start + Math.floor(((result.end - result.start) * column) / result.bins)
}

export function decodeBins(buffer: ArrayBuffer): BinsResult {
  const view = new DataView(buffer)
  if (buffer.byteLength < HEADER_BYTES) throw new Error('bins: truncated header')
  const version = view.getUint32(0, true)
  if (version !== BINS_FORMAT_VERSION) throw new Error(`bins: unsupported version ${version}`)
  const count = view.getUint32(4, true)
  const bins = view.getUint32(8, true)
  const start = Number(view.getBigUint64(16, true))
  const end = Number(view.getBigUint64(24, true))
  const channels = new Map<number, Uint8Array>()
  let offset = HEADER_BYTES
  for (let i = 0; i < count; i++) {
    if (offset + 4 + bins > buffer.byteLength) throw new Error('bins: truncated channel')
    const index = view.getUint16(offset, true)
    channels.set(index, new Uint8Array(buffer, offset + 4, bins))
    offset += 4 + bins
  }
  return { start, end, bins, channels }
}

/** Inverse of {@link decodeBins}; used by the browser mock and tests. */
export function encodeBins(result: BinsResult): ArrayBuffer {
  const buffer = new ArrayBuffer(HEADER_BYTES + result.channels.size * (4 + result.bins))
  const view = new DataView(buffer)
  view.setUint32(0, BINS_FORMAT_VERSION, true)
  view.setUint32(4, result.channels.size, true)
  view.setUint32(8, result.bins, true)
  view.setBigUint64(16, BigInt(result.start), true)
  view.setBigUint64(24, BigInt(result.end), true)
  let offset = HEADER_BYTES
  for (const [index, cells] of result.channels) {
    view.setUint16(offset, index, true)
    new Uint8Array(buffer, offset + 4, result.bins).set(cells)
    offset += 4 + result.bins
  }
  return buffer
}
