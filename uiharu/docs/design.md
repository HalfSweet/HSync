# uiharu runtime design

How the Rust runtime in `crates/uiharu-core` meets the requirements in [prd.md](prd.md), and how the reference protocols in `crates/uiharu-protocols` use it. This is the first step of the plan: everything a DSL decoder will compile to exists here, written by hand. When the rules below change, this document changes with them.

## 1. Decoders are resumable futures

A decoder is a Rust `async fn` that reads through an `Inbox<T>` (events) or a `SignalCursor` (logic samples) and writes through an `Outbox<T>`. Every read and write is an `await` point. An operation returns `Pending` in three cases, and completes nothing when it does:

| Wait           | When                                        | Who clears it                             |
| -------------- | ------------------------------------------- | ----------------------------------------- |
| `Wait::Input`  | the input queue has no item                 | the upstream node, or the source's `push` |
| `Wait::Output` | the output queue is full                    | a reader downstream, or the caller        |
| `Wait::Budget` | the node completed its budget of operations | the next round of `Pipeline::run`         |

Rust's `async` lowering is the "resumable state machine" of the PRD: the future's locals (the bytes of a 24-bit address read so far, the batch of data bytes, the word being shifted) live in a struct of fixed size with no heap growth and no runtime. A decoder paused on output never reads its input again or sends an event twice, because the suspended operation is the write itself; on resume the write retries, and the input it followed was consumed before. The plan for the compiler is to emit this very shape — `async fn`s over the same handles — and let `rustc` do the lowering, so the IR does not need its own state machine transform for the Rust backend.

Handwritten decoders follow three rules the compiler will enforce for DSL code: no buffer grows without a declared bound (the NOR data batch holds at most 32 bytes); nothing holds a whole transaction; every `await` is one of the runtime's operations.

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
| NOR `address`                   | the three address words                                        | the last one's last bit                            |
| NOR `data` (full batch)         | the words of the batch                                         | the last byte's last bit                           |
| NOR `data` (flushed by an end)  | the words of the batch                                         | the end that decided it                            |
| NOR `end` (complete)            | the SPI `end`                                                  | the release tick                                   |
| NOR `end` (incomplete, aborted) | the words read but not reported, through the transaction's end | the release, the gap's start or the end of samples |
| NOR `unknown-command`           | the opcode word (and the SPI `begin`)                          | the opcode's last bit                              |

### Items

A stream carries `Item<T>`: `Event(Event<T>)`, `Gap(TickRange)` (samples and whatever was on them are missing), `Progress(Tick)` (nothing more comes before this tick) and `End(Tick)` (nothing follows). Nodes pass gaps, progress and the end down the graph after closing what they had open.

### Broadcast queues

An output port is one bounded `Broadcast<T>` queue. Items are stored once; every reader — downstream nodes and the caller's exposed readers alike — keeps its own position, and an item is dropped once every reader has passed it. The slowest reader therefore holds the queue, and a full queue makes the producer wait: that is backpressure, and it reaches the source, whose `push` hands the block back when the source's own queue is full. Two `Progress` items in a row fold into one, so progress never fills a queue. The `End` stays in the queue so every later read finds it.

## 3. Logic samples

A `LogicBlock` holds `len` ticks of up to 64 channels, bit-packed per channel, least significant bit first; `from_interleaved` reads the sample-major layout of raw binary exports. A source takes `SignalItem`s: `Data(Arc<LogicBlock>)`, `Progress`, `Gap` and `End`. The first item decides where the stream starts; after it, blocks must follow each other or an announced gap exactly. A source that cannot keep its samples announces a `Gap` for what it dropped; the engine never drops anything itself.

The `SignalCursor` is a decoder's view of a source. `next_change(mask)` finds the next tick where a watched channel changes level and reports every watched channel that changed at that tick together with all levels there, so that channels changing at the same instant are handled consistently; `sample()` reads the levels at the cursor without moving it. The cursor keeps the levels just before its position across blocks, so an edge on a block's first sample is found like any other and **where the samples are cut into blocks does not change what a decoder sees**. Only the first sample of the stream and the first sample after a gap have nothing to compare with: they are never edges. The scan works a 64-sample word at a time.

## 4. The graph

`Graph` takes sources (`SourceSpec`: channels, time base, blocks held) and nodes (`Node`: a `NodeSpec` of typed ports and a `start` that binds them and returns the future). Ports are typed at compile time (`OutputRef<T>`, `InputRef<T>`) and checked again at run time, which is what a dynamic graph from the DSL will rely on:

- `connect` checks that both ports carry the same `SchemaId` (name and version) and that the output's `Props` satisfy the input's `Requires` (`Equals`, `AtLeast`); an input reads one output.
- `expose` names an output for the caller; names are unique.
- `build` checks that every input is connected, that there is no cycle (topological order), and that no node reads two time domains (sources are domains; nodes inherit the domain they read).

Diagnostics name ports as `node#index.port` and say what was required and what was found, e.g. `spi-nor-read24#1.bus requires wordBits = 8 of spi#0.events, which declares 4`.

The SPI output declares `wordBits`, `bitOrder`, `mode`, `cs`, `mosi`, `miso`; SPI NOR requires `wordBits = 8`, `bitOrder = "msb-first"`, `cs = true`, `mosi = true`, `miso = true`. The source declares `channels` and `rateHz`; SPI requires `channels >= ` the highest channel it reads + 1.

## 5. The executor

