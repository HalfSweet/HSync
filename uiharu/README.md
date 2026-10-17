# uiharu

**A streaming protocol decode engine, written in Rust, licensed under Apache-2.0.**

uiharu decodes digital captures while they are being taken: decoders run as resumable state machines over blocks of logic samples, hand typed events to each other through bounded streams with backpressure, and are wired into a decode graph that checks its connections. The same code decodes a live capture and a file replayed later, and produces the same events however the samples are cut into blocks and however often the reader pauses.

It replaces the sigrok decoders in [HSync](../README.md) and is meant to be embedded in logic analyzer software, automated test tools and waveform analysis tools. The architecture is described in [docs/design.md](docs/design.md); [docs/writing-decoders.md](docs/writing-decoders.md) says how a decoder is written, and `docs/protocols/` has one page per protocol.

## Status

The runtime and the protocol decoders, written by hand in Rust in the shape the TypeScript DSL will take, with the acceptance tests of the design's properties; the engine decodes in HSync through `crates/hsync-uiharu`. The DSL and the decoder IR are defined in [docs/dsl.md](docs/dsl.md): the reference decoders written in the DSL are under `dsl/`, the IR with its checks in `crates/uiharu-ir`, the compiler's front end under `compiler/`, the backend that emits Rust from the IR in `crates/uiharu-codegen`, and the generated decoders in `crates/uiharu-dsl-protocols`, which produce the same events as the hand-written ones they were written from. Next, the generated decoders replace the hand-written ones in the registry.

