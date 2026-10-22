# The decoder DSL and its IR

Protocol decoders will be written in a restricted TypeScript, compiled ahead of time — without a JavaScript runtime — through a decoder IR into Rust over the runtime of `uiharu-core`. This document defines the two: what a decoder looks like in the DSL and what the compiler turns it into. It comes from the hand-written decoders of `crates/uiharu-protocols`, which were written in the shape the compiler will emit: §1 says what they have in common, §2 defines the DSL, §3 what the runtime does around a decoder's body, §4 the IR, §5 how the IR maps to Rust, §6 what the examples cover and what is still missing, §7 the compiler's front end, §8 the interpreter that runs the IR as it is, for decoders loaded at run time; §5 also describes the backend and the runtime it emits against.

Where things are:

| Path                                     | Contents                                                                                                                                                                                                                                                                         |
| ---------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `dsl/src/index.ts`                       | The DSL's API as TypeScript declarations: integer types, provenance, the cursors, the batch, the decoder base classes, `defineLogicDecoder`, `defineStreamDecoder`, `defineBusDecoder`                                                                                           |
| `dsl/decoders/{pwm,spi,spi-nor,uart}.ts` | The reference decoders in the DSL, one per shape: a line decoder, a framed bus decoder, a decoder on a framed contract, a timed decoder. They type-check (`pnpm typecheck` covers `dsl/`) and compile to `dsl/ir/`                                                               |
| `dsl/contracts/i2c.ts`                   | A contract declared without a decoder: the types of `i2c.events` and its framing, for the hand-written I²C decoder and for DSL decoders built on I²C                                                                                                                             |
| `crates/uiharu-ir`                       | The IR as `serde` data types with its JSON form, and `validate`, the checks of the engine's rules; `tests/fixtures/pwm.json` is `pwm.ts` lowered by hand                                                                                                                         |
| `compiler/src/`                          | The compiler's front end: loads the `dsl/` project through the TypeScript compiler API, checks the subset and lowers decoder and contract modules to local IR (`pnpm dsl:compile`); `compiler.test.ts` holds its tests                                                           |
| `crates/uiharu-codegen`                  | The backend: Rust from the IR, over `uiharu_core::dsl`; `pnpm dsl:generate` generates production and backend-fixture Rust; tests check reproducibility                                                                                                                           |
| `crates/uiharu-interp`                   | The interpreter: runs a module as the Rust generated from it would run, without generating or compiling anything, for decoders loaded at run time (§8); `tests/equivalence.rs` checks it against the generated decoders, `tests/constructs.rs` on the constructs they do not use |
| `crates/uiharu-dsl-protocols`            | The handwritten `src/lib.rs` wrapper exposes decoders and contracts generated locally into ignored `src/generated/`; `tests/contracts.rs` checks the generated contracts' JSON                                                                                                   |
| `crates/uiharu-core/src/dsl.rs`          | The runtime generated code runs on: the main loops and the operation wrappers of §3, `FramedBus<E: Framed>`, `Batch<T, N>`, `Cut`, `Int`                                                                                                                                         |
| `dsl/ir/`                                | Disposable, ignored IR of the decoders and contracts as the compiler writes it; `pnpm dsl:generate` prepares it with Rust before direct workspace Cargo commands                                                                                                                 |

## 1. What the decoders have in common

Thirty-seven decoders were written by hand. Three things recur in all of them: the shape of their input, a small vocabulary of runtime operations, and a handful of idioms — the same boilerplate written again and again around protocol-specific code. The DSL keeps the protocol code, the compiler writes the idioms.

### Three shapes of input

| Shape      | Reads                                                             | Decoders                                                                                                                                                                          |
| ---------- | ----------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **logic**  | the source's samples through a `SignalCursor`                     | SPI, I²C, UART, CAN, 1-Wire link, I²S, PWM, timing, parallel, WS281x, USB, USB PD, JTAG, SWD, MDIO, Microwire, PS/2, Wiegand, AM230x, NEC, RC-5, RC-6, SIRC                       |
| **stream** | the events of an unframed lower stream through an `Inbox`         | Modbus RTU, LIN, MIDI, DMX512 (`uart.events`), AVR ISP (`spi.events`), APA102 on a bus without chip select                                                                        |
| **bus**    | the frames of a framed lower stream through the contract's cursor | SPI NOR, SD card, nRF24L01+, APA102 (`spi.events`), 93Cxx (`microwire.events`), 24xx (`i2c.events`), DS18B20 (`onewire.events`), USB transactions (`usb.packets`), 1-Wire network |

Logic decoders split again by what drives them: **edges** (SPI, I²C, PWM, JTAG, SWD, MDIO, Microwire, I²S, parallel, timing follow `next_change` and never ask what time it is) and **time** (UART, CAN, 1-Wire, USB, USB PD, WS281x, PS/2, Wiegand, AM230x and the infrared decoders turn bit times into ticks, `seek` to sample points and wait for edges with deadlines).

### The vocabulary

Everything a decoder waits for is one of these operations, and nothing else can make it wait:

| Input  | Operations                                                                                                         |
| ------ | ------------------------------------------------------------------------------------------------------------------ |
| logic  | `next_change(mask)`, `next_change_before(mask, until)`, `sample()`; and without waiting `seek(tick)`, `position()` |
| stream | `recv()`                                                                                                           |
| bus    | `bus.next()` → a frame cursor; on it `item()`, `item_or_end()`, `drain()`, `consumed()`, `id()`, `is_partial()`    |
| output | `emit(at, event)`, `gap(range)`, `progress(tick)`, `end(tick)`; `progress_sink().hold(from)`                       |
| time   | `io.time_base()`, `io.input_props(index)`                                                                          |

Every cursor over a framed contract (`spi::SpiBus`, `i2c::I2cBus`, `microwire::MicrowireBus`, `onewire::OneWireBus`, `usb::UsbBus`) has the same shape: `next()` waits for a frame, gap, progress or end; the frame cursor reads one item at a time, returns the normal end only from `item_or_end`, turns every other end into an error naming the cause (`Incomplete`, `Truncated`, `Gap`, `EndOfInput`, plus a line or field that is not there), records what it consumed for the provenance, and passes progress met inside a frame on, held back to what the decoder has not reported. What differs is the item type and which event kinds begin, carry and end a frame. That is a declaration, not code: a contract says how it is framed (§2.3) and the cursor is generated.

