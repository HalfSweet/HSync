# uiharu architecture

uiharu is a streaming protocol decode engine: it decodes digital captures while they are being taken, keeps its state and memory bounded, and produces the same events for a live capture and for the same samples replayed from a file. It is written in Rust, licensed under Apache-2.0, and meant to be embedded — in HSync, where it replaces the sigrok decoders, and in other logic analyzer, test and waveform tools.

The design has two chains. Protocol authors will write decoders in a restricted TypeScript DSL that compiles ahead of time, without a JavaScript runtime; users compose decode graphs through typed ports:

```text
compile chain:  TypeScript DSL → type and subset checks → decoder IR → resumable state machines → native Rust
run chain:      sample blocks → signal cursor → bus events (e.g. spi.events) → protocol events (e.g. nor.events) → UI / export
```

The runtime end of both chains exists today, written by hand in the shape the compiler will emit: `crates/uiharu-core` is the runtime (ticks, sample blocks and the signal cursor, the event envelope and bounded broadcast streams, typed decode graphs, the executor), `crates/uiharu-protocols` holds the decoders (each protocol a module with its contract, its decoder node, a waveform synthesizer and tests; see [writing-decoders.md](writing-decoders.md) for how one is written and `protocols/` for one page per protocol), and `crates/uiharu-cli` is the command line (`decode` to NDJSON, `gen`, `bench`). The DSL, its compiler and the IR come next; the app is wired to the engine after that.

Four properties shape everything below. **Streaming**: a decoder consumes blocks of samples as they arrive and emits every result as soon as it is certain; nothing waits for the end of a capture or of a transaction. **Bounded**: queues, decoder state and buffers have explicit limits; a source that cannot wait announces a gap, and the engine never drops samples itself. **Deterministic**: the same samples through the same graph yield the same events, however the samples are cut into blocks and however often the reader pauses. **Honest**: an incomplete transaction is never called complete, a line that is not connected reads `None`, and a result that could not be seen is marked, not guessed.

This document describes how the runtime achieves that and how the reference protocols use it. When the rules below change, this document changes with them.

## 1. Decoders are resumable futures

A decoder is a Rust `async fn` that reads through an `Inbox<T>` (events) or a `SignalCursor` (logic samples) and writes through an `Outbox<T>`. Every read and write is an `await` point. An operation returns `Pending` in three cases, and completes nothing when it does:

| Wait           | When                                        | Who clears it                             |
| -------------- | ------------------------------------------- | ----------------------------------------- |
| `Wait::Input`  | the input queue has no item                 | the upstream node, or the source's `push` |
| `Wait::Output` | the output queue is full                    | a reader downstream, or the caller        |
| `Wait::Budget` | the node completed its budget of operations | the next round of `Pipeline::run`         |

Rust's `async` lowering is the resumable state machine the design calls for: the future's locals (the bytes of a 24-bit address read so far, the batch of data bytes, the word being shifted) live in a struct of fixed size with no heap growth and no runtime. A decoder paused on output never reads its input again or sends an event twice, because the suspended operation is the write itself; on resume the write retries, and the input it followed was consumed before. The plan for the compiler is to emit this very shape — `async fn`s over the same handles — and let `rustc` do the lowering, so the IR does not need its own state machine transform for the Rust backend.

Handwritten decoders follow three rules the compiler will enforce for DSL code: no buffer grows without a declared bound (the NOR data batch holds at most 32 bytes); nothing holds a whole transaction; every `await` is one of the runtime's operations.

Contracts stay generic. The cursor over a contract offers that contract's primitives and nothing more — for `spi.events`, the next word with both lines' values (`Transaction::word`), the next word or the normal end (`word_or_end`), and draining (`drain`). Which line a protocol reads, byte order and field widths are the consuming decoder's own code: the NOR decoder assembles its 24-bit address from three words itself. A contract is the more useful the less it knows about the protocols above it, and a sequence of plain reads already has the property a multi-byte read needs: pausing between them keeps what was read, because that is the state of the future.