`Pipeline::run` polls the nodes in topological order, each with a fresh budget of operations, and repeats the pass until no node completed an operation: every unfinished node then waits for input that has not arrived or for an output the caller has not read. A node that only ran out of budget goes on in the next pass, after the others. Everything runs on the caller's thread, in a fixed order, with no clocks or threads involved; given the same items in the same order the pipeline does the same thing. `RunReport` says what moved and what each node waits for; `Stats` reports every queue's length, capacity and high-water mark.

### What determinism covers

Through the same graph, the same samples produce, per stream, the same events — same `seq`, `span`, `settled` and payload — however they are cut into blocks, however often and for how long the reader pauses, and whatever the queue sizes and budgets. The acceptance suite (`crates/uiharu-protocols/tests/acceptance.rs`) checks this with proptest. Two things are not covered, by design: `Progress` items, which say where time has got to and so depend on the blocks; and the order in which two different streams' items interleave when read together, which follows processing. The CLI's NDJSON is therefore compared one stream at a time.

### Memory

Queues are bounded by their capacity; the source holds at most its configured number of blocks plus the one each reader is scanning; a decoder's state is its future, of fixed size, plus its declared buffers. The acceptance suite runs a 20 000-byte read through eight-item queues and checks the high-water marks.

## 6. Recovery

The SPI decoder marks what it could not see:

- chip select found asserted at the first sample, or at the first sample after a gap, begins a transaction with `partial: true` — its earlier words were not seen and its word alignment is unknown;
- a gap or the end of the samples while chip select is asserted ends the transaction with `reason: "aborted"`;
- leftover bits never make a word; a clock edge at the very tick chip select changes belongs to no transaction.

The SPI NOR decoder reads through a `Transaction` cursor whose reads return an error instead of a word when the transaction ends, and the runtime around the body applies the PRD's policy:

| The body ran into               | `ReadError`        | The runtime emits for the open operation | Then                                              |
| ------------------------------- | ------------------ | ---------------------------------------- | ------------------------------------------------- |
| chip select released early      | `Incomplete`       | `end { reason: "incomplete" }`           | the next transaction                              |
| SPI `end { reason: "aborted" }` | `Truncated`        | `end { reason: "aborted" }`              | the next transaction (`truncated: abort-current`) |
| a gap                           | `Gap(range)`       | `end { reason: "aborted" }`              | the gap is passed on; resync at the next `begin`  |
| the end of the stream           | `EndOfInput(tick)` | `end { reason: "aborted" }`              | the end is passed on                              |
| a partial transaction           | —                  | nothing                                  | its words are drained                             |

Only `read_u8_or_end` ever returns the normal end; a body that returns before the end has the rest of its transaction drained for it. An incomplete transaction is never called complete: `complete` is emitted only by the body, after `read_u8_or_end` saw the normal end.

## 7. From the DSL to the Rust API

| PRD (TypeScript)                         | Phase 1 (Rust)                                                                         |
| ---------------------------------------- | -------------------------------------------------------------------------------------- |
| `eventSchema<SpiEvent>("spi.events", 1)` | `impl EventSchema for SpiEvent { const SCHEMA = SchemaId::new("spi.events", 1) }`      |
| `defineDecoder({ input, output, … })`    | `impl Node` with a `NodeSpec` of `InputSpec`/`OutputSpec`                              |
| `requires: { wordBits: 8, … }`           | `Requires::new().equals("wordBits", 8)…`                                               |
| `async transaction(tx, out)`             | `async fn transaction(tx: Transaction<'_>, out: &mut NorOut) -> Result<(), ReadError>` |
| `tx.readU8("mosi")`                      | `tx.read_u8(Line::Mosi).await?`                                                        |
| `tx.readU24BE("mosi")`                   | `tx.read_u24_be(Line::Mosi).await?`                                                    |
| `tx.readU8OrEnd("miso")`                 | `tx.read_u8_or_end(Line::Miso).await?` → `Read::Value(u8)` or `Read::End`              |
| `tx.drain()`                             | `tx.drain().await?`                                                                    |
| `out.begin("read", tx.id)`               | `out.begin(Command::Read, tx.id(), tx.consumed()).await`                               |
| `recovery: { truncated, gap }`           | `recover()` around the body, as in §6                                                  |
| `g.source("logic")`                      | `graph.source(SourceSpec::new(…))`                                                     |
| `g.node(SpiDecoder, { clk, … })`         | `graph.add(Spi::new(SpiConfig { … })?)`                                                |
| `g.connect(a.outputs.x, b.inputs.y)`     | `graph.connect(Spi::events(spi), SpiNorRead24::input(nor))?`                           |
| `g.expose(name, port)`                   | `graph.expose(name, port)?` then `pipeline.exposed(ref)`                               |

## 8. NDJSON

`uiharu decode` writes one line per item: `{"stream", "seq", "span": [start, end], "settled", "event"}` for events, `{"stream", "gap": [start, end]}`, `{"stream", "end": tick}`, and with `--progress` `{"stream", "progress": tick}`. Event payloads are the contracts' types as serde serializes them: a `kind` tag in kebab-case (`begin`, `word`, `end`, `address`, `data`, `unknown-command`) and the fields by name.

## 9. Limits of this step

- Sources have at most 64 channels; SPI words have at most 8 bits; SPI is single-line (no dual or quad I/O).
- One pipeline runs on one thread; a capture thread hands blocks over through `Arc<LogicBlock>`.
- `Progress` is forwarded, not generated: a node does not announce that it has consumed a block, so a merged view of several streams has no watermark yet.
- The engine is not wired into HSync: an adapter from `hsync-capture` to `LogicBlock`s, and from exposed streams to the data stream and the timeline, comes with the integration.

Next: the decoder IR extracted from these two decoders, the TypeScript compiler that produces it and emits this Rust, and the performance work the benchmark (`uiharu bench`) is there to measure.