### The idioms

| Idiom                 | Where                                                                                                                                                                                                                                                                    |
| --------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| **Pass-through**      | Every decoder passes gaps, progress and the end down after closing what it had open, and forwards progress held back to the start of what it has under way (`min(t, in-flight start)`): SPI's first bit, UART's start edge, NOR's batch, Modbus's oldest unreported byte |
| **Resync**            | A logic decoder starts where levels are unknown, at the first sample and after every gap: it samples the bus state first, opens a frame found under way as `partial`, and goes back there after a gap                                                                    |
| **Cut**               | A gap or the end of the samples inside a frame unwinds the code reading it (`Result<_, Cut>` in UART and CAN, `ReadError` in every bus decoder), the frame is closed with the abnormal reason, and the control item is passed on                                         |
| **Body and recovery** | A bus decoder has a body for one frame and a table around it: which end reason each cut gets, placed over what was read but not reported; a partial frame is drained; what the body did not read is drained for it                                                       |
| **Bounded batch**     | Data goes out in batches of a declared size (`DATA_BATCH`, 32), with the joined provenance of its items, settled by its last item or by the end that decided it; the batch's start is what the decoder holds progress back to                                            |
| **Timed read**        | `seek(start + offset)` then `sample()`, progress forwarded held at `start`; a deadline through `next_change_before`, `Quiet` at the deadline                                                                                                                             |
| **Silence**           | A frame ends when its silence is known to have passed: by the next item, or by a progress tick, acted on by the tick it carries, never by its arrival                                                                                                                    |
| **Integer time**      | Bit times become ticks in integer arithmetic with a 128-bit intermediate (`rate × position / (200 × baud)`), rounded down or up as the protocol says                                                                                                                     |

The decoder-specific code — what a byte means, how fields are assembled, which line is read, what is reported — is ordinary imperative code over integers, records, enums and tagged unions. It does not need a special language, only a bounded one.

## 2. The DSL

A decoder is a TypeScript module that imports from `uiharu` (`dsl/src/index.ts`), declares its contract, its configuration and a class with the decoding logic, and registers them with `defineLogicDecoder`, `defineStreamDecoder` or `defineBusDecoder`. It is checked by the TypeScript compiler like any module; the uiharu compiler then checks the subset and lowers it. The module never runs as JavaScript, so JavaScript's semantics do not apply to it: integers have widths, records are values, and `===` compares values.

### 2.1 Types

| DSL                                            | Meaning                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                              |
| ---------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `u8`, `u16`, `u32`, `u64`                      | Unsigned integers of that width. A literal or a plain `number` fits any width; a narrower value widens on its own; a wider one does not fit a narrower variable (TypeScript reports it). Arithmetic is done at the width of its context — the declared type of what the result is assigned to, passed as, returned as or compared with — or, without one, at the width of its widest operand, and wraps there: `100 * halfBits` returned as a `u64` is a `u64` product, `(x << 1) \| bit` stored in a `u8` wraps at 8 bits. `/` and `%` by zero yield zero; a shift yields zero when the amount reaches the width; `as u8` truncates |
| `bool`                                         | `boolean`                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                            |
| `Tick`                                         | `u64`: a sample index on the source's clock. Ticks add and subtract like integers                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                    |
| `Channel`                                      | `u8`, a channel of the source; a configuration field of this type is offered as a channel by the interface                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                           |
| `'a' \| 'b'`                                   | An enum: a union of string literals, written as text in events and configurations                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                    |
| `interface R { … }`                            | A record: fields of these types. Records are values with one owner (a field of `this`, a local, a slot of an array). `const x = this.field` borrows the field when `x` is written through (`x.f = …`, or `x` passed to a function that writes through its parameter) and copies it otherwise; every other `const x = …` copies. In Rust every record is `Copy`                                                                                                                                                                                                                                                                       |
| `{ kind: 'a'; … } \| { kind: 'b'; … }`         | A tagged union on `kind`: events, results, anything with cases. Fields are reached under `switch (x.kind)` or `if (x.kind === 'a')`                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                  |
| `T \| undefined`                               | An option. It is read under `if (x !== undefined)`, through `x ?? d`, `x?.f`, or — in a bus decoder's body only — `x!`, which cuts the body as `missing` when the value is not there                                                                                                                                                                                                                                                                                                                                                                                                                                                 |
| `Bounded<T, N>`                                | An array of at most `N` items, read with `for … of` or a constant index below `N`; array literals are bounded by their length                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                        |
| `Batch<T>`                                     | `batch<T>(capacity)`: the one buffer a decoder keeps, with the provenance of what it holds (§2.5)                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                    |
| `Range`, `Provenance`, `Levels`, `ChannelMask` | The runtime's own types: `at`, `one`, `over`, `join` make provenance; `mask(...)`, `.has`, `.with` make and read masks; `levels.high(channel)` reads a level                                                                                                                                                                                                                                                                                                                                                                                                                                                                         |
| `string`                                       | Only as the message a configuration check returns                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                    |

Integer helpers: `min`, `max`, `saturatingSub`, `countOnes`, `fromBool`, `mulDiv` (`⌊a × b / c⌋` without overflowing in between) and `mulDivCeil`.

Native event contracts can also declare these exact wire types:

```ts
import type { HexU64, NumericEnum, u8 } from 'uiharu'

type ReadingBytes = readonly [u8, u8, u8, u8, u8]
type Clause = NumericEnum<{ c22: 22; c45: 45 }>
type Ack = 'ok' | 'wait' | 'fault' | { other: u8 }
type Reading = {
  kind: 'reading'
  bytes: ReadingBytes
  romId: HexU64
  clause: Clause
  ack: Ack
}
```

