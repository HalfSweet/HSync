// SPDX-License-Identifier: Apache-2.0

/**
 * The decoder DSL: what a protocol decoder is written against.
 *
 * A decoder is a TypeScript module that imports from `uiharu`, declares its contract (the
 * events it emits), its configuration and a class with the decoding logic, and registers the
 * three with `defineLogicDecoder`, `defineStreamDecoder` or `defineBusDecoder`. The module
 * never runs as JavaScript: the compiler type-checks it with TypeScript, lowers it to the
 * decoder IR and emits Rust over the runtime of `uiharu-core`. The rules of the subset are in
 * `docs/dsl.md`; this file declares the API, which is why everything in it is a `declare`.
 *
 * Integers carry their width in the type (`u8`, `u16`, `u32`, `u64`): a literal or a plain
 * `number` fits any width, a narrower value widens on its own, and a wider one does not fit a
 * narrower variable, which TypeScript reports. Arithmetic is done at the width of the wider
 * operand and wraps there; JavaScript's own number semantics never apply.
 */

declare const width: unique symbol
declare const channel: unique symbol
declare const bound: unique symbol

/** An unsigned 8-bit integer. */
export type u8 = number & { readonly [width]?: 8 }
/** An unsigned 16-bit integer. */
export type u16 = number & { readonly [width]?: 8 | 16 }
/** An unsigned 32-bit integer. */
export type u32 = number & { readonly [width]?: 8 | 16 | 32 }
/** An unsigned 64-bit integer. */
export type u64 = number & { readonly [width]?: 8 | 16 | 32 | 64 }
/** A yes or no. */
export type bool = boolean
/** A sample index on the source's clock, counted from its first sample. */
export type Tick = u64
/** A channel of the source, `0` to `MAX_CHANNELS - 1`; a configuration field of this type is
 * offered as a channel by the interface. */
export type Channel = u8 & { readonly [channel]?: true }

/** How many channels a source has at most. */
export declare const MAX_CHANNELS: u8

/** An array of at most `N` items: the one kind of list a decoder keeps or emits. A batch's
 * items fit a bound at or above the batch's capacity. */
export type Bounded<T, N extends number> = readonly T[] & { readonly [bound]?: N }

// Provenance

/** A half-open range of ticks, `start..end`. */
export interface Range {
  readonly start: Tick
  readonly end: Tick
}

/** Where an event comes from: the ticks it covers and the tick at which it became certain. */
export interface Provenance {
  readonly span: Range
  readonly settled: Tick
}

/** A result decided at `tick` that covers no ticks. */
export declare function at(tick: Tick): Provenance
/** A result covering the single tick `tick`. */
export declare function one(tick: Tick): Provenance
/** A result covering `start..end`, certain at `settled`. */
export declare function over(start: Tick, end: Tick, settled: Tick): Provenance
/** The provenance of a result built from both: the union of the spans, certain when the later
 * of the two was. */
export declare function join(a: Provenance, b: Provenance): Provenance

// Integer helpers, in the integer arithmetic the runtime does

/** The smaller of two integers. */
export declare function min<T extends u64>(a: T, b: T): T
/** The larger of two integers. */
export declare function max<T extends u64>(a: T, b: T): T
/** `a - b`, or zero when `b` is larger. */
export declare function saturatingSub<T extends u64>(a: T, b: T): T
/** How many bits of `value` are set. */
export declare function countOnes(value: u64): u8
/** `1` for `true`, `0` for `false`. */
export declare function fromBool(value: bool): u8
/** `⌊a × b / c⌋` without overflowing in between; zero when `c` is zero. */
export declare function mulDiv(a: u64, b: u64, c: u64): u64
/** `⌈a × b / c⌉` without overflowing in between; zero when `c` is zero. */
export declare function mulDivCeil(a: u64, b: u64, c: u64): u64

// Logic samples

/** The levels of every channel at one tick. */
export interface Levels {
  /** Whether `channel` is high. */
  high(channel: Channel): bool
}

/** A set of channels to watch for changes. */
export interface ChannelMask {
  /** Whether `channel` is in the set. */
  has(channel: Channel): bool
  /** The set with `channel` added. */
  with(channel: Channel): ChannelMask
}

/** The set of the given channels; a line that is not connected (`undefined`) is left out. */
export declare function mask(...channels: readonly (Channel | undefined)[]): ChannelMask

