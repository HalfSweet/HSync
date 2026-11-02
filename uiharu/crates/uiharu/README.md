# uiharu

A streaming protocol decode engine for logic analyzer software, automated tests and
waveform tools. Written in Rust and licensed under Apache-2.0.

Decoders process blocks of logic samples, exchange typed events through bounded
queues with backpressure, and compose into a checked decode graph. Events within
each output stream are independent of block boundaries and reader pauses.

```sh
cargo add uiharu
```

Using a published crate requires Rust 1.90 or newer. Node/pnpm is needed only to
develop the TypeScript decoders in a source checkout; published packages already
contain the generated Rust.

## A typed SPI pipeline

```rust
use std::sync::Arc;
use uiharu::{Graph, SignalItem, SourceSpec, TimeBase};
use uiharu::protocols::spi::{Spi, SpiConfig};

# fn main() -> Result<(), Box<dyn std::error::Error>> {
let mut graph = Graph::new();
let source = graph.source(SourceSpec::new("logic", 4, TimeBase::new(25_000_000)))?;
let spi = graph.add(Spi::new(SpiConfig::default())?);
graph.connect(source.samples, Spi::input(spi))?;
let output = graph.expose("spi", Spi::events(spi))?;
let mut pipeline = graph.build()?;
let events = pipeline.exposed(output).expect("exposed once");

// For each bit-packed LogicBlock supplied by your capture source:
# let block = uiharu::LogicBlockBuilder::new(uiharu::Tick::ZERO, 4);
# let mut block = block;
# block.push_repeated(uiharu::Levels(2), 20);
# let block = block.finish()?;
pipeline.push(source.id, SignalItem::Data(Arc::new(block)))?;
pipeline.run();
while let Some(item) = events.next() {
    // Typed SPI events, gaps, progress and end-of-stream notifications.
    println!("{item:?}");
    if matches!(item, uiharu::Item::End(_)) {
        break;
    }
}
# Ok(())
# }
```

The complete [SPI example](https://github.com/HalfSweet/HSync/blob/main/uiharu/crates/uiharu/examples/spi.rs)
generates and decodes a transaction. Add another decoder to consume the SPI events,
for example SPI NOR flash, SD cards or an nRF24L01+ radio.

## Features

| Feature     | Default | Provides                                                                                                |
| ----------- | ------- | ------------------------------------------------------------------------------------------------------- |
| `protocols` | Yes     | Built-in decoders under `uiharu::protocols`                                                             |
| `registry`  | No      | Named decoders, UI metadata, JSON pipeline descriptions and runtime IR loading under `uiharu::registry` |
| `testing`   | No      | Decoder acceptance harnesses under `uiharu::testing`, intended for dev-dependencies                     |

With `default-features = false`, only the core runtime is included. For a configurable
application, run `cargo add uiharu --features registry`.

## Command line

```sh
cargo install uiharu-cli --locked
uiharu gen flash.bin
uiharu decode flash.bin --stats
```

## Documentation and compatibility

- [Engine guide and protocol coverage](https://github.com/HalfSweet/HSync/tree/main/uiharu)
- [Architecture](https://github.com/HalfSweet/HSync/blob/main/uiharu/docs/design.md)
- [Writing decoders](https://github.com/HalfSweet/HSync/blob/main/uiharu/docs/writing-decoders.md)
- [TypeScript DSL and versioned IR](https://github.com/HalfSweet/HSync/blob/main/uiharu/docs/dsl.md)
- [Release and compatibility policy](https://github.com/HalfSweet/HSync/blob/main/uiharu/docs/releasing.md)

The `uiharu-*` crates share a release version independent of the HSync application.
Before 1.0, a minor version may introduce incompatible API changes; patch releases
preserve API compatibility. IR format compatibility is identified separately by
the module's `ir` field and `uiharu_ir::IR_VERSION`.
