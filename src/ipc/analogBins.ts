/**
 * Decoding of the binary analog viewport format (`hsync_session::encode_analog_bins`): the
 * lowest and highest volts of each column of the analog channels asked for.
 */

export const ANALOG_BINS_FORMAT_VERSION = 1
const HEADER_BYTES = 32

export interface AnalogColumns {
  /** Lowest volts of all the channel's samples so far; NaN without any. */
  low: number
  /** Highest volts of all the channel's samples so far; NaN without any. */
  high: number
  /** Lowest and highest volts of each column in turn; NaN beyond the samples. */
  columns: Float32Array
}

/** Columns of several analog channels over one sample range, as {@link BinsResult}'s. */
export interface AnalogBinsResult {
  start: number
  end: number
  bins: number
  channels: Map<number, AnalogColumns>
}

export function decodeAnalogBins(buffer: ArrayBuffer): AnalogBinsResult {
  const view = new DataView(buffer)
  if (buffer.byteLength < HEADER_BYTES) throw new Error('analog bins: truncated header')
  const version = view.getUint32(0, true)
  if (version !== ANALOG_BINS_FORMAT_VERSION) {
    throw new Error(`analog bins: unsupported version ${version}`)
  }
  const count = view.getUint32(4, true)
  const bins = view.getUint32(8, true)
  const start = Number(view.getBigUint64(16, true))
  const end = Number(view.getBigUint64(24, true))
  const channels = new Map<number, AnalogColumns>()
  const size = 12 + 8 * bins
  let offset = HEADER_BYTES
  for (let i = 0; i < count; i++) {
    if (offset + size > buffer.byteLength) throw new Error('analog bins: truncated channel')
    const index = view.getUint16(offset, true)
    // Copied out: the floats need not be aligned in the buffer.
    const columns = new Float32Array(2 * bins)
    for (let at = 0; at < 2 * bins; at++) {
      columns[at] = view.getFloat32(offset + 12 + 4 * at, true)
    }
    channels.set(index, {
      low: view.getFloat32(offset + 4, true),
      high: view.getFloat32(offset + 8, true),
      columns,
    })
    offset += size
  }
  return { start, end, bins, channels }
}

/** Inverse of {@link decodeAnalogBins}; used by the browser mock and tests. */
export function encodeAnalogBins(result: AnalogBinsResult): ArrayBuffer {
  const size = 12 + 8 * result.bins
  const buffer = new ArrayBuffer(HEADER_BYTES + result.channels.size * size)
  const view = new DataView(buffer)
  view.setUint32(0, ANALOG_BINS_FORMAT_VERSION, true)
  view.setUint32(4, result.channels.size, true)
  view.setUint32(8, result.bins, true)
  view.setBigUint64(16, BigInt(result.start), true)
  view.setBigUint64(24, BigInt(result.end), true)
  let offset = HEADER_BYTES
  for (const [index, channel] of result.channels) {
    view.setUint16(offset, index, true)
    view.setFloat32(offset + 4, channel.low, true)
    view.setFloat32(offset + 8, channel.high, true)
    channel.columns.forEach((volts, at) => view.setFloat32(offset + 12 + 4 * at, volts, true))
    offset += size
  }
  return buffer
}