- A homogeneous tuple, named or inline and optionally `readonly`, is an exact
  fixed array. Its Rust type is `[T; N]`, and JSON input must contain exactly `N`
  items. Optional, rest and heterogeneous tuple elements are rejected. The
  supported length is 1–32, matching the native array serialization implemented
  by the current serde derives. A fixed array and `Bounded<T, N>` are distinct;
  there is no implicit conversion between them. Both support indexing and
  `for … of`; constructing a fixed array requires an exact-length array literal
  in its typed context.
- `HexU64` is a field annotation, not a different arithmetic type: a direct
  record or event field remains a Rust `u64`. JSON writes lowercase hexadecimal
  with a `0x` prefix and no padding; input accepts an unsigned integer or a
  hexadecimal string with an optional `0x`/`0X` prefix. It uses the existing
  `uiharu_core::hex_u64` representation, so ROM identities above 2^53 stay exact.
  Plain `u64` fields still require numbers. Optional/nested annotations and uses
  outside field declarations are rejected; calculations use `u64`. Source
  numeric literals must be safe integers; construct larger values with integer
  operations or supply them through an annotated field.
- `NumericEnum` preserves both symbolic variant names and numeric wire values:
  `Clause::C22`/`Clause::C45` serialize as `22`/`45`. A bare numeric literal union
  also works and receives generated names such as `Value22`. Assignments,
  equality and `switch` use the declared numeric literals; undeclared wire values
  are rejected. Arithmetic, ordering, increments and casts of a numeric enum are
  rejected rather than silently erasing its validation.
- A union of unit strings and single-property payload objects is an externally
  tagged enum. `Ack` generates `Ok`, `Wait`, `Fault` and `Other(u8)`; its JSON is
  exactly `"ok"`, `"wait"`, `"fault"` or `{"other": 7}`. Construct values with the
  same source literals and objects. Test unit variants with equality or a
  `switch`, and payloads with a TypeScript-compatible guard such as
  `typeof ack === 'object' && 'other' in ack` before reading `ack.other`.

Rust identifiers derived from enum wire strings are valid and collision-free;
punctuation or spellings that normalize to the same identifier never change the
wire strings. Early `break` inside a `switch` leaves that switch, including when
nested inside a loop; `continue` still targets the enclosing source loop.

### 2.2 Code

The subset is TypeScript without its dynamic parts: `let`/`const`, assignment and compound assignment, `if`/`else`, `switch` over an enum, a union's `kind` or integers (with `default`), `for (;;)`, `while`, `for (let i = a; i < b; i++)`, `for (const x of array)`, labelled `break`/`continue`, `return`, ternaries, `&&`/`||`/`!`, the arithmetic, bitwise and comparison operators, object literals of records and union variants, calls to the module's own functions and to the runtime's helpers, and `await` on the runtime's operations and the decoder's own `async` methods. Template strings make the messages of configuration checks.

Out: closures, classes other than the decoder's, `this` outside it, `any`, `number` arithmetic at JavaScript's width, floating point, strings beyond messages, exceptions, recursion, unbounded arrays and maps, `Promise` beyond `await`, generators, `new`, and anything from the JavaScript standard library.

Free functions are pure: synchronous, without access to the decoder, over their parameters alone (records are passed by reference). The decoder's methods have `this` and may be `async`; an `async` method is always awaited, a synchronous one never. Every loop waits, breaks or returns on every iteration, so that no loop can spin without yielding to the executor.

### 2.3 Contracts

```ts
export type SpiEvent =
  | { kind: 'begin'; tx: TxId; partial: bool }
  | { kind: 'word'; tx: TxId; mosi: u8 | undefined; miso: u8 | undefined }
  | { kind: 'end'; tx: TxId; reason: EndReason }

export const spiEvents = eventSchema<SpiEvent>('spi.events', 1)
  .roles({ begin: 'framing', word: 'data', end: 'framing' })
  .framed({
    begin: 'begin',
    items: ['word'],
    end: 'end',
    id: 'tx',
    partial: 'partial',
    reason: 'reason',
    normal: ['cs-release'],
    unframed: 0,
  })
```

`eventSchema` ties the event type to the schema name and version. `roles` says what each kind of event is to someone reading the bus, whatever the protocol, so that an interface can show events of the same role alike: `framing` (where a transaction, frame or scan begins or ends), `address` (whom the traffic is for), `command` (what is asked), `control` (what a frame says of itself, such as a CAN control field), `data` (what it carries), `check` (a CRC or checksum), `handshake` (the other side's answer), `line` (a condition of the line itself: a line reset, a break) or `error` (something the decoder reports as wrong). A role names what an event is, never how it looks. Every contract declares one for every kind of event, and the checker holds it to that. A kind's role can carry `errorWhen`, field values that make one of its events an error: `frame: { role: 'data', errorWhen: [{ parity: 'error' }, { framingOk: false }] }` makes a character with a wrong parity bit or a bad stop bit an error, and an event is one when it has every value of any one condition. The conditions name only what the contract defines as wrong: an I²C address no device acknowledged is an error, a data byte the master did not acknowledge is not, because that is how a read ends. `framed` declares how the stream frames its traffic: the event kind that begins a frame, the kinds that carry its items, the kind that ends it, the field that names the frame on all of them, the `begin` field saying the frame was under way, the `end` field saying why it ended and its values for a normal end, and the id of the one frame of a bus that has no framing line. A decoder reading the contract gets a cursor over its frames whose items are the item events without the frame's id: with one kind of item, its fields alone (`ItemOf<typeof spiEvents>` is `{ mosi: u8 | undefined; miso: u8 | undefined }`); with several — I²C's addresses, data bytes and repeated STARTs — their union tagged by `kind`, read with `switch (item.kind)`. The cursor is generated from the declaration (§5); it offers the contract's primitives — the next item, the normal end, draining — and nothing more.

A contract can be declared without a decoder: a module under `dsl/contracts/` with the event type, its enums and the `eventSchema` call, for a decoder that stays hand-written. Its Rust types are generated like any other's, the hand-written decoder emits them, and a decoder written in the DSL can stack on it. `contracts/i2c.ts` is one: the hand-written I²C decoder of `crates/uiharu-protocols` emits its types, and the 24xx EEPROM decoder reads them through `FramedBus`.

Independent producers may import that same contract as their output. The
generator reuses the owning module's Rust payload and `EventSchema` implementation;
the interpreter and catalog resolve the imported schema by both name and version.
Conflicting definitions of the same schema identity are rejected. Identical JSON
shapes declared as unrelated Rust types are not an implicit adapter.

Events carry bus data only, as the contract pages describe them; numbers take the narrowest width that fits, a line that is not connected or a value that could not be seen is `undefined`.

### 2.4 The decoder

```ts
class Pwm extends LogicDecoder<PwmConfig, PwmEvent> {
  readonly watch = mask(this.config.channel)
  leading: Tick | undefined = undefined
  trailing: Tick | undefined = undefined

  override hold(): Tick | undefined { return this.leading }
  override onGap(): void { this.leading = undefined; this.trailing = undefined }

  override async run(): Promise<void> {
    for (;;) {
      const edge = await this.signal.nextChange(this.watch)
      …
      await this.out.emit({ kind: 'cycle', periodTicks, activeTicks }, over(start, edge.tick, edge.tick))
    }
  }
}

export const pwm = defineLogicDecoder({
  name: 'pwm', category: 'line',
  config: { defaults: { channel: 0, polarity: 'active-high' }, validate },
  requires: (config) => ({ channels: atLeast(config.channel + 1) }),
  output: pwmEvents,
  props: (config) => ({ polarity: config.polarity }),
  locales: { 'en-US': { name: 'PWM', fields: { channel: 'Signal line' }, … }, 'zh-CN': { … } },
  decoder: Pwm,
})
```

The class is the decoder's state and logic. Its fields are the state that lives across frames and gaps, initialized in order with `this.config` and `this.rate` (the source's samples per second) in scope; its methods are the logic; a few method names are **hooks** the runtime calls:

| Hook                          | Kind          | Called                                                                                                                                                                               |
| ----------------------------- | ------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `run()`                       | logic, stream | at the first sample or item, where levels are unknown, and again after every gap. Returning from it means nothing more is decoded: the runtime passes the rest of the stream through |
| `transaction(tx)`             | bus           | once per frame, with the cursor over its items                                                                                                                                       |
| `recover(reason, at)`         | bus           | when a read cut the body short, with the reason the declared `recovery` maps the cut to, placed over what was read but not reported                                                  |
| `hold()`                      | all           | when progress is passed on: the tick from which the decoder holds what it has not reported yet, so that progress is held back to it                                                  |
| `onGap(range)`, `onEnd(tick)` | all           | when a read meets a gap or the end, before it is passed on: close what is open with the abnormal reason, forget what the lost samples may have changed                               |
| `onProgress(tick)`            | all           | when a read meets progress, before it is passed on: a decoder framed by silences closes a frame whose silence has passed here, by the tick carried                                   |

The definition names the decoder, its category for the catalog, its configuration (defaults and the check that rejects a configuration with a message), what its input requires of the output it reads (`atLeast`, `atMost`, `within`, `oneOf`, or a value to equal), its output's schema and the properties it declares, and the class. A bus decoder adds `partial` (`'drain'`, the default, or `'read'`: what to do with a frame whose beginning was not seen) and `recovery`: the reason `recover` gets for a frame that ended normally too soon (`incomplete`), for one cut off by the lower decoder (`truncated`), for an item without a value the body required with `!` (`missing`), for a gap and for the end of the stream. `locales`, optional, is what an interface shows for the decoder, by language tag (`en-US`, `zh-CN`…): its name, a one-line summary and the labels of its configuration fields, their enumerated values, its event kinds and their fields — literals only, typed against the configuration and the event union so that a misspelt name is a type error; what is left out shows as the identifier, or as the interface's own copy for a decoder of that name. The engine itself shows nothing: the copy is for an interface that loads the module (§8).

### 2.5 Reading, writing, holding

A logic decoder reads through `this.signal`: `nextChange(mask)` waits for the next edge and returns its tick, the channels that changed and all levels; `nextChangeBefore(mask, until)` gives up at the deadline with `{ kind: 'quiet', tick }`; `sample()` reads the levels at the cursor; `seek(tick)` moves it; `position` says where it stands. A stream decoder reads `this.input.next()`: the event with its provenance. A bus decoder reads the cursor its body is given: `tx.item()`, `tx.itemOrEnd()` (`undefined` at the normal end), `tx.drain()`, `tx.id`, `tx.partial`.

None of these returns a gap, progress or the end: progress is passed on by the runtime (after `onProgress`, held back to `hold()`), a gap or the end cuts the body and reaches the hooks (§3). A bus decoder's body is also cut by the frame's end where an item was expected, by a cut-off frame and by `!` on a missing value; the recovery closes it.

Every event states its provenance — `at(tick)` for a result decided at an instant, `one(tick)` for one covering a tick, `over(start, end, settled)` for one covering several ticks, settled where it became certain — except in a bus decoder, where `emit(event)` without it covers what was read through the cursor since provenance was last taken (`this.consumed()`), which is what the hand-written decoders pass by hand.

A batch (`batch<T>(capacity)`) is the one buffer a decoder keeps: `push(item, at)` joins the item's provenance into the batch's, `isFull` says it must go out, `take()` hands the items and their provenance over, `at` is where the batch starts for `hold()`. Pushing to a full batch is a programming error the compiler will flag where it can see it (a `push` not followed by a check of `isFull`).

## 3. What the runtime does around the body

The runtime's part is the same for every decoder of a kind; this is what the backend emits around the DSL code, in the shape of the hand-written main loops. Every operation of §2.5 is a wrapper over the runtime's call that turns progress into a forward and a gap or the end into a **cut**, an error that unwinds the body:

```text
nextChange(mask):
  loop
    match signal.next_change(mask):
      At(tick, changed, levels) → return edge
      Progress(t)               → onProgress(t); out.progress(min(hold() ?? t, t)); continue
      Gap(range)                → cut Gap(range)
      End(tick)                 → cut End(tick)
```