## 2. Streams

### Envelope

Every event carries `seq` (its number in its stream, events only), `span` (the ticks it covers on the signal; empty for a result decided at an instant) and `settled` (the tick at which the result became certain). Within one stream, events settle in order: no later event is settled earlier than an earlier one.

Provenance is explicit in handwritten Rust. A node reading events takes `Provenance` from its transaction cursor — `tx.consumed()` hands over the ticks of everything consumed since it was last called — and passes it to the writer. The compiler will insert these calls; a DSL author never sees them.

| Event                           | Span                                                           | Settled                                            |
| ------------------------------- | -------------------------------------------------------------- | -------------------------------------------------- |
| SPI `begin`                     | the tick chip select asserted (or the first sample seen)       | that tick                                          |
| SPI `word`                      | first bit's tick … last bit's tick + 1                         | the last bit's tick                                |
| SPI `end` (cs-release)          | the tick chip select released                                  | that tick                                          |
| SPI `end` (aborted)             | empty, at the gap's start or the end of the samples            | that tick                                          |
| NOR `begin`                     | SPI `begin` through the opcode word                            | the opcode's last bit                              |
| NOR `address`                   | the address words, three or four                               | the last one's last bit                            |
| NOR `dummy`, `status`, `id`     | the words they are read from                                   | the last one's last bit                            |
| NOR `data` (full batch)         | the words of the batch                                         | the last byte's last bit                           |
| NOR `data` (flushed by an end)  | the words of the batch                                         | the end that decided it                            |
| NOR `end` (complete)            | the SPI `end`                                                  | the release tick                                   |
| NOR `end` (incomplete, aborted) | the words read but not reported, through the transaction's end | the release, the gap's start or the end of samples |
| NOR `unknown-command`           | the opcode word (and the SPI `begin`)                          | the opcode's last bit                              |

### Items

A stream carries `Item<T>`: `Event(Event<T>)`, `Gap(TickRange)` (samples and whatever was on them are missing), `Progress(Tick)` and `End(Tick)` (nothing follows). Nodes pass gaps, progress and the end down the graph after closing what they had open.

`Progress(t)` is a promise: every later event of the stream has a span starting at or after `t` (and so settles at or after `t`). It lets a decoder built on time — a frame that ends after a silence — act on time passing without waiting for the next edge, provided it acts on the tick carried, never on the moment the item arrived. A source's first item is preceded by `Progress(start)`, so every stream knows where its samples begin. The signal cursor announces the end of a block it used up when the next one has not arrived; a decoder forwards progress it receives, but holds it back to the start of whatever it has under way (the first bit of a word being shifted, the start edge of a character) — forwarding `min(t, in-flight start)` — so that the promise holds for the event it will emit later. A decoder reading a bus through a contract's cursor does not see the progress inside a frame: the cursor passes it on itself through the decoder's output (`Outbox::progress_sink`), held back to what the decoder has consumed through it since it last took the provenance and to what the decoder says it still holds (`ProgressSink::hold`: provenance taken early for an event still to come, such as a batch being filled); the progress between frames comes back from `next()` for the decoder to pass on. Progress items depend on how the samples were cut and are excluded from determinism; folding keeps them from filling a queue.

### Broadcast queues

An output port is one bounded `Broadcast<T>` queue. Items are stored once; every reader — downstream nodes and the caller's exposed readers alike — keeps its own position, and an item is dropped once every reader has passed it. The slowest reader therefore holds the queue, and a full queue makes the producer wait: that is backpressure, and it reaches the source, whose `push` hands the block back when the source's own queue is full. Two `Progress` items in a row fold into one, so progress never fills a queue. The `End` stays in the queue so every later read finds it.

## 3. Logic samples