/** A tick where some watched channels changed. */
export interface Edge {
  readonly kind: 'edge'
  readonly tick: Tick
  /** The watched channels whose level differs from the tick before. */
  readonly changed: ChannelMask
  /** The levels of every channel at `tick`. */
  readonly levels: Levels
}

/** No watched channel changed before the deadline; the cursor now stands at `tick`. */
export interface Quiet {
  readonly kind: 'quiet'
  readonly tick: Tick
}

/**
 * A decoder's view of the source's samples: a position that only moves forward, with the
 * levels just before it kept across blocks. A gap or the end of the samples met by a read cuts
 * the decoder's `run` (see `LogicDecoder`); progress met by a read is passed on by the runtime,
 * held back to what `hold()` says.
 */
export interface SignalCursor {
  /** The next tick the cursor will look at. */
  readonly position: Tick
  /** Waits for the next tick at or after the cursor where a channel of `mask` changes level,
   * and moves past it. */
  nextChange(mask: ChannelMask): Promise<Edge>
  /** Like `nextChange`, but gives up at `until`: when nothing changes before that tick, the
   * cursor moves there and the result is `quiet`. With an empty mask this waits for the
   * samples up to `until` to arrive. */
  nextChangeBefore(mask: ChannelMask, until: Tick): Promise<Edge | Quiet>
  /** Waits for the sample at the cursor and reads every channel's level there, without
   * moving the cursor. */
  sample(): Promise<Levels>
  /** Moves the cursor forward to `tick`; a tick not after the position leaves it where it is. */
  seek(tick: Tick): void
}

// Event streams

/** An event read from a lower stream, with where it came from. */
export interface Received<E> {
  readonly event: E
  readonly at: Provenance
}

/** A decoder's reader on the events of an unframed lower stream (`uart.events` …). A gap or the
 * end met by a read cuts the decoder's `run`; progress is passed on by the runtime after
 * `onProgress`, held back to what `hold()` says. */
export interface Stream<E> {
  /** Waits for the next event and takes it. */
  next(): Promise<Received<E>>
}

// Framed contracts

/** A cursor over the items of one frame of a framed lower stream: a transaction of SPI, of
 * I²C, of Microwire … Every read consumes one item and can wait across blocks; nothing is read
 * twice when the decoder pauses between reads. The frame's end where an item was expected, a
 * cut-off frame, a gap and the end of the stream cut the decoder's `transaction`, which the
 * declared recovery then closes. */
export interface Transaction<I> {
  /** The frame's id, as the lower contract numbers it. */
  readonly id: u64
  /** Whether the frame's beginning was not seen: it was under way at the start of the samples
   * or after a gap. */
  readonly partial: bool
  /** Reads the next item; the frame's end, normal or not, cuts the body here. */
  item(): Promise<I>
  /** Reads the next item, or finds the frame's normal end (`undefined`). */
  itemOrEnd(): Promise<I | undefined>
  /** Consumes the rest of the frame up to its normal end. */
  drain(): Promise<void>
}

// Output

/** A decoder's writer on its output stream. */
export interface Outbox<E> {
  /** Sends an event, waiting for room in the queue. */
  emit(event: E, at: Provenance): Promise<void>
}

/** The writer of a bus decoder: an event emitted without provenance covers what was read
 * through the transaction cursor since provenance was last taken (`consumed()`). */
export interface BusOutbox<E> {
  emit(event: E, at?: Provenance): Promise<void>
}

/** The items taken out of a batch, with the provenance of all of them. */
export interface Taken<T> {
  readonly items: readonly T[]
  /** `undefined` when the batch was empty. */
  readonly at: Provenance | undefined
}

/** A buffer of at most `capacity` items waiting to go out in one event, with the provenance
 * of what it holds. A batch is the one buffer a decoder keeps; it has a declared bound. */
export interface Batch<T> {
  readonly length: u32
  readonly capacity: u32
  readonly isEmpty: bool
  readonly isFull: bool
  /** The provenance of the items held, `undefined` when there are none. */
  readonly at: Provenance | undefined
  /** Adds an item; the batch must not be full (a batch goes out when it is full, see
   * `isFull`). */
  push(item: T, at: Provenance): void
  /** Takes every item out. */
  take(): Taken<T>
  /** Drops every item. */
  clear(): void
}