| Crate                         | Contents                                                                                                                                                                                                                                         |
| ----------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `crates/uiharu-core`          | The runtime: ticks and time bases, bit-packed sample blocks and the signal cursor, the event envelope and bounded broadcast streams, typed decode graphs and the executor                                                                        |
| `crates/uiharu-protocols`     | The protocol decoders, each with its contract, synthesizer and tests: see [Protocols](#protocols)                                                                                                                                                |
| `crates/uiharu-registry`      | The node types by name: build a decoder from its JSON configuration, its manifest for a user interface, the pipeline description and its builder, outputs read as JSON items                                                                     |
| `crates/uiharu-cli`           | The `uiharu` command: `decode` a raw capture to NDJSON, `gen` a synthetic capture, `bench` the pipeline                                                                                                                                          |
| `crates/uiharu-ir`            | The decoder IR: what the DSL compiles to and the Rust backend emits from, as `serde` types with its JSON form, and the checks of the engine's rules on it                                                                                        |
| `dsl/`                        | The decoder DSL: its API as TypeScript declarations (`src/index.ts`), the reference decoders written in it (`decoders/`) and their IR as the compiler writes it (`ir/`)                                                                          |
| `compiler/`                   | The compiler's front end: checks the DSL subset and lowers decoder modules to the IR (`pnpm dsl:compile`, `pnpm dsl:check`)                                                                                                                      |
| `crates/uiharu-codegen`       | The backend: emits Rust over `uiharu_core::dsl` from the IR (`cargo run -p uiharu-codegen -- uiharu/dsl/ir uiharu/crates/uiharu-dsl-protocols/src`)                                                                                              |
| `crates/uiharu-dsl-protocols` | The reference decoders generated from the IR, checked in and tested for the same events as the hand-written ones                                                                                                                                 |
| `docs/`                       | The architecture (`design.md`), the DSL and its IR (`dsl.md`), how to write a decoder (`writing-decoders.md`), one page per protocol (`protocols/`) and the performance study with the comparison against the sigrok decoders (`performance.md`) |

The crates are members of the repository's Cargo workspace but depend on nothing outside `uiharu/` except `serde`, `serde_json` and `thiserror`.

## Protocols

Each decoder is a module of `crates/uiharu-protocols` with a page in `docs/protocols/` that spells out its contract (the event kinds and their fields), its configuration and its rules. Every decoder was written from the protocol's public specification.

| Decoder                | Type in pipeline files | Reads              | Page                                          |
| ---------------------- | ---------------------- | ------------------ | --------------------------------------------- |
| SPI                    | `spi`                  | logic samples      | [spi.md](docs/protocols/spi.md)               |
| SPI NOR flash          | `spi-nor`              | `spi.events`       | [spi-nor.md](docs/protocols/spi-nor.md)       |
| SD card, SPI mode      | `sdcard-spi`           | `spi.events`       | [sdcard-spi.md](docs/protocols/sdcard-spi.md) |
| APA102 / SK9822 LEDs   | `apa102`               | `spi.events`       | [apa102.md](docs/protocols/apa102.md)         |
| AVR ISP                | `avr-isp`              | `spi.events`       | [avr-isp.md](docs/protocols/avr-isp.md)       |
| nRF24L01+              | `nrf24l01`             | `spi.events`       | [nrf24l01.md](docs/protocols/nrf24l01.md)     |
| Microwire              | `microwire`            | logic samples      | [microwire.md](docs/protocols/microwire.md)   |
| 93Cxx EEPROM           | `eeprom93xx`           | `microwire.events` | [microwire.md](docs/protocols/microwire.md)   |
| I²C                    | `i2c`                  | logic samples      | [i2c.md](docs/protocols/i2c.md)               |
| 24xx EEPROM            | `eeprom24xx`           | `i2c.events`       | [eeprom24xx.md](docs/protocols/eeprom24xx.md) |
| UART                   | `uart`                 | logic samples      | [uart.md](docs/protocols/uart.md)             |
| Modbus RTU             | `modbus-rtu`           | `uart.events`      | [modbus-rtu.md](docs/protocols/modbus-rtu.md) |
| LIN                    | `lin`                  | `uart.events`      | [lin.md](docs/protocols/lin.md)               |
| MIDI                   | `midi`                 | `uart.events`      | [midi.md](docs/protocols/midi.md)             |
| DMX512                 | `dmx512`               | `uart.events`      | [dmx512.md](docs/protocols/dmx512.md)         |
| CAN 2.0 and CAN FD     | `can`                  | logic samples      | [can.md](docs/protocols/can.md)               |
| 1-Wire link layer      | `onewire-link`         | logic samples      | [onewire.md](docs/protocols/onewire.md)       |
| 1-Wire network         | `onewire`              | `onewire.link`     | [onewire.md](docs/protocols/onewire.md)       |
| DS18B20                | `ds18b20`              | `onewire.events`   | [ds18b20.md](docs/protocols/ds18b20.md)       |
| I²S                    | `i2s`                  | logic samples      | [i2s.md](docs/protocols/i2s.md)               |
| PWM                    | `pwm`                  | logic samples      | [pwm.md](docs/protocols/pwm.md)               |
| Edge timing            | `timing`               | logic samples      | [timing.md](docs/protocols/timing.md)         |
| Parallel bus           | `parallel`             | logic samples      | [parallel.md](docs/protocols/parallel.md)     |
| WS281x LEDs            | `ws281x`               | logic samples      | [ws281x.md](docs/protocols/ws281x.md)         |
| USB low/full speed     | `usb`                  | logic samples      | [usb.md](docs/protocols/usb.md)               |
| USB transactions       | `usb-transactions`     | `usb.packets`      | [usb.md](docs/protocols/usb.md)               |
| USB Power Delivery     | `usb-pd`               | logic samples      | [usb-pd.md](docs/protocols/usb-pd.md)         |
| JTAG                   | `jtag`                 | logic samples      | [jtag.md](docs/protocols/jtag.md)             |
| SWD                    | `swd`                  | logic samples      | [swd.md](docs/protocols/swd.md)               |
| MDIO                   | `mdio`                 | logic samples      | [mdio.md](docs/protocols/mdio.md)             |
| NEC infrared           | `ir-nec`               | logic samples      | [ir-nec.md](docs/protocols/ir-nec.md)         |
| RC-5 infrared          | `ir-rc5`               | logic samples      | [ir-rc5.md](docs/protocols/ir-rc5.md)         |
| RC-6 infrared          | `ir-rc6`               | logic samples      | [ir-rc6.md](docs/protocols/ir-rc6.md)         |
| Sony SIRC infrared     | `ir-sirc`              | logic samples      | [ir-sirc.md](docs/protocols/ir-sirc.md)       |
| PS/2                   | `ps2`                  | logic samples      | [ps2.md](docs/protocols/ps2.md)               |
| Wiegand                | `wiegand`              | logic samples      | [wiegand.md](docs/protocols/wiegand.md)       |
| AM2302 / DHT22 / DHT11 | `am230x`               | logic samples      | [am230x.md](docs/protocols/am230x.md)         |

## Quick start

```sh
cargo build -p uiharu-cli
target/debug/uiharu gen flash.bin --reads 16 --bytes 64     # a synthetic SPI NOR capture
target/debug/uiharu decode flash.bin > events.ndjson        # SPI and SPI NOR events
target/debug/uiharu decode flash.bin --stream nor --chunk 4096 --stats
target/debug/uiharu bench --samples 50000000                # throughput, latency, memory
```

`decode` reads raw binary captures: one sample after the other, ⌈channels / 8⌉ bytes each, bit _i_ of byte _j_ being channel 8 _j_ + _i_ — the layout of HSync's binary export. The SPI bus is described with `--clk`, `--cs`, `--mosi`, `--miso` (`none` for a line that is not connected), `--mode`, `--word-bits`, `--lsb-first` and `--cs-active-high`; `uiharu decode --help` lists everything.

Any pipeline can be described in a JSON file instead — the source, the nodes with their type and configuration, the connections between ports named as the nodes declare them, and the outputs to write — and run with `uiharu decode --graph flash.json capture.bin`:

```json
{
  "source": { "channels": 4, "rate": 25000000 },
  "nodes": {
    "spi": { "type": "spi", "config": { "clk": 0, "cs": 1, "mosi": 2, "miso": 3, "mode": 0 } },
    "flash": { "type": "spi-nor" }
  },
  "connections": [
    { "from": "source", "to": "spi.logic" },
    { "from": "spi.events", "to": "flash.bus" }
  ],
  "expose": { "spi": "spi.events", "nor": "flash.events" }
}
```

A configuration may leave fields out (they take the node's defaults); a connection the decoders cannot accept is refused with the graph's diagnostic, such as `flash#1.bus requires wordBits = 8 of spi#0.events, which declares 4`. The node types are listed in `crates/uiharu-registry`, with a manifest each (`manifests/<type>.json`: ports, configuration fields, event kinds); each protocol page in `docs/protocols/` names its type, ports and configuration.

Every event is one line of JSON: the stream it belongs to, its number in that stream, the ticks it covers, the tick at which it became certain, and the event itself as the stream's contract spells it. Gaps and the end of each stream are lines too.

```json
{"stream":"spi","seq":1,"span":[48,105],"settled":104,"event":{"kind":"word","tx":1,"mosi":3,"miso":255}}
{"stream":"nor","seq":0,"span":[40,105],"settled":104,"event":{"kind":"begin","op":1,"tx":1,"command":"read"}}
{"stream":"nor","seq":1,"span":[112,297],"settled":296,"event":{"kind":"address","op":1,"address":14069354}}
{"stream":"nor","seq":2,"span":[304,2345],"settled":2344,"event":{"kind":"data","op":1,"offset":0,"bytes":[3,126,243]}}
{"stream":"nor","seq":3,"span":[2352,2353],"settled":2352,"event":{"kind":"end","op":1,"reason":"complete"}}
{"stream":"nor","gap":[5000,5200]}
{"stream":"nor","end":14664}
```

The lines of one stream are the same whatever the block size, queue size or budget; lines of different streams interleave in the order the decoders produced them, which does depend on the blocks, so compare streams one at a time (`--stream`).

## Embedding

```rust
use std::sync::Arc;
use uiharu_core::{Graph, SignalItem, SourceSpec, TimeBase};
use uiharu_protocols::spi::{Spi, SpiConfig};
use uiharu_protocols::spi_nor::{SpiNor, SpiNorConfig};

let mut graph = Graph::new();
let logic = graph.source(SourceSpec::new("logic", 4, TimeBase::new(25_000_000)))?;
let spi = graph.add(Spi::new(SpiConfig::default())?);
let flash = graph.add(SpiNor::new(SpiNorConfig::default()));
graph.connect(logic.samples, Spi::input(spi))?;            // checks the channel count
graph.connect(Spi::events(spi), SpiNor::input(flash))?;    // checks word width, bit order, lines
let operations = graph.expose("flash", SpiNor::events(flash))?;
let mut pipeline = graph.build()?;                          // checks for cycles and unconnected inputs
let operations = pipeline.exposed(operations).expect("exposed once");

// As blocks of samples arrive (a `LogicBlock`, bit-packed per channel):
pipeline.push(logic.id, SignalItem::Data(Arc::new(block)))?; // `Err(Full)` hands the block back
pipeline.run();                                              // until every decoder waits
while let Some(item) = operations.next() { /* events, gaps, progress, the end */ }
```

A source that cannot hold its samples back announces what it lost with `SignalItem::Gap`; the engine never drops samples itself. `pipeline.stats()` reports every queue's size and high-water mark.

## License

uiharu is licensed under the [Apache License 2.0](LICENSE). It is written from scratch: nothing in `uiharu/` is ported from DSView, libsigrok or libsigrokdecode, and it depends on none of HSync's GPL-licensed parts. Every source file starts with `// SPDX-License-Identifier: Apache-2.0`.