A `LogicBlock` holds `len` ticks of up to 64 channels, bit-packed per channel, least significant bit first; `from_interleaved` reads the sample-major layout of raw binary exports. A source takes `SignalItem`s: `Data(Arc<LogicBlock>)`, `Progress`, `Gap` and `End`. The first item decides where the stream starts; after it, blocks must follow each other or an announced gap exactly. A source that cannot keep its samples announces a `Gap` for what it dropped; the engine never drops anything itself.

The `SignalCursor` is a decoder's view of a source, and it only moves forward. `next_change(mask)` finds the next tick where a watched channel changes level and reports every watched channel that changed at that tick together with all levels there, so that channels changing at the same instant are handled consistently; `next_change_before(mask, until)` does the same but gives up at a deadline, moving the cursor there and returning `Quiet` — which is how a timed protocol waits for an edge within a bit, or (with an empty mask) for the samples up to a tick to arrive; `seek(tick)` jumps ahead, releasing the blocks passed; `sample()` reads the levels at the cursor without moving it. Decoders turn bit times and timeouts into ticks with the time base `Io::time_base()` gives them, in integer arithmetic.

`seek` jumps unconditionally: a gap ending before the sought tick is reported by the next read, and the cursor stays at the sought tick, so the samples between the gap's end and that tick are passed over. A decoder that must resynchronise at a gap's end instead waits with `next_change_before` (an empty mask if it only wants the samples up to a tick), which returns the gap first and leaves the cursor at its end.

The cursor keeps the levels just before its position across blocks and seeks, so an edge on a block's first sample, or at the very tick a seek landed on, is found like any other and **where the samples are cut into blocks does not change what a decoder sees**. Only the first sample of the stream and the first sample after a gap have nothing to compare with: they are never edges. When the cursor uses a block up and the next one has not arrived, it reports the block's end as `Progress` once, so that decoders built on time (a frame that ends after a silence) can see time pass without waiting for the next edge; these announcements depend on the blocks and are excluded from determinism, and a decoder must only use the tick they carry, never their arrival. The scan works a 64-sample word at a time.

## 4. The graph

`Graph` takes sources (`SourceSpec`: channels, time base, blocks held) and nodes (`Node`: a `NodeSpec` of typed ports and a `start` that binds them and returns the future). `start` receives an `Io` that, besides the ports, tells the node the time base of the source its inputs come from and the properties the outputs connected to its inputs declare, so a decoder can take its word width from the bus below it instead of asking twice. Ports are typed at compile time (`OutputRef<T>`, `InputRef<T>`) and checked again at run time, which is what a dynamic graph from the DSL will rely on:

- `connect` checks that both ports carry the same `SchemaId` (name and version) and that the output's `Props` satisfy the input's `Requires` (`Equals`, `OneOf`, `AtLeast`, `AtMost`, `Within`); an input reads one output. A graph built from a description names ports instead: `output`, `input`, `connect_dyn` and `expose_dyn` do the same with the same checks, and `Pipeline::exposed_dyn` refuses a reader of the wrong type rather than panicking.
- `expose` names an output for the caller; names are unique.
- `build` checks that every input is connected, that there is no cycle (topological order), and that no node reads two time domains (sources are domains; nodes inherit the domain they read).

Diagnostics name ports as `node#index.port` and say what was required and what was found, e.g. `spi-nor#1.bus requires wordBits = 8 of spi#0.events, which declares 4`.

### Several inputs