/** A batch of at most `capacity` items. */
export declare function batch<T>(capacity: u32): Batch<T>

// Contracts

/** An event type: a tagged union on `kind`. */
export interface Tagged {
  readonly kind: string
}

/**
 * How a contract frames its traffic, for the cursor consumers read it through: which event
 * begins a frame, which carries its items, which ends it, the field that names the frame on all
 * three, the `begin` field that says the frame was already under way, the `end` field that says
 * why it ended and its value for the normal end. `unframed` is the id of the one frame of a bus
 * that has no framing line (SPI without chip select): it never begins or ends.
 */
export interface Framing<E extends Tagged, I extends E['kind'], Id extends string> {
  readonly begin: E['kind']
  readonly item: I
  readonly end: E['kind']
  readonly id: Id
  readonly partial?: string
  readonly reason: string
  readonly normal: string
  readonly unframed?: u64
}

/** The identity of an event stream's contract: its name and version, tied to the event type
 * and, for a framed contract, to the item type its cursor hands out. */
export interface Schema<E extends Tagged, I> {
  readonly name: string
  readonly version: u32
  readonly event?: E
  readonly item?: I
}

/** The contract of a stream as `eventSchema` declares it: unframed, unless `framed` says how
 * it frames its traffic. */
export interface UnframedSchema<E extends Tagged> extends Schema<E, never> {
  /** Declares the framing: `item` is the kind of event that carries a frame's items, and the
   * cursor hands them out without the `kind` and the frame's id. */
  framed<I extends E['kind'], Id extends string>(
    framing: Framing<E, I, Id>,
  ): Schema<E, Omit<Extract<E, { kind: I }>, 'kind' | Id>>
}

/** The item a framed schema's cursor hands out. */
export type ItemOf<S> = S extends Schema<Tagged, infer I> ? I : never

/** Declares the contract of a stream: `name` dotted and lower case (`spi.events`), `version`
 * from 1, a new version for every change that breaks readers. */
export declare function eventSchema<E extends Tagged>(name: string, version: u32): UnframedSchema<E>

// Ports

/** A value a port property or requirement may take. */
export type PropValue = bool | u64 | string

/** What an input requires of one property of the output it reads. */
export interface Requirement {
  readonly requirement: 'equals' | 'one-of' | 'at-least' | 'at-most' | 'within'
  readonly values: readonly PropValue[]
}

/** The property must be an integer of at least `least`. */
export declare function atLeast(least: u64): Requirement
/** The property must be an integer of at most `most`. */
export declare function atMost(most: u64): Requirement
/** The property must be an integer from `min` to `max`, both included. */
export declare function within(min: u64, max: u64): Requirement
/** The property must equal one of `values`. */
export declare function oneOf(...values: readonly PropValue[]): Requirement

/** Requirements by property name; a plain value requires the property to equal it. */
export type Requires = Readonly<Record<string, PropValue | Requirement>>
/** Properties by name. */
export type Props = Readonly<Record<string, PropValue>>

// Decoders

/** Where the catalog lists a decoder. */
export type Category = 'bus' | 'device' | 'line'

/** A decoder's configuration: its defaults and, when not every value is valid, the check that
 * rejects a configuration with a message. */
export interface ConfigSpec<C> {
  readonly defaults: C
  validate?(config: C): string | undefined
}

/**
 * What every decoder has: its configuration, the source's clock and the hooks the runtime calls
 * around its body.
 *
 * - `hold()` says from which tick on the decoder holds what it has not reported yet (the first
 *   bit of a word being shifted, the start edge of a character, the batch being filled):
 *   progress met by a read is passed on held back to it, so that the promise of progress holds
 *   for the event still to come.
 * - `onGap(range)` runs when a read meets a gap, before the gap is passed on: close what is
 *   open with the abnormal reason, forget what the lost samples may have changed. `run` is
 *   then entered again, where levels are unknown.
 * - `onEnd(tick)` runs when a read meets the end of the samples, before the end is passed on.
 * - `onProgress(tick)` runs when a read meets progress, before it is passed on: a decoder
 *   framed by silences closes a frame whose silence has passed here.
 */
