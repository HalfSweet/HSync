# Writing a decoder

How a protocol is added to `crates/uiharu-protocols`: its contract, its decoder, its synthesizer, its tests and its page. The SPI module (`src/spi/`) and the SPI NOR decoder (`src/spi_nor.rs`) are the reference for every rule here; [design.md](design.md) explains the runtime they run on.

## Clean room

uiharu is Apache-2.0 and written from scratch. A decoder is implemented from the protocol's **public specification** — the standard, the vendor datasheet, the application note — which the module documentation cites. Never open, read or translate `src-tauri/resources/decoders/` (DSView's GPL decoders), `crates/hsync-decode`, `crates/hsync-srd`, or any sigrok or DSView source; the feature checklist of a protocol (channels, options, what to report) comes from the task description, written in its author's own words, not from that code.

## Where things go

```text
crates/uiharu-protocols/src/<name>/mod.rs       re-exports
crates/uiharu-protocols/src/<name>/events.rs    the contract: event types, ids, schema id
crates/uiharu-protocols/src/<name>/decoder.rs   the node: config, validation, props, the async run loop
crates/uiharu-protocols/src/<name>/bus.rs       the cursor consumers read the stream through (framed contracts)
crates/uiharu-protocols/src/<name>/synth.rs     a waveform builder for tests and tools
crates/uiharu-protocols/tests/<name>.rs         integration tests on synthesized waveforms
docs/protocols/<name>.md                        the contract and the decoding rules, for users of the stream
```

A decoder built on another contract (SPI NOR on `spi.events`, an EEPROM on `i2c.events`) is a single file or module named after the protocol, with its own events and tests. Add the module to `src/lib.rs`; touch nothing else outside your module (not `uiharu-cli`, not `design.md`, no new dependencies — `serde`, `thiserror`, `proptest` and `uiharu_core::testing` are what there is).

## The contract

The contract is the event stream between layers — what a decoder emits and what decoders built on it read. Design it for the bus, not for the protocols above it.

- One schema per contract, `SchemaId::new("<name>.events", 1)`, implemented by the payload enum through `EventSchema`. A change that breaks readers is a new version.
- The payload is an enum with `#[serde(tag = "kind", rename_all = "kebab-case")]`, fields named as the standard names them, numbers as the narrowest integer that fits, `Option` for a line that is not connected or a value that could not be seen — never a made-up zero. Events carry bus data only, no interpretation and no UI text: what a byte means is the next layer's business.
- Where the bus frames its traffic (chip select, start/stop, a frame's SOF/EOF), the contract frames too: a `begin` carrying an id, the frame's events carrying that id, an `end` carrying the id and the reason — the normal one and the abnormal ones (`aborted` for a gap or the end of the samples). A frame already under way when the samples begin or resume after a gap begins with `partial: true`. Give the reasons the names a reader of the bus would use.
- The output declares in `Props` what readers may require: `wordBits`, `bitOrder`, which lines are present, the mode, the framing — camelCase keys, values a reader can compare for equality or as a lower bound (`Requires::equals`, `Requires::at_least`).
- A framed contract comes with a cursor consumers read it through, in the shape of `spi::SpiBus` and `spi::Transaction`: `next()` waits for the next frame, gap, progress or end; the frame cursor offers the frame's primitives only — the next word with every line's value, the next word or the normal end, draining — and `consumed()` for the provenance of what was read. Byte order, field widths, integer assembly and which line a protocol reads are the consuming decoder's own code: a contract is the more useful the less it knows about the protocols above it.

## The decoder

A decoder is a `Node`: a configuration struct with `Default`, `validate()` and `props()`; a `spec()` declaring one input (`InputSpec::new::<Arc<LogicBlock>>("logic", Requires::new().at_least("channels", highest + 1))` for a bus decoder, `InputSpec::new::<LowerEvent>("bus", requires)` for a stacked one) and one output (`OutputSpec::new::<Event>("events", props)`); typed port helpers (`Spi::input(node)`, `Spi::events(node)`); and `start`, which binds the ports and returns `Box::pin(run(…))`.

`run` is an `async fn` over the runtime's handles and follows these rules:

- Every read and write is an `await`: `SignalCursor::next_change(mask)` and `sample()`, `Inbox::recv`, a cursor's `word()`, `Outbox::emit/gap/progress/end`. Nothing is consumed by an operation that did not complete, so a decoder paused on output never reads its input again or sends an event twice; do not cache items to work around backpressure.
- State is what the protocol needs to continue — the frame under way, the bits of the current word, a bounded batch — and nothing more. Never hold a whole frame or transaction; every buffer has a declared bound (a `const`), and a batch goes out when it is full or when the frame ends.
- Behaviour must not depend on how the samples are cut into blocks, on where the reader pauses, or on queue sizes and budgets. The signal cursor carries the levels before its position across blocks; only the first sample of the stream and the first after a gap are never edges — use `sample()` there to learn the bus state, and begin a frame as `partial` when the bus is found busy.
- A gap or the end of the samples while a frame is under way ends it with the abnormal reason, then the gap or end is passed on; leftover bits never make a word; events of other channels changing at the very tick a framing line changes belong to no frame (state the rule in the module documentation, as `spi/decoder.rs` does). Progress items are passed on.
- Every event states its provenance: `Provenance::one(tick)` for an event decided at one sample, `Provenance::over(span, settled)` for one covering several — the span of the bits or words it is made of, `settled` the tick at which it became certain, so that the stream's events settle in order (a batch flushed by a frame's end settles at that end). Decoders on a contract take the provenance of what they consumed from the cursor (`tx.consumed()`) and pass it to the writer.
- A decoder built on a contract has a body for one frame and a recovery around it, as `spi_nor.rs` does: the body reads through the cursor and returns early on the cursor's error; the recovery closes what the body opened with the reason the error names (`Incomplete` for a frame that ended normally but too soon, `Aborted` for a cut-off frame, a gap or the end of input) and the main loop passes gaps, progress and the end down. Only the read that is allowed to see the normal end returns it; a frame the body did not finish is drained for it.
- Module documentation states the decoding rules as a bullet list, cites the specification, and names the limits (word widths, modes, what is out of scope).

