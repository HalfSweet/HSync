// SPDX-License-Identifier: Apache-2.0

/**
 * The UART decoder: the characters on one line of an asynchronous serial link, each timed from
 * its start edge.
 *
 * Implemented from the asynchronous start-stop character format of TIA/EIA-232-F and the
 * receivers of 16550-class UARTs: the line idles at mark; a character is a start bit at space,
 * 5 to 9 data bits, an optional parity bit and 1, 1½ or 2 stop bits at mark; a line held at
 * space through a whole character is a break.
 *
 * The rules:
 *
 * - mark, the idle level, is high, or low on an inverted line; space is the other level. A mark
 *   bit is a 1, a space bit a 0;
 * - a character begins at an edge from mark to space; an edge back to mark begins nothing;
 * - the start bit is read at its sample point: if the line is back at mark there, the edge was a
 *   glitch and the search for a start edge goes on from that point;
 * - every later bit is read at its sample point and nowhere else: a character is timed from its
 *   start edge, and edges inside it are just its bits;
 * - bit `n` of a character, the start bit being bit 0, is read at
 *   `start + ⌊rate × (100 n + p) / (100 × baud)⌋`, where `rate` is the source's samples per
 *   second and `p` the sample point in percent of a bit; the half stop bit of 1½ stop bits is
 *   read at `p` percent of its half bit. Each position is computed from the start edge and
 *   rounded down, so rounding never accumulates;
 * - the data bits make the value least or most significant bit first, as configured; the parity
 *   bit is checked against them; every stop bit must read mark, or the character has a framing
 *   error;
 * - a character whose every bit reads space, its parity and stop bits included, is a break: the
 *   decoder waits for the line to return to mark and reports how long it was held at space;
 * - after a character the search for the next start edge begins at its last stop bit's sample
 *   point, so a sender somewhat faster than the configured rate is followed;
 * - a character spans from its start edge to the computed end of its stop bits and is settled
 *   at its last stop bit's sample point; a break spans from its start edge to the first tick
 *   back at mark, where it is settled;
 * - the first sample of the stream and the first after a gap are never an edge: a line found at
 *   space there is a character under way, which is skipped;
 * - a character or break cut off by a gap or by the end of the samples is not reported: the
 *   stream has no frame to abort;
 * - progress is passed on, but never past the start edge of a character or break still being
 *   read;
 * - with fewer than two ticks per bit, bits cannot be told apart, so the input requires the
 *   source's `rateHz` to be at least twice the baud rate. Should a decoder start on a slower
 *   source all the same, it reports no characters and only passes gaps, progress and the end on.
 *
 * Limits: one line per decoder (a link with RX and TX takes two); 5 to 9 data bits; the sample
 * point is a single sample per bit, without majority voting; nothing is reported about the idle
 * line itself.
 */

import type { Channel, Levels, Provenance, Tick, bool, u16, u32, u64, u8 } from 'uiharu'
import {
  LogicDecoder,
  MAX_CHANNELS,
  atLeast,
  countOnes,
  defineLogicDecoder,
  eventSchema,
  fromBool,
  mask,
  mulDiv,
  over,
} from 'uiharu'

/** What the parity bit of a character says. */
export type ParityCheck = 'ok' | 'error'

/** What travels on a UART event stream. The stream is unframed: every character is one event,
 * decided once its last stop bit has been sampled. */
export type UartEvent =
  /** A character. `value` is the data bits as a number: least significant bit first, the first
   * data bit is bit 0; most significant bit first, it is the highest of the configured data
   * bits. `parity` is `undefined` on a line without a parity bit; `framingOk` says whether
   * every stop bit read mark. */
  | { kind: 'frame'; value: u16; parity: ParityCheck | undefined; framingOk: bool }
  /** The line was held at space through a whole character and then returned to mark;
   * `ticks` is how long, from the edge that looked like a start bit to the first tick back at
   * mark. */
  | { kind: 'break'; ticks: u64 }

/** The schema of UART event streams. */
export const uartEvents = eventSchema<UartEvent>('uart.events', 1)