export declare abstract class Decoder<C> {
  readonly config: C
  /** The source's samples per second; zero when unknown. */
  readonly rate: u64
  hold(): Tick | undefined
  onGap(range: Range): void | Promise<void>
  onEnd(tick: Tick): void | Promise<void>
  onProgress(tick: Tick): void | Promise<void>
}

/** A decoder of logic samples. `run` is entered at the first sample, where levels are unknown,
 * and again after every gap; returning from it means nothing more is decoded: gaps, progress
 * and the end are then passed on by the runtime. */
export declare abstract class LogicDecoder<C, E extends Tagged> extends Decoder<C> {
  readonly signal: SignalCursor
  readonly out: Outbox<E>
  abstract run(): Promise<void>
}

/** A decoder of an unframed lower stream. `run` is entered at the first item and again after
 * every gap; returning from it means nothing more is decoded. */
export declare abstract class StreamDecoder<
  C,
  L extends Tagged,
  E extends Tagged,
> extends Decoder<C> {
  readonly input: Stream<L>
  readonly out: Outbox<E>
  abstract run(): Promise<void>
}

/**
 * A decoder of a framed lower stream. `transaction` runs once per frame, reading it through the
 * cursor; when a read cuts it short, `recover` runs with the reason the declared recovery maps
 * the cut to, placed over what was read but not reported, and the rest of the frame is drained
 * for it. A frame whose beginning was not seen is drained without running the body, unless the
 * definition says `partial: 'read'`.
 */
export declare abstract class BusDecoder<C, I, E extends Tagged, R> extends Decoder<C> {
  readonly out: BusOutbox<E>
  /** The provenance of what was read through the cursor since this was last called (or an
   * event was emitted without provenance): an empty span where the last item was settled when
   * nothing was. */
  consumed(): Provenance
  abstract transaction(tx: Transaction<I>): Promise<void>
  abstract recover(reason: R, at: Provenance): Promise<void>
}

/** How a cut-off body of a bus decoder is closed: the reason `recover` gets for a frame that
 * ended normally too soon, for one cut off by the lower decoder, for an item without a value
 * the body required (`!`), for a gap and for the end of the stream. */
export interface Recovery<R> {
  readonly incomplete: R
  readonly truncated: R
  readonly missing: R
  readonly gap: R
  readonly end: R
}

/** A decoder type, as the registry offers it. */
export interface DecoderType {
  readonly name: string
}

export interface LogicDecoderDefinition<C, E extends Tagged> {
  readonly name: string
  readonly category: Category
  readonly config: ConfigSpec<C>
  /** What the decoder requires of the source: `channels`, `rateHz`. */
  requires?(config: C): Requires
  readonly output: Schema<E, unknown>
  /** What the output declares. */
  props?(config: C): Props
  readonly decoder: new () => LogicDecoder<C, E>
}

export interface StreamDecoderDefinition<C, L extends Tagged, E extends Tagged> {
  readonly name: string
  readonly category: Category
  readonly config: ConfigSpec<C>
  readonly input: Schema<L, unknown>
  /** What the decoder requires of the lower stream. */
  requires?(config: C): Requires
  readonly output: Schema<E, unknown>
  props?(config: C): Props
  readonly decoder: new () => StreamDecoder<C, L, E>
}

export interface BusDecoderDefinition<C, L extends Tagged, I, E extends Tagged, R> {
  readonly name: string
  readonly category: Category
  readonly config: ConfigSpec<C>
  readonly input: Schema<L, I>
  requires?(config: C): Requires
  readonly output: Schema<E, unknown>
  props?(config: C): Props
  /** What to do with a frame whose beginning was not seen: drain it (the default) or read it. */
  readonly partial?: 'drain' | 'read'
  readonly recovery: Recovery<R>
  readonly decoder: new () => BusDecoder<C, I, E, R>
}

/** Registers a decoder of logic samples. */
export declare function defineLogicDecoder<C, E extends Tagged>(
  definition: LogicDecoderDefinition<C, E>,
): DecoderType
/** Registers a decoder of an unframed lower stream. */
export declare function defineStreamDecoder<C, L extends Tagged, E extends Tagged>(
  definition: StreamDecoderDefinition<C, L, E>,
): DecoderType
/** Registers a decoder of a framed lower stream. */
export declare function defineBusDecoder<C, L extends Tagged, I, E extends Tagged, R>(
  definition: BusDecoderDefinition<C, L, I, E, R>,
): DecoderType