## The synthesizer

`synth.rs` builds `LogicBlock`s of the bus for tests, the CLI's generator and benchmarks, in the shape of `spi::synth::SpiWaveform`: constructed from the decoder's config, the channel count, the samples per bit (or half period) and a start tick; methods for idle time, for framing and for one word or symbol honouring the config (mode, bit order, polarity); `position()` and `finish()`. Generators of higher protocols (`nor_read_words`) build word lists for the bus synthesizer rather than samples.

## Tests

- Unit tests in the modules for configuration validation, `props()`, and the serialized shape of each event kind (`serde_json`), as `spi/events.rs` does.
- Integration tests in `tests/<name>.rs` on synthesized waveforms through `uiharu_core::testing::Harness`: build it with a closure that adds and connects your node(s) and returns the output to read; `run_all` for one piece, `feed`/`feed_cut` for blocks of chosen sizes with gaps and pauses, `end()` to finish; `items`, `events()`, `tap()` for a second stream. Check exact events, spans and settled ticks for a representative frame in every mode or option the decoder has; the partial frame at the start; the frame cut off by the end of the samples; the gap inside a frame and the resynchronization after it; leftover bits; the line that is not connected; a value that shows up before its frame ends (incremental decoding).
- Call `uiharu_core::testing::assert_block_invariant` on at least one rich waveform with gaps: it runs the samples through every block size, reading pattern and queue configuration and demands the events of one uninterrupted run. A proptest over random traffic, as `tests/acceptance.rs` does for SPI → SPI NOR, is welcome where the protocol has many shapes.
- For a stacked decoder, test the requirements: a lower stream that does not satisfy them must be refused with `GraphError::Requirement` naming the property.
- Tests use `expect("…")`, never `unwrap()`; `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --all --check` and `cargo test -p uiharu-protocols` must pass.

## The page

`docs/protocols/<name>.md` is for users of the stream: the contract (each event kind, its fields and what they mean on the bus), the props the output declares, the requirements a stacked decoder has, the decoding rules (framing, sampling, what is marked `partial`/`aborted`, what is dropped), the configuration (lines, modes, options) and the limits. English, concise, tables for the events; no comparison with other tools.

## Commits

Fine-grained Conventional Commits with the `uiharu` scope, subjects imperative and in lower case, ≤ 72 characters, bodies saying why: `feat(uiharu): decode i2c transfers from logic samples` (contract, decoder, synthesizer and their tests in one commit, or the contract first when it stands alone), `test(uiharu): …` for tests added on their own, `docs(uiharu): …` for the page. lefthook formats and lints what is committed; never `--no-verify`.
