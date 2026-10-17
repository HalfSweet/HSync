// SPDX-License-Identifier: Apache-2.0

/**
 * The SPI decoder: words shifted on the sampling clock edge while chip select is asserted,
 * each chip select period a transaction.
 *
 * The rules, which the mode only parametrizes:
 *
 * - a bit is taken on the sampling edge of the clock: rising when CPOL equals CPHA (modes 0
 *   and 3), falling otherwise (modes 1 and 2);
 * - bits are taken only while chip select is asserted; a bus without chip select is one
 *   unframed stream of words, transaction 0, with no begin or end;
 * - a clock edge at the very tick chip select changes belongs to neither transaction;
 * - bits left over when chip select is released, the samples end or a gap starts do not make
 *   a word and are dropped;
 * - chip select found asserted at the first sample, or at the first sample after a gap,
 *   begins a transaction marked partial: its earlier words were not seen;
 * - a gap or the end of the samples while chip select is asserted aborts the transaction.
 */

import type { Channel, ItemOf, Levels, Range, Tick, bool, u64, u8 } from 'uiharu'
import {
  LogicDecoder,
  MAX_CHANNELS,
  at,
  atLeast,
  defineLogicDecoder,
  eventSchema,
  fromBool,
  mask,
  max,
  one,
  over,
} from 'uiharu'

/** A transaction: one chip select period. Numbered from 1 by the decoder; 0 stands for the
 * unframed stream of a bus without chip select. */
export type TxId = u64

/** Why a transaction ended. */
export type EndReason =
  /** Chip select was released: the transaction is whole. */
  | 'cs-release'
  /** The samples ended or were lost while chip select was still asserted. */
  | 'aborted'

/** What travels on an SPI event stream. */
export type SpiEvent =
  /** Chip select was asserted. `partial`: it was already asserted when the samples began (at
   * the start of the capture or after a gap), so earlier words of this transaction were not
   * seen. */
  | { kind: 'begin'; tx: TxId; partial: bool }
  /** A word was shifted on both lines; a line that is not connected reads `undefined`. */
  | { kind: 'word'; tx: TxId; mosi: u8 | undefined; miso: u8 | undefined }
  /** The transaction ended. */
  | { kind: 'end'; tx: TxId; reason: EndReason }

/** The schema of SPI event streams: framed by chip select, a transaction's items are its
 * words. */
export const spiEvents = eventSchema<SpiEvent>('spi.events', 1).framed({
  begin: 'begin',
  items: ['word'],
  end: 'end',
  id: 'tx',
  partial: 'partial',
  reason: 'reason',
  normal: ['cs-release'],
  unframed: 0,
})

/** One word as both lines carried it, as decoders built on SPI read it. */
export type SpiWord = ItemOf<typeof spiEvents>

/** Which bit of a word is shifted first. */
export type BitOrder = 'msb-first' | 'lsb-first'

/** How the bus is wired and clocked. */
export interface SpiConfig {
  /** The clock channel. */
  clk: Channel
  /** The chip select channel, if the bus has one. */
  cs: Channel | undefined
  /** The master-out channel, if connected. */
  mosi: Channel | undefined
  /** The master-in channel, if connected. */
  miso: Channel | undefined
  /** The SPI mode, 0 to 3: bit 1 is CPOL, bit 0 is CPHA. */
  mode: u8
  /** Bits per word, 1 to 8. */
  wordBits: u8
  /** Which bit comes first. */
  bitOrder: BitOrder
  /** Whether chip select is asserted low. */
  csActiveLow: bool
}

/** Clock polarity: whether the clock idles high. */
export function cpol(config: SpiConfig): bool {
  return (config.mode & 0b10) !== 0
}

/** Clock phase: whether bits are taken on the second edge of each clock period. */
export function cpha(config: SpiConfig): bool {
  return (config.mode & 0b01) !== 0
}

/** Whether bits are taken on the rising edge of the clock. */
export function samplesOnRising(config: SpiConfig): bool {
  return cpol(config) === cpha(config)
}

/** Whether `levels` show chip select asserted; always, without a chip select line. */
export function selected(config: SpiConfig, levels: Levels): bool {
  return config.cs === undefined ? true : levels.high(config.cs) !== config.csActiveLow
}

/** The highest channel the decoder reads. */
function highest(config: SpiConfig): u8 {
  return max(max(config.clk, config.cs ?? 0), max(config.mosi ?? 0, config.miso ?? 0))
}

/** Checks the configuration. */
function validate(config: SpiConfig): string | undefined {
  if (config.mode > 3) return `SPI mode ${config.mode} does not exist; modes are 0 to 3`
  if (config.wordBits < 1 || config.wordBits > 8) {
    return `${config.wordBits} bits per word; 1 to 8 are supported`
  }
  if (config.mosi === undefined && config.miso === undefined) {
    return 'neither MOSI nor MISO is connected'
  }
  let seen = mask()
  for (const line of [config.clk, config.cs, config.mosi, config.miso]) {
    if (line === undefined) continue
    if (line >= MAX_CHANNELS) {
      return `channel ${line} is out of range; channels are 0 to ${MAX_CHANNELS - 1}`
    }
    if (seen.has(line)) return `channel ${line} is used for two lines`
    seen = seen.with(line)
  }
  return undefined
}