A node may read several inputs — two event streams, or a stream and the samples it refers to — on one condition: what it does must depend on the items alone, never on which input happened to fill first. It therefore waits until every input it still reads has an item (`ready_all`, which also takes a signal cursor), then takes the item whose time is earliest (`Item::time`: where an event's span starts, where a gap begins, the tick progress or the end announces), the lowest port on a tie. A node reading the samples beside a stream moves the cursor through them as the stream tells it (`seek` at every event and at every progress item), so that the blocks behind are let go: a seek drops the blocks and the progress the position is past; a gap stays, since the decoder must learn of missing samples and pass them on.

Inputs that share an upstream — a decoder's events and the source's samples — tie the node to the source's queue: the node must keep the samples from the earliest start the upstream has not reported yet, and the source holds at most its configured number of blocks. A graph whose lookback exceeds that stalls: every node waits, nothing can be read, and the source is full. `Pipeline::push` then returns `PushError::Stalled`, naming what each node waits for, instead of handing the block back as merely full; the cure is a source holding more blocks, or an upstream that reports sooner.

The command line builds graphs from a JSON description (`uiharu decode --graph`; the format is in the README): a source, named nodes with a type and a configuration deserialized by the node's config type, connections between `node.port` names, and the outputs to expose. It is the hand-written form of the graph description the DSL compiler will emit, and the registry of node types in `crates/uiharu-cli/src/pipeline.rs` — how to build each node from its configuration and how to write its output — is the one place a decoder is made available to the command line.

The SPI output declares `wordBits`, `bitOrder`, `mode`, `cs`, `mosi`, `miso`; SPI NOR requires `wordBits = 8`, `bitOrder = "msb-first"`, `cs = true`, `mosi = true`, `miso = true`. The source declares `channels` and `rateHz`; SPI requires `channels >= ` the highest channel it reads + 1.

## 5. The executor

`Pipeline::run` polls the nodes in topological order, each with a fresh budget of operations, and repeats the pass until no node completed an operation: every unfinished node then waits for input that has not arrived or for an output the caller has not read. A node that only ran out of budget goes on in the next pass, after the others. Everything runs on the caller's thread, in a fixed order, with no clocks or threads involved; given the same items in the same order the pipeline does the same thing. `RunReport` says what moved and what each node waits for; `Stats` reports every queue's length, capacity and high-water mark. A run that moved nothing, with nothing left for the caller to read, leaves the source full for good: the next `push` reports the stall (§4) rather than letting the caller spin.

### What determinism covers

Through the same graph, the same samples produce, per stream, the same events — same `seq`, `span`, `settled` and payload — however they are cut into blocks, however often and for how long the reader pauses, and whatever the queue sizes and budgets. The acceptance suite (`crates/uiharu-protocols/tests/acceptance.rs`) checks this with proptest. Two things are not covered, by design: `Progress` items, which say where time has got to and so depend on the blocks; and the order in which two different streams' items interleave when read together, which follows processing. The CLI's NDJSON is therefore compared one stream at a time.

### Memory

Queues are bounded by their capacity; the source holds at most its configured number of blocks plus the one each reader is scanning; a decoder's state is its future, of fixed size, plus its declared buffers. The acceptance suite runs a 20 000-byte read through eight-item queues and checks the high-water marks.

## 6. Recovery

The SPI decoder marks what it could not see:

- chip select found asserted at the first sample, or at the first sample after a gap, begins a transaction with `partial: true` — its earlier words were not seen and its word alignment is unknown;
- a gap or the end of the samples while chip select is asserted ends the transaction with `reason: "aborted"`;
- leftover bits never make a word; a clock edge at the very tick chip select changes belongs to no transaction.

The SPI NOR decoder reads through a `Transaction` cursor whose reads return an error instead of a word when the transaction ends, and the runtime around the body applies the recovery policy:

| The body ran into               | `ReadError`        | The runtime emits for the open operation | Then                                              |
| ------------------------------- | ------------------ | ---------------------------------------- | ------------------------------------------------- |
| chip select released early      | `Incomplete`       | `end { reason: "incomplete" }`           | the next transaction                              |
| SPI `end { reason: "aborted" }` | `Truncated`        | `end { reason: "aborted" }`              | the next transaction (`truncated: abort-current`) |
| a gap                           | `Gap(range)`       | `end { reason: "aborted" }`              | the gap is passed on; resync at the next `begin`  |
| the end of the stream           | `EndOfInput(tick)` | `end { reason: "aborted" }`              | the end is passed on                              |
| a partial transaction           | —                  | nothing                                  | its words are drained                             |

Only `word_or_end` ever returns the normal end; a body that returns before the end has the rest of its transaction drained for it. An incomplete transaction is never called complete: `complete` is emitted only by the body, after `word_or_end` saw the normal end.

## 7. From the DSL to the Rust API

| DSL (planned)                            | Phase 1 (Rust)                                                                                |
| ---------------------------------------- | --------------------------------------------------------------------------------------------- |
| `eventSchema<SpiEvent>("spi.events", 1)` | `impl EventSchema for SpiEvent { const SCHEMA = SchemaId::new("spi.events", 1) }`             |
| `defineDecoder({ input, output, … })`    | `impl Node` with a `NodeSpec` of `InputSpec`/`OutputSpec`                                     |
| `requires: { wordBits: 8, … }`           | `Requires::new().equals("wordBits", 8)…`                                                      |
| `async transaction(tx, out)`             | `async fn transaction(tx: Transaction<'_>, out: &mut NorOut) -> Result<(), ReadError>`        |
| `(await tx.word()).mosi`                 | `tx.word().await?.mosi()?` (`Word { mosi, miso }`; a line the input requires is never `None`) |
| `address = (address << 8) \| …`          | plain code in the decoder: `(address << 8) \| u32::from(tx.word().await?.mosi()?)`            |
| `tx.wordOrEnd()`                         | `tx.word_or_end().await?` → `Read::Word(word)` or `Read::End`                                 |
| `tx.drain()`                             | `tx.drain().await?`                                                                           |
| `out.begin("read", tx.id)`               | `out.begin(Command::Read, tx.id(), tx.consumed()).await`                                      |
| `recovery: { truncated, gap }`           | `recover()` around the body, as in §6                                                         |
| `g.source("logic")`                      | `graph.source(SourceSpec::new(…))`                                                            |
| `g.node(SpiDecoder, { clk, … })`         | `graph.add(Spi::new(SpiConfig { … })?)`                                                       |
| `g.connect(a.outputs.x, b.inputs.y)`     | `graph.connect(Spi::events(spi), SpiNor::input(nor))?`                                        |
| `g.expose(name, port)`                   | `graph.expose(name, port)?` then `pipeline.exposed(ref)`                                      |

## 8. NDJSON

`uiharu decode` writes one line per item: `{"stream", "seq", "span": [start, end], "settled", "event"}` for events, `{"stream", "gap": [start, end]}`, `{"stream", "end": tick}`, and with `--progress` `{"stream", "progress": tick}`. Event payloads are the contracts' types as serde serializes them: a `kind` tag in kebab-case (`begin`, `word`, `end`, `address`, `data`, `unknown-command`) and the fields by name. Ticks and counters are plain numbers. A field that may hold a value of 2^53 or more — a 1-Wire ROM code, the word of a parallel bus of more than 53 lines — is marked `#[serde(with = "uiharu_core::hex_u64")]` and written as a hexadecimal string (`"0x28ab0000c0ffee1d"`), which JavaScript's `JSON.parse` keeps exactly where it would round a number; binary formats get the integer.

## 9. Limits of this step

- Sources have at most 64 channels; SPI words have at most 8 bits; SPI is single-line (no dual or quad I/O).
- One pipeline runs on one thread; a capture thread hands blocks over through `Arc<LogicBlock>`.
- A node with several inputs merges them by time itself (§4); the runtime offers the readiness wait and the ordering key, not a merge node, and a lookback longer than the source's queue stalls rather than being spilled.
- `Progress` is generated only by the signal cursor, at the end of a block it used up while the next one was not there; every node passes it down, inside frames through the contract cursors, so that a reader of several streams can take the least of their watermarks.
- The engine is not wired into HSync: an adapter from `hsync-capture` to `LogicBlock`s, and from exposed streams to the data stream and the timeline, comes with the integration.

Next: the decoder IR extracted from these two decoders, the TypeScript compiler that produces it and emits this Rust, and the performance work the benchmark (`uiharu bench`) is there to measure.