(`sample`, `nextChangeBefore` and a stream's `next` are the same; `nextChangeBefore` returns `quiet` at the deadline.) The main loop of a logic or stream decoder:

```text
initialize the fields in order, with config and rate in scope
loop
  match run():
    returned        → pass gaps, progress and the end through; stop
    cut Gap(range)  → onGap(range); out.progress(min(hold() ?? range.start, range.start)); out.gap(range); continue
    cut End(tick)   → onEnd(tick);  out.progress(min(hold() ?? tick, tick));               out.end(tick);  stop
```

Entering `run` again after a gap is the resync of the hand-written decoders: its locals are gone, the fields stay (which is what `onGap` is for), and the first read finds the first sample after the gap, which is never an edge. Progress is announced before a gap or the end goes out so that a reader framed by silences learns how long the line was quiet, which UART does by hand today.

A bus decoder's main loop is the hand-written SPI NOR loop with the table filled in from the declaration:

```text
loop
  match bus.next():
    Frame(tx)       → if tx.partial and partial == drain: tx.drain()
                      else: transaction(tx)
                      on a cut: recover(recovery[cut], placed)      — placed: the span of what was read but
                      finish the frame (drain what the body did not read) — not reported, or else the cut's tick;
                      on a cut there: recover again                   — settled at the cut's tick
    Gap(range)      → onGap(range); out.gap(range)
    Progress(t)     → onProgress(t); out.progress(min(hold() ?? t, t))
    End(tick)       → onEnd(tick); out.end(tick); stop
```

Progress met inside a frame is forwarded by the cursor, held back to what was consumed since provenance was last taken and to `hold()`. The cuts of a bus body: `incomplete` (the frame ended normally where an item was expected: the frame is whole, the command is not), `truncated` (the lower decoder aborted the frame), `missing` (`!` on a value that was not there), `gap`, `end`; the recovery maps each to a reason of the decoder's own, which `recover` emits for what it had open.

These loops are what makes a DSL decoder deterministic and bounded by construction: the body never sees how the samples were cut, never holds a control item, and every buffer it keeps is declared.

## 4. The IR

The compiler lowers a DSL module to one JSON document per decoder, read by `uiharu-ir` (`Module`). It is self-contained: the contracts it reads are copied into it with the types they need (`imports`), so that the backend needs nothing else.

```json
{
  "ir": 1, "name": "pwm",
  "types":     [ { "name": "PwmEvent",  "type": "union",  "variants": [ { "name": "cycle", "fields": [ … ] } ] },
                 { "name": "Polarity",  "type": "enum",   "variants": ["active-high", "active-low"] },
                 { "name": "PwmConfig", "type": "record", "fields": [ { "name": "channel", "type": "u8", "channel": true }, … ] } ],
  "contracts": [ { "schema": { "name": "pwm.events", "version": 1 }, "event": "PwmEvent" } ],
  "imports":   [ ],
  "consts":    [ ],
  "functions": [ { "name": "isActive", "params": [ … ], "ret": "bool", "body": [ … ] } ],
  "decoder": {
    "name": "pwm", "category": "line",
    "config":   { "type": "PwmConfig", "defaults": { "channel": 0, "polarity": "active-high" }, "validate": "validate" },
    "input":    { "kind": "logic" },
    "requires": [ { "key": "channels", "requirement": "at-least", "values": [ … ] } ],
    "output":   { "schema": { "name": "pwm.events", "version": 1 } },
    "props":    [ { "key": "polarity", "value": { … } } ],
    "locales":  { "en-US": { "name": "PWM", "summary": "…", "fields": { "channel": "Signal line" }, "values": { … }, "events": { … }, "eventFields": { … } }, "zh-CN": { … } },
    "state":    [ { "name": "leading", "type": { "option": "u64" }, "init": { "expr": "none" } }, … ],
    "methods":  [ { "name": "run", "async": true, "body": [ … ] }, { "name": "hold", "ret": { "option": "u64" }, "body": [ … ] }, … ]
  }
}
```

**Types** are strings — the primitives `bool`, `u8`, `u16`, `u32`, `u64`, `text`, `mask`, `levels`, `range`, `provenance`, `cursor` (the frame cursor of a bus decoder) or a declared name — or objects: `{ "option": T }`, `{ "array": T, "bound": N }`, `{ "fixedArray": T, "length": N }`, `{ "batch": T, "bound": N }`. Declared types are records (fields), enums (variant names as events spell them) and unions (variants with fields, tagged by `kind`). A contract names its schema, its event union, the role of each kind of event (§2.3: `"roles": { "frame": { "role": "data", "errorWhen": [{ "parity": "error" }] } }`, a lone condition written as a list of one; a module compiled before roles were declared has none, and its events show as data) and, when framed, its framing (§2.3: `items`, the kinds that carry a frame's items, and `normal`, the reasons of a normal end, are lists); an import carries another module's contract with its types. A module without a `decoder` declares contracts only.

The additive native-contract forms are `"type": "numeric-enum"` with
`"variants": [{ "name": "c22", "value": 22 }, …]`, and
`"type": "external-enum"` with `"variants": [{ "name": "ok" },
{ "name": "other", "payload": "u8" }]`. A field may carry
`"json": "hex-u64"` only when its type is `u64`. The field metadata survives
imports and generated framed-item types. Numeric enums use `enum` expressions
with logical variant names; external enums use `variant` expressions with no
fields for unit variants or one synthetic `value` field for a payload. This
reuses variant matching and bindings while keeping the native wire shape.
Existing IR version 1 modules remain valid without these optional forms.

**The decoder** has its ports: `input` is `{ "kind": "logic" }`, `{ "kind": "stream", "schema": … }` or `{ "kind": "bus", "schema": … }`; `requires` and `props` are evaluated with `config` in scope; `locales` is the copy of §2.4 by language tag, every part optional; `partial` and `recovery` (with the reason enum's name) belong to bus decoders. `state` lists the fields with their initializers; `methods` the functions with `self`, the hooks among them by name.

**Statements** are tagged by `stmt`: `let` (with `mutable` and an optional declared `type`), `let-ref` (a local borrowing a field of `self`), `assign` (with an optional compound `op`), `if`, `if-some` (an option tested and bound), `let-some` (bound, or an `else` that leaves the block; both carry `borrow` when the binding of a record field of `self` is written through), `match` (arms with a `variant` pattern binding fields, a `values` pattern of enum names or integers, or `default`), `loop`, `while`, `for` (a half-open integer range), `for-each` (an array), `break`, `continue`, `return`, `expr`. A function's parameters carry `mutable` when the function writes through them.

**Expressions** are tagged by `expr`: `lit`, `text`, `format`, `none`, `local`, `self` (a field), `field`, `index`, `tag` (a union's kind), `unary`, `binary`, `cast`, `record`, `variant`, `enum`, `array`, `call` (a free function), `method` (a synchronous method), `intrinsic` (a runtime helper), `ternary`, `if-some`, `if-variant`, `required` (`!`) and `await`. An `await` wraps an **operation** tagged by `op`: `next-change`, `next-change-before`, `sample` (logic), `next` (stream), `item`, `item-or-end`, `drain` (bus), `emit` (with `at` optional for bus decoders) and `method` (an async method).

`validate` checks what the compiler enforces and the backend relies on:

- every type, function, method, field, local, variant and intrinsic resolves, and nothing is declared twice or named after a primitive;
- every array and batch has a bound of at least one item; a fixed array has an exact length from 1 through 32;
- a free function is synchronous and has no `self`; only an `async` method awaits; an async method is called with `await`, a synchronous one without;
- nothing recurses: the call graph of functions and methods has no cycle, so that the decoder's state — the future — has a fixed size;
- every `loop` and `while` waits, breaks or returns on every iteration;
- the operations match the input: the signal's for a logic decoder, `next` for a stream decoder, the cursor's, `consumed`, `tx-id`, `tx-partial` and `required` for a bus decoder; an event of a logic or stream decoder states its provenance;
- while a local borrows a field of `self` (`let-ref`, or an `if-some`/`let-some` marked `borrow` because its binding is written through), no method of `self` is called — the borrow rule the Rust backend needs, kept simple enough to state;
- the hooks a decoder's kind calls for are there with the right shape (§2.4), and a bus decoder declares its recovery with variants of its reason enum;
- a framed contract's framing names variants and fields that exist, and its normal ends are variants of the reason enum;
- a contract's roles, when it has them, give every variant of its event union a role and name no other, and each error condition names fields of its kind with values those fields can take (a boolean, an integer, text, or the name of a variant of the field's enum);
- the copy a decoder carries names fields of its configuration, values of its enumerated fields, kinds of event it emits and fields of those events, under language tags;
- text is made only in free functions: the messages of configuration checks.

Typing is not repeated here: the TypeScript checker types the DSL on the way in and rustc checks the emitted Rust on the way out; the IR carries the declared types so that the backend can emit them.

## 5. From the IR to Rust

The backend emits one Rust module per IR module over `uiharu-core`, in the shape the hand-written decoders have — which is why they were written that way. `rustc`'s `async` lowering is the resumable state machine; the IR needs no state-machine transform.

| IR                                                | Rust                                                                                                                                                                                                                                            |
| ------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| a record, an enum, a union                        | a `struct`, a unit `enum` with `#[serde(rename_all = "kebab-case")]`, an `enum` with `#[serde(tag = "kind", rename_all = "kebab-case", rename_all_fields = "camelCase")]`                                                                       |
| `u8` … `u64`, `bool`, `option<T>`                 | the same; `u64` ticks become `Tick` at the runtime's API                                                                                                                                                                                        |
| `array<T, N>`, `fixedArray<T, N>`, `batch<T, N>`  | `Vec<T>` bounded by N, `[T; N]` with exactly N items, and the runtime's `Batch<T, N>`                                                                                                                                                           |
| a contract                                        | the event enum with `impl EventSchema`, and for a framed one the item type (a struct for one kind of item, an enum for several) and `impl Framed` (which variants begin, carry and end a frame) for the runtime's generic cursor `FramedBus<E>` |
| `config`, `requires`, `props`, `validate`         | the `Config` struct with `Default`, `validate()`, `props()`, and `Requires` in `NodeSpec`                                                                                                                                                       |
| `state`, `methods`                                | a `struct` of the fields and an `impl` with the methods; the hooks are methods the generated main loop calls                                                                                                                                    |
| a free function                                   | a `fn`; records as `&mut` parameters                                                                                                                                                                                                            |
| `let-ref`                                         | `let x = &mut self.field;` or `let Some(x) = &mut self.field else { … }`                                                                                                                                                                        |
| `if-some`, `let-some`, `match`, `if-variant`      | `if let Some(x)`, `let Some(x) = … else`, `match`                                                                                                                                                                                               |
| `+`, `-`, `*`, `/`, `%`, shifts, `cast`           | `wrapping_add`/`wrapping_sub`/`wrapping_mul`, `checked_div().unwrap_or(0)`, `checked_shl().unwrap_or(0)`, `as`; `mul-div` through `u128`                                                                                                        |
| an `await` on an operation                        | the generated wrapper of §3 with `?`: `self.next_change(mask).await?`                                                                                                                                                                           |
| a cut                                             | `Err(Cut::Gap(range))`, `Err(Cut::End(tick))`; for a bus body the cursor's `ReadError`                                                                                                                                                          |
| `emit` without `at`                               | `self.out.emit(self.bus.consumed(), event).await`                                                                                                                                                                                               |
| `hold`, `onGap`, `onEnd`, `onProgress`, `recover` | called by the generated main loop and wrappers (§3)                                                                                                                                                                                             |
| the module                                        | `impl Node`: `spec()` from the ports, `start()` binding them and returning `Box::pin(main_loop(…))`; a registry `Kind` and a manifest from the config fields and the event kinds                                                                |

The runtime side of this is `uiharu_core::dsl`: the generic cursor `FramedBus<E: Framed>` over every generated contract (its `impl Framed`, generated from the framing, says which events begin, carry and end a frame; the cursors still written by hand have the same shape), `Batch<T, const N: usize>` with its provenance, the `Cut` error, the `Int` arithmetic, and the decoder traits `LogicDecoder`, `StreamDecoder` and `BusDecoder` whose default methods are the operation wrappers of §3 and whose main loops (`run_logic`, `run_stream`, `run_bus`) drive the generated state. The backend is `crates/uiharu-codegen`: one IR module becomes one Rust module — the contract's types with their serde attributes and `impl EventSchema`, the configuration with `Default`, `validate`, `props` and `requires`, the free functions, the node (`impl Node`, `new`, `input`, `events`) and a private `State` with the decoder's fields and methods, plus the trait impl that adapts the hooks. The emitter types every expression as it goes, so that widths are reconciled (§2.1), options wrapped, records passed by reference and the arguments of a call on `self` that themselves await evaluated first. The generated sources are formatted with `rustfmt` and written to the ignored `crates/uiharu-dsl-protocols/src/generated/` directory. Its small handwritten `src/lib.rs` wrapper stays tracked. Code-generation tests regenerate production and backend-fixture Rust to check reproducibility, rather than comparing against committed generated files.

The generated decoders are the PWM, SPI, SPI NOR and UART decoders of the engine: `crates/uiharu-protocols` re-exports them under its own module names (`uiharu_protocols::spi::Spi` is `uiharu_dsl_protocols::spi::Spi`, with the names decoders built on SPI use: `SpiBus` is `FramedBus<SpiEvent>`, `Word` the item struct, `TxId` a `u64`), and the registry, the command line and the application build them from there. The hand-written sources they replaced are gone; what stayed hand-written is the synthesizers and, above the generated contracts, the decoders built on them: the SD card, APA102, AVR ISP and nRF24L01+ decoders read `spi.events` through `FramedBus`, Modbus RTU, LIN, MIDI and DMX512 read `uart.events` from their inbox. The proof that the compiler preserves the engine's properties is that the hand-written integration tests of the four protocols and the acceptance suite (`crates/uiharu-protocols/tests/`) pass on the generated decoders unchanged in what they expect; before the replacement, `tests/equivalence.rs` ran hand-written and generated side by side on random waveforms with the same result (PR #34).

## 6. Coverage

The four reference decoders exercise every construct of the DSL and the IR:

| Decoder                   | Shape                       | Exercises                                                                                                                                                                                          |
| ------------------------- | --------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `dsl/decoders/pwm.ts`     | logic, edges                | the smallest decoder: options, `hold`, `onGap`, provenance over a span; lowered by hand to `crates/uiharu-ir/tests/fixtures/pwm.json`                                                              |
| `dsl/decoders/spi.ts`     | logic, edges, framed output | a framed contract declared with `framed`, resync (`sample` at every entry of `run`, a `partial` begin), `let-ref` on a record field, an unframed bus (transaction 0), aborting on a gap or the end |
| `dsl/decoders/spi-nor.ts` | bus                         | a body over the cursor with `item`, `itemOrEnd`, `drain`, `!` and `consumed()`; the recovery table and `recover`; a batch with `hold`; state across frames forgotten on a gap; tables as `switch`  |
| `dsl/decoders/uart.ts`    | logic, time                 | `rate`, `mulDiv`, `seek` and `sample` at computed points, a bounded array literal, a decoder that returns from `run` when the source is too slow                                                   |

What the DSL does not express yet, and what the remaining decoders would need:

- **A node with several inputs** (`ready_all`, the time-ordered merge of design.md §4): no decoder uses one today; a `defineMergeDecoder` can come with the first that does.
- **A cursor that is not a frame cursor**: the 1-Wire link layer's `LinkBus` reads "a reset, then bits until something ends them" and names a bad slot among its errors; USB's `UsbBus` names a damaged packet. They are framed contracts with extra end reasons: the framing declaration will take a list of abnormal reasons with the cut each maps to.
- **Properties of the input**: Modbus RTU and LIN take the baud rate from the UART stream's props (`io.input_props`). A stream decoder's definition will declare which props it reads, typed, as `this.input.props`.
- **Several outputs**: no decoder has them; the IR's `output` can become a list when one does.

The rest of the decoders fit the three shapes and the vocabulary: the edge-driven logic decoders are SPI with other framing rules, the timed ones are UART with other timings and deadlines, the stream decoders are Modbus with other silences, the bus decoders are SPI NOR with other command sets.

## 7. The compiler

`compiler/src/` is the front end: `program.ts` loads the `dsl/` project through the TypeScript compiler API (and virtual files, for tests), `types.ts` maps TypeScript's types to the IR's — widths from the brand of `u8` … `u64`, the DSL's interfaces to the runtime's types, records, enums and unions by the names the module declares — and collects a file's type declarations, `lower.ts` lowers statements and expressions, and `module.ts` assembles the module: contracts and the ones imported with their types, constants, free functions, the definition and the class, then the two passes that decide what is written through (`mutable` parameters) and what borrows a field of the decoder. A module with TypeScript errors is not lowered; a construct outside the subset is reported as `file:line:column: message`.

TypeScript's narrowing is what the IR's binding constructs come from: `if (x !== undefined)` lowers to `if-some`, `if (x === undefined) leave` to `let-some` for the rest of the block, `if (x.kind === 'v')` and `switch (x.kind)` to `match` with the variant's fields bound as the arm reads them, `a !== undefined && …`, `a?.b`, `a ?? b` and `x === 0` on an option to their expression forms. A `const` naming a narrowed field (`const t = this.transfer` after the check) names the binding instead of copying. Everything else is ordinary lowering, one construct to one.

## 8. Running the IR as it is: the interpreter

The IR is also run directly, by `crates/uiharu-interp`, for a decoder that is not built into the engine: one under work, tried on a capture before its Rust is generated and the engine rebuilt, or one a user brings to the application. The interpreter is written so that what runs is what the generated decoder would do — the proof is the equivalence suite, which runs the interpreted reference decoders beside the generated ones on random waveforms and demands the same items, event for event, with the same provenance, gaps, progress and end, whatever the blocks, pauses and queues (`crates/uiharu-interp/tests/equivalence.rs`; `tests/constructs.rs` covers the constructs the reference decoders do not use, on modules written for them).

- **Loading.** `Program::load` checks the module as `uiharu-ir` does (§4) and compiles its functions into the instructions of a small stack machine. The compiler types every expression the way the backend does (§5: arithmetic at the width of its context when wider than its operands, options without a wrapper, records by value, with one owner) and resolves names to slots and indices; the control flow is flattened into jumps, calls push a frame, a record a function writes through its parameter is stored back where it came from. What the backend would not emit — a `tag` read outside a match, an integer local without its width, a value that does not fit — is refused with the path of the construct, as the validator names it.
- **Running.** `Interpreter::new(program, config)` is the node of a module for one configuration. The configuration is read from JSON as the generated `Config` is (its fields in snake case, the module's defaults for what is left out, unknown fields, wrong types and values out of range refused with serde's words), checked by the module's `validate`, and the ports, properties and requirements are those the generated node declares, so that a description runs on either. The node runs on the same main loops (`run_logic`, `run_stream`, `run_bus`) through drivers that call the module's hooks, so that everything around the body — progress held back to `hold`, cuts, the recovery table, the pass-through — is the runtime's own; `hold` runs outside the body and may not change the state. Every wait is one of the runtime's operations, awaited where the instruction is: nothing is consumed by an operation that did not complete.
- **Events.** An interpreted decoder's events are `DynEvent`s: the JSON of the contract, as its Rust type serializes it (design.md §8). Interpreted decoders exchange them as they are; a stream between an interpreted decoder and a compiled one goes through a bridge node (`ToDyn<T>`, `FromDyn<T>`) that converts through that JSON and declares the producer's properties, so that the consumer's requirements are checked against the real stream. A bus decoder's lower stream is framed by the contract's declaration as data (`DynFramer`, a `Framer` for `FramedBus`, which the generated decoders drive through `Declared<E>`, the framing their event type implements).
- **Faults.** A fault of the interpreter — a construct the checks let through, a hook that changes the state, an event of a lower stream that does not fit its contract — ends the decoder's decoding: what it emitted before stays, its stream passes through to the end, and the fault is reported through the node's `FaultHandle`, which the registry's `Built::faults` and the command line read.
- **Where it is used.** `uiharu_registry::Catalog` holds loaded modules beside the engine's node types, under the module's name, with a manifest from the declarations that carries the module's copy (`locales`, its configuration fields keyed as the manifest names them); a loaded module stands in for the node type of the same name, so that a decoder under work replaces the one built in, and `build_with` wires a description with a catalog, inserting the bridges where a connection crosses. `uiharu decode --ir <file>` loads a module on the command line.
- **Cost.** On the synthetic SPI NOR capture of `uiharu gen` (40 M samples, 632 400 SPI words, release build), the SPI → SPI NOR pipeline takes 0.5 s compiled, 1.2 s with SPI NOR interpreted over the compiled SPI, and 6.6 s with both interpreted: the edge-driven bus decoder pays about twelve times its compiled cost, a device decoder over a compiled bus about two and a half; the output is the same to the byte.

### Local generation workflow

TypeScript decoder and contract sources are tracked; generated IR and Rust are disposable, ignored local output. Run these commands from the repository root after installing the Node/pnpm dependencies and the Rust toolchain with rustfmt:

```sh
pnpm dsl:compile          # generate production and backend-fixture IR only
pnpm dsl:generate         # compile IR and generate both Rust output directories
pnpm dsl:check            # regenerate, then type-check generated Rust and fixtures
```

The output directories are:

| Output               | Path                                                            |
| -------------------- | --------------------------------------------------------------- |
| Production IR        | `uiharu/dsl/ir/`                                                |
| Backend-fixture IR   | `uiharu/crates/uiharu-interp/tests/fixtures/backend/ir/`        |
| Production Rust      | `uiharu/crates/uiharu-dsl-protocols/src/generated/`             |
| Backend-fixture Rust | `uiharu/crates/uiharu-interp/tests/fixtures/backend/generated/` |

`pnpm dsl:compile` prunes stale JSON from both IR directories. `pnpm dsl:generate` includes that step and generates both Rust directories. `pnpm dsl:check` runs the full generation step, then `cargo check --locked -p uiharu-dsl-protocols -p uiharu-interp --all-targets` to type-check production and backend-fixture Rust. It does not compare generated files with Git snapshots. Commit the TypeScript sources, never the generated IR or Rust. The small handwritten production `src/lib.rs` wrapper remains tracked.

Run `pnpm dsl:generate` before direct workspace Cargo build, fmt, clippy or test commands and before building or running CLI tools, examples or decoder metadata export. Direct `cargo tauri` also needs this preparation. `pnpm tauri dev` and `pnpm tauri build` use a wrapper that generates first; `pnpm bundle` inherits it. `pnpm bindings` also generates first. `pnpm test` and `pnpm test:watch` prepare only IR for frontend/compiler tests. Rust and bundle CI explicitly generate; frontend CI prepares only IR. Installer builds compile the generated Rust; the local IR directories are not shipped as runtime resources.

The compiler runs on Node's own TypeScript support (no build step; `erasableSyntaxOnly` keeps the sources to what it strips). Its tests (`pnpm test`) lower `pwm.ts` and compare the result with the tracked, hand-authored `crates/uiharu-ir/tests/fixtures/pwm.json`, compile production modules without diagnostics, and compile snippets outside the subset to see them reported. On the Rust side, `uiharu-ir`'s tests validate every file of `dsl/ir/`; code-generation tests check reproducibility and generated/interpreted parity tests check semantics. The hand-authored PWM fixture is independent test input and remains tracked.

## 8. Next

Decoders are written in the DSL or by hand as convenient: the four reference protocols are the DSL's, and the rest stay hand-written with their contracts in Rust — Microwire, 1-Wire, USB, SWD, JTAG, CAN, USB PD — until a decoder written in the DSL needs to read one, when the contract moves to `dsl/contracts/` and its cursor becomes an instance of `FramedBus`, as I²C's did. The DSL grows when a decoder needs it:

1. The subset (§6): typed input properties, cursors with extra abnormal reasons, nodes with several inputs.
2. Manifests for the interface, generated from the configuration record and the event kinds.