/** The transaction under way. */
interface Transfer {
  tx: TxId
  /** Bits taken into the current word. */
  bits: u8
  mosi: u8
  miso: u8
  /** The tick of the current word's first bit. */
  firstBit: Tick
}

function newTransfer(tx: TxId): Transfer {
  return { tx, bits: 0, mosi: 0, miso: 0, firstBit: 0 }
}

class Spi extends LogicDecoder<SpiConfig, SpiEvent> {
  readonly watch = mask(this.config.clk, this.config.cs)
  readonly framed = this.config.cs !== undefined
  nextTx: u64 = 1
  /** Without chip select every word belongs to transaction 0, which never begins or ends. */
  transfer: Transfer | undefined = this.framed ? undefined : newTransfer(0)
  selected = false

  /** Progress promises that nothing starting earlier follows: while a word is under way,
   * progress stops at its first bit. */
  override hold(): Tick | undefined {
    return this.transfer !== undefined && this.transfer.bits > 0
      ? this.transfer.firstBit
      : undefined
  }

  override async onGap(range: Range): Promise<void> {
    await this.abort(range.start)
  }

  override async onEnd(tick: Tick): Promise<void> {
    await this.abort(tick)
  }

  /** The samples stop at `tick`, for a gap or for good: the transaction under way is aborted;
   * on a bus without chip select its leftover bits are dropped. */
  async abort(tick: Tick): Promise<void> {
    if (this.transfer === undefined) return
    if (this.framed) {
      const tx = this.transfer.tx
      this.transfer = undefined
      await this.out.emit({ kind: 'end', tx, reason: 'aborted' }, at(tick))
    } else {
      this.transfer.bits = 0
      this.transfer.mosi = 0
      this.transfer.miso = 0
    }
  }

  begin(): TxId {
    const tx = this.nextTx
    this.nextTx += 1
    this.transfer = newTransfer(tx)
    return tx
  }

  /** Takes a bit from the data lines on the sampling edge at `tick`; the word goes out once
   * whole. */
  async shift(tick: Tick, levels: Levels): Promise<void> {
    const config = this.config
    if (this.transfer === undefined) return
    const transfer = this.transfer
    if (transfer.bits === 0) transfer.firstBit = tick
    const mosi = fromBool(config.mosi !== undefined && levels.high(config.mosi))
    const miso = fromBool(config.miso !== undefined && levels.high(config.miso))
    if (config.bitOrder === 'msb-first') {
      transfer.mosi = (transfer.mosi << 1) | mosi
      transfer.miso = (transfer.miso << 1) | miso
    } else {
      transfer.mosi |= mosi << transfer.bits
      transfer.miso |= miso << transfer.bits
    }
    transfer.bits += 1
    if (transfer.bits < config.wordBits) return
    const word: SpiEvent = {
      kind: 'word',
      tx: transfer.tx,
      mosi: config.mosi === undefined ? undefined : transfer.mosi,
      miso: config.miso === undefined ? undefined : transfer.miso,
    }
    const where = over(transfer.firstBit, tick + 1, tick)
    transfer.bits = 0
    transfer.mosi = 0
    transfer.miso = 0
    await this.out.emit(word, where)
  }

  override async run(): Promise<void> {
    // Each entry starts where levels are unknown: at the first sample and after every gap.
    const levels = await this.signal.sample()
    this.selected = selected(this.config, levels)
    if (this.framed && this.selected) {
      const tx = this.begin()
      await this.out.emit({ kind: 'begin', tx, partial: true }, one(this.signal.position))
    }
    for (;;) {
      const edge = await this.signal.nextChange(this.watch)
      const cs = this.config.cs
      if (cs !== undefined && edge.changed.has(cs)) {
        this.selected = selected(this.config, edge.levels)
        if (this.selected) {
          const tx = this.begin()
          await this.out.emit({ kind: 'begin', tx, partial: false }, one(edge.tick))
        } else if (this.transfer !== undefined) {
          const tx = this.transfer.tx
          this.transfer = undefined
          await this.out.emit({ kind: 'end', tx, reason: 'cs-release' }, one(edge.tick))
        }
        // A clock edge at this very tick belongs to neither transaction.
        continue
      }
      if (
        this.selected &&
        edge.changed.has(this.config.clk) &&
        edge.levels.high(this.config.clk) === samplesOnRising(this.config)
      ) {
        await this.shift(edge.tick, edge.levels)
      }
    }
  }
}

/** The SPI decoder node: logic samples in, `spi.events` out. */
export const spi = defineLogicDecoder({
  name: 'spi',
  category: 'bus',
  config: {
    defaults: {
      clk: 0,
      cs: 1,
      mosi: 2,
      miso: 3,
      mode: 0,
      wordBits: 8,
      bitOrder: 'msb-first',
      csActiveLow: true,
    },
    validate,
  },
  requires: (config) => ({ channels: atLeast(highest(config) + 1) }),
  output: spiEvents,
  props: (config) => ({
    wordBits: config.wordBits,
    bitOrder: config.bitOrder,
    mode: config.mode,
    cs: config.cs !== undefined,
    mosi: config.mosi !== undefined,
    miso: config.miso !== undefined,
  }),
  decoder: Spi,
})