/** The parity bit a character carries. */
export type Parity = 'none' | 'even' | 'odd' | 'mark' | 'space'

/** How long the stop period of a character is. */
export type StopBits = 'one' | 'one-and-a-half' | 'two'

/** Which data bit is sent first. */
export type BitOrder = 'lsb-first' | 'msb-first'

/** How the line is wired and what its characters look like. */
export interface UartConfig {
  /** The channel the line is on. */
  line: Channel
  /** Bits per second, above zero. */
  baud: u32
  /** Data bits per character, 5 to 9. */
  dataBits: u8
  /** The parity bit. */
  parity: Parity
  /** The stop period. */
  stopBits: StopBits
  /** Which data bit comes first. */
  bitOrder: BitOrder
  /** Whether the line idles low: mark is low and space high. */
  inverted: bool
  /** Where in each bit the line is read, in percent of the bit time, 1 to 99. */
  samplePoint: u8
}

/** The parity bit the data bits `value` call for; `undefined` without a parity bit. */
export function parityBit(parity: Parity, value: u16): bool | undefined {
  const oddOnes = countOnes(value) % 2 === 1
  switch (parity) {
    case 'none':
      return undefined
    case 'even':
      return oddOnes
    case 'odd':
      return !oddOnes
    case 'mark':
      return true
    case 'space':
      return false
  }
}

/** The stop period in half bits. */
function halfBits(stopBits: StopBits): u8 {
  switch (stopBits) {
    case 'one':
      return 2
    case 'one-and-a-half':
      return 3
    case 'two':
      return 4
  }
}

/** A character's length, start, data, parity and stop bits, in two-hundredths of a bit. */
function length(config: UartConfig): u64 {
  const whole: u64 = 1 + config.dataBits + fromBool(config.parity !== 'none')
  return 200 * whole + 100 * halfBits(config.stopBits)
}

function validate(config: UartConfig): string | undefined {
  if (config.baud === 0) return 'the baud rate must be above zero'
  if (config.dataBits < 5 || config.dataBits > 9) {
    return `${config.dataBits} data bits; 5 to 9 are supported`
  }
  if (config.samplePoint < 1 || config.samplePoint > 99) {
    return `a sample point at ${config.samplePoint}% of the bit; 1% to 99% are supported`
  }
  if (config.line >= MAX_CHANNELS) {
    return `channel ${config.line} is out of range; channels are 0 to ${MAX_CHANNELS - 1}`
  }
  return undefined
}

/** Where the bits of a character fall, in ticks from its start edge. */
interface Timing {
  rate: u64
  baud: u64
  samplePoint: u64
}

/** The timing at `rate` samples per second; `undefined` when a bit lasts less than two ticks. */
function timingOf(config: UartConfig, rate: u64): Timing | undefined {
  const baud: u64 = config.baud
  if (baud > 0 && rate / baud >= 2) return { rate, baud, samplePoint: config.samplePoint }
  return undefined
}

/** Ticks from the start edge to `position`, given in two-hundredths of a bit, rounded down. */
function ticks(timing: Timing, position: u64): u64 {
  return mulDiv(timing.rate, position, 200 * timing.baud)
}

/** The sample point of bit `bit`, the start bit being bit 0. */
function bitPoint(timing: Timing, bit: u64): u64 {
  return ticks(timing, 200 * bit + 2 * timing.samplePoint)
}

/** The sample point of a half bit that begins where bit `bit` would. */
function halfBitPoint(timing: Timing, bit: u64): u64 {
  return ticks(timing, 200 * bit + timing.samplePoint)
}

/** A character or break, read to its end. */
interface Found {
  at: Provenance
  event: UartEvent
}

class Uart extends LogicDecoder<UartConfig, UartEvent> {
  readonly timing = timingOf(this.config, this.rate)
  readonly watch = mask(this.config.line)
  /** The start edge of the character or break being read: progress is held back to it. */
  start: Tick | undefined = undefined

  override hold(): Tick | undefined {
    return this.start
  }

  override onGap(): void {
    this.start = undefined
  }

  /** Whether `levels` show the line at mark. */
  mark(levels: Levels): bool {
    return levels.high(this.config.line) !== this.config.inverted
  }

  /** Reads the line `offset` ticks after the start edge at `start`: whether it is at mark. */
  async read(start: Tick, offset: u64): Promise<bool> {
    this.signal.seek(start + offset)
    return this.mark(await this.signal.sample())
  }

  /** Reads the character whose start edge is at `start`; `undefined` when the start bit was a
   * glitch. */
  async character(timing: Timing, start: Tick): Promise<Found | undefined> {
    const config = this.config
    // With few ticks per bit the start bit's sample point can be the start edge itself, which
    // reads space.
    const startBit = bitPoint(timing, 0)
    if (startBit > 0 && (await this.read(start, startBit))) return undefined
    const dataBits: u64 = config.dataBits
    let value: u16 = 0
    for (let i: u64 = 0; i < dataBits; i++) {
      const one: u16 = fromBool(await this.read(start, bitPoint(timing, 1 + i)))
      value = config.bitOrder === 'lsb-first' ? value | (one << i) : (value << 1) | one
    }
    let next: u64 = 1 + dataBits
    let allSpace = value === 0
    let parity: ParityCheck | undefined = undefined
    const wanted = parityBit(config.parity, value)
    if (wanted !== undefined) {
      const bit = await this.read(start, bitPoint(timing, next))
      next += 1
      allSpace = allSpace && !bit
      parity = bit === wanted ? 'ok' : 'error'
    }
    const second =
      config.stopBits === 'one'
        ? undefined
        : config.stopBits === 'two'
          ? bitPoint(timing, next + 1)
          : halfBitPoint(timing, next + 1)
    let framingOk = true
    let settled = start
    for (const point of [bitPoint(timing, next), second]) {
      if (point === undefined) continue
      const bit = await this.read(start, point)
      framingOk = framingOk && bit
      allSpace = allSpace && !bit
      settled = start + point
    }
    if (allSpace) return await this.lineBreak(start)
    const end = start + ticks(timing, length(config))
    return { at: over(start, end, settled), event: { kind: 'frame', value, parity, framingOk } }
  }

  /** Waits for the line to return to mark after the break that began at `start`. */
  async lineBreak(start: Tick): Promise<Found> {
    for (;;) {
      // One line only changes from space to mark.
      const edge = await this.signal.nextChange(this.watch)
      if (this.mark(edge.levels)) {
        return {
          at: over(start, edge.tick, edge.tick),
          event: { kind: 'break', ticks: edge.tick - start },
        }
      }
    }
  }

  override async run(): Promise<void> {
    const timing = this.timing
    // Fewer than two ticks per bit: nothing is decoded, the samples pass.
    if (timing === undefined) return
    for (;;) {
      const edge = await this.signal.nextChange(this.watch)
      // Only an edge to space begins a character.
      if (this.mark(edge.levels)) continue
      this.start = edge.tick
      const found = await this.character(timing, edge.tick)
      this.start = undefined
      if (found !== undefined) await this.out.emit(found.event, found.at)
    }
  }
}

/** The UART decoder node: logic samples in, `uart.events` out. */
export const uart = defineLogicDecoder({
  name: 'uart',
  category: 'bus',
  config: {
    defaults: {
      line: 0,
      baud: 115_200,
      dataBits: 8,
      parity: 'none',
      stopBits: 'one',
      bitOrder: 'lsb-first',
      inverted: false,
      samplePoint: 50,
    },
    validate,
  },
  requires: (config) => ({
    channels: atLeast(config.line + 1),
    rateHz: atLeast(2 * config.baud),
  }),
  output: uartEvents,
  // The bit time in ticks is not among them: it depends on the source's rate, which a reader
  // learns from its own time base.
  props: (config) => ({
    baud: config.baud,
    dataBits: config.dataBits,
    parity: config.parity,
    stopBits: config.stopBits,
    bitOrder: config.bitOrder,
  }),
  decoder: Uart,
})
