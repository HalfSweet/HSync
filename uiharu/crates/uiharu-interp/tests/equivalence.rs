// SPDX-License-Identifier: Apache-2.0

//! The interpreted reference decoders emit what the decoders generated from the same IR emit:
//! on random waveforms, with gaps, item for item — events, their provenance, gaps, progress
//! and the end — and whatever the blocks, the pauses and the queues.

mod common;

use common::{as_dyn, program};
use proptest::prelude::*;
use uiharu_core::testing::{Harness, assert_block_invariant};
use uiharu_core::{
    GraphConfig, InputRef, Item, Levels, LogicBlock, LogicBlockBuilder, Node, OutputRef, Tick,
    TickRange, TimeBase,
};
use uiharu_interp::{DynEvent, FromDyn, Interpreter, ToDyn};
use uiharu_protocols::pwm::{Polarity, Pwm, PwmConfig, PwmEvent};
use uiharu_protocols::spi::synth::{SpiWaveform, nor_read_words};
use uiharu_protocols::spi::{BitOrder, Spi, SpiConfig, SpiEvent};
use uiharu_protocols::spi_nor::{AddressBytes, NorEvent, SpiNor, SpiNorConfig, synth};
use uiharu_protocols::uart::synth::{Faults, UartWaveform};
use uiharu_protocols::uart::{
    BitOrder as UartBitOrder, Parity, StopBits, Uart, UartConfig, UartEvent,
};

const RATE: u64 = 1_000_000;
const CHANNELS: u8 = 4;

fn json<T: serde::Serialize>(config: &T) -> serde_json::Value {
    serde_json::to_value(config).expect("serializes")
}

/// An interpreted logic decoder of `name`, configured as the compiled decoder's `config`.
fn interpreted_logic(
    name: &str,
    config: serde_json::Value,
    graph: GraphConfig,
) -> Harness<DynEvent> {
    Harness::build(CHANNELS, TimeBase::new(RATE), graph, |g, source| {
        let node = g.add(Interpreter::new(program(name), &config).expect("configures"));
        g.connect(source.samples, Interpreter::logic_input(node))?;
        Ok(Interpreter::events(node))
    })
    .expect("builds")
}

fn compiled_pwm(config: PwmConfig, graph: GraphConfig) -> Harness<PwmEvent> {
    Harness::build(CHANNELS, TimeBase::new(RATE), graph, |g, source| {
        let node = g.add(Pwm::new(config).expect("configures"));
        g.connect(source.samples, Pwm::input(node))?;
        Ok(Pwm::events(node))
    })
    .expect("builds")
}

fn compiled_spi(config: SpiConfig, graph: GraphConfig) -> Harness<SpiEvent> {
    Harness::build(CHANNELS, TimeBase::new(RATE), graph, |g, source| {
        let node = g.add(Spi::new(config).expect("configures"));
        g.connect(source.samples, Spi::input(node))?;
        Ok(Spi::events(node))
    })
    .expect("builds")
}

fn compiled_uart(config: UartConfig, graph: GraphConfig) -> Harness<UartEvent> {
    Harness::build(CHANNELS, TimeBase::new(RATE), graph, |g, source| {
        let node = g.add(Uart::new(config).expect("configures"));
        g.connect(source.samples, Uart::input(node))?;
        Ok(Uart::events(node))
    })
    .expect("builds")
}

/// Which decoders of the SPI → SPI NOR pipeline are interpreted.
#[derive(Clone, Copy, Debug)]
struct Mix {
    spi: bool,
    nor: bool,
}

const MIXES: [Mix; 3] = [
    Mix {
        spi: false,
        nor: true,
    },
    Mix {
        spi: true,
        nor: false,
    },
    Mix {
        spi: true,
        nor: true,
    },
];

/// The NOR events of the pipeline `mix` describes, as an interpreted decoder would emit them.
fn nor_pipeline(
    mix: Mix,
    spi: SpiConfig,
    nor: SpiNorConfig,
    graph: GraphConfig,
) -> Harness<DynEvent> {
    Harness::build(CHANNELS, TimeBase::new(RATE), graph, |g, source| {
        // The SPI stream as `DynEvent`s, bridged from the compiled decoder when need be.
        let props = Spi::new(spi).expect("configures").spec().outputs[0]
            .props
            .clone();
        let spi_out: OutputRef<DynEvent> = if mix.spi {
            let node = g.add(Interpreter::new(program("spi"), &json(&spi)).expect("configures"));
            g.connect(source.samples, Interpreter::logic_input(node))?;
            Interpreter::events(node)
        } else {
            let node = g.add(Spi::new(spi).expect("configures"));
            g.connect(source.samples, Spi::input(node))?;
            let bridge = g.add(ToDyn::<SpiEvent>::new(props.clone()));
            g.connect(Spi::events(node), InputRef::<SpiEvent>::of_node(bridge, 0))?;
            OutputRef::of_node(bridge, 0)
        };
        if mix.nor {
            let node =
                g.add(Interpreter::new(program("spi-nor"), &json(&nor)).expect("configures"));
            g.connect(spi_out, Interpreter::bus_input(node))?;
            Ok(Interpreter::events(node))
        } else {
            let bridge = g.add(FromDyn::<SpiEvent>::new(props));
            g.connect(spi_out, InputRef::<DynEvent>::of_node(bridge, 0))?;
            let node = g.add(SpiNor::new(nor).expect("configures"));
            g.connect(
                OutputRef::<SpiEvent>::of_node(bridge, 0),
                SpiNor::input(node),
            )?;
            let bridge = g.add(ToDyn::<NorEvent>::new(
                SpiNor::new(nor).expect("configures").spec().outputs[0]
                    .props
                    .clone(),
            ));
            g.connect(
                SpiNor::events(node),
                InputRef::<NorEvent>::of_node(bridge, 0),
            )?;
            Ok(OutputRef::of_node(bridge, 0))
        }
    })
    .expect("builds")
}

fn compiled_nor(spi: SpiConfig, nor: SpiNorConfig, graph: GraphConfig) -> Harness<NorEvent> {
    Harness::build(CHANNELS, TimeBase::new(RATE), graph, |g, source| {
        let spi_node = g.add(Spi::new(spi).expect("configures"));
        g.connect(source.samples, Spi::input(spi_node))?;
        let nor_node = g.add(SpiNor::new(nor).expect("configures"));
        g.connect(Spi::events(spi_node), SpiNor::input(nor_node))?;
        Ok(SpiNor::events(nor_node))
    })
    .expect("builds")
}

/// Runs `block` less `gaps` through `harness` in one piece and takes everything it emitted.
fn run<T: Clone + 'static>(
    mut harness: Harness<T>,
    block: &LogicBlock,
    gaps: &[TickRange],
) -> Vec<Item<T>> {
    harness.feed_with_gaps(block, gaps, &[u64::MAX]);
    harness.end();
    harness.items
}

/// A gap inside `block`, from the fractions `at` and `len` of its length.
fn gap_in(block: &LogicBlock, gap: Option<(f64, u64)>) -> Vec<TickRange> {
    gap.and_then(|(at, len)| {
        let start = ((block.len() as f64) * at) as u64;
        let end = (start + len).min(block.len());
        (start < end).then(|| TickRange::new(Tick(start), Tick(end)))
    })
    .into_iter()
    .collect()
}

// PWM

/// A pulse train on channel 1 with channel 0 busy too, from `seed`.
fn pulses(seed: u64, cycles: u64) -> LogicBlock {
    let mut builder = LogicBlockBuilder::new(Tick::ZERO, CHANNELS);
    for i in 0..cycles {
        let period = (i * 53 + seed) % 40 + 2;
        let high = (i * 29 + seed * 7 + 3) % period;
        let noise = i & 1;
        builder.push_repeated(Levels(0b10 | noise), high);
        builder.push_repeated(Levels(noise ^ 1), period - high);
    }
    builder.finish().expect("a block")
}

fn pwm_config(polarity: Polarity) -> PwmConfig {
    PwmConfig {
        channel: 1,
        polarity,
    }
}

// SPI

#[derive(Clone, Debug)]
enum Tx {
    Read { address: u32, len: usize },
    FastRead { address: u32, len: usize },
    Program { address: u32, len: usize },
    Status { opcode: u8, reads: usize },
    JedecId { len: usize },
    Mode { enter: bool },
    Other { opcode: u8, extra: u8 },
    Short { address_bytes: u8 },
    Partial { mosi: u8, miso: u8, bits: u8 },
    Empty,
}

fn tx() -> impl Strategy<Value = Tx> {
    prop_oneof![
        5 => (0u32..0x100_0000, 0usize..70).prop_map(|(address, len)| Tx::Read { address, len }),
        2 => (0u32..0x100_0000, 0usize..40).prop_map(|(address, len)| Tx::FastRead { address, len }),
        2 => (0u32..0x100_0000, 0usize..40).prop_map(|(address, len)| Tx::Program { address, len }),
        1 => (prop::sample::select(vec![0x05u8, 0x35, 0x15, 0x01]), 0usize..5)
            .prop_map(|(opcode, reads)| Tx::Status { opcode, reads }),
        1 => (0usize..40).prop_map(|len| Tx::JedecId { len }),
        1 => any::<bool>().prop_map(|enter| Tx::Mode { enter }),
        2 => (any::<u8>(), 0u8..4).prop_map(|(opcode, extra)| Tx::Other { opcode, extra }),
        1 => (0u8..3).prop_map(|address_bytes| Tx::Short { address_bytes }),
        1 => (any::<u8>(), any::<u8>(), 1u8..8).prop_map(|(mosi, miso, bits)| Tx::Partial { mosi, miso, bits }),
        1 => Just(Tx::Empty),
    ]
}

fn spi_waveform(config: SpiConfig, txs: &[Tx], half: u64) -> LogicBlock {
    let mut waveform = SpiWaveform::new(config, CHANNELS, half, Tick::ZERO);
    waveform.idle(7);
    for (n, tx) in txs.iter().enumerate() {
        let bytes = |len: usize| -> Vec<u8> { (0..len).map(|i| (i * 31 + n * 7) as u8).collect() };
        match tx {
            Tx::Read { address, len } => {
                waveform.transaction(nor_read_words(*address, &bytes(*len)))
            }
            Tx::FastRead { address, len } => {
                waveform.transaction(synth::read(0x0b, *address, 3, 1, &bytes(*len)));
            }
            Tx::Program { address, len } => {
                waveform.transaction(synth::program(0x02, *address, 3, &bytes(*len)));
            }
            Tx::Status { opcode, reads } => {
                waveform.transaction(synth::read_status(*opcode, &bytes(*reads)));
            }
            Tx::JedecId { len } => waveform.transaction(synth::read_id(0x9f, 0, &bytes(*len))),
            Tx::Mode { enter } => {
                waveform.transaction(synth::command(if *enter { 0xb7 } else { 0xe9 }));
            }
            Tx::Other { opcode, extra } => {
                let words = std::iter::once((*opcode, 0xff))
                    .chain((0..*extra).map(|i| (i, i.wrapping_mul(3))));
                waveform.transaction(words);
            }
            Tx::Short { address_bytes } => {
                waveform.transaction(
                    nor_read_words(0xabcdef, &[])
                        .into_iter()
                        .take(1 + usize::from(*address_bytes)),
                );
            }
            Tx::Partial { mosi, miso, bits } => {
                waveform.select();
                waveform.word(0x03, 0xff);
                waveform.partial_word(*mosi, *miso, *bits);
                waveform.release();
            }
            Tx::Empty => waveform.transaction([]),
        }
        waveform.idle(3 + (n as u64 % 5) * 4);
    }
    waveform.finish()
}

fn spi_config() -> impl Strategy<Value = SpiConfig> {
    (0u8..4, 1u8..9, any::<bool>(), any::<bool>(), any::<bool>()).prop_map(
        |(mode, word_bits, lsb_first, no_cs, cs_active_high)| SpiConfig {
            mode,
            word_bits,
            bit_order: if lsb_first {
                BitOrder::LsbFirst
            } else {
                BitOrder::MsbFirst
            },
            cs: if no_cs { None } else { Some(1) },
            cs_active_low: !cs_active_high,
            ..SpiConfig::default()
        },
    )
}

// UART

#[derive(Clone, Debug)]
enum Sent {
    Character { value: u16, faults: Faults },
    Idle(u64),
    Break(u64),
    Glitch(u64),
}

fn sent() -> impl Strategy<Value = Sent> {
    prop_oneof![
        8 => (any::<u16>(), any::<bool>(), prop::bool::weighted(0.1))
            .prop_map(|(value, parity, framing)| Sent::Character { value, faults: Faults { parity: parity && framing, framing } }),
        4 => (any::<u16>()).prop_map(|value| Sent::Character { value, faults: Faults::NONE }),
        1 => (any::<u16>()).prop_map(|value| Sent::Character { value, faults: Faults::PARITY }),
        3 => (0u64..300).prop_map(Sent::Idle),
        1 => (50u64..2000).prop_map(Sent::Break),
        1 => (1u64..6).prop_map(Sent::Glitch),
    ]
}

fn uart_config() -> impl Strategy<Value = UartConfig> {
    (
        5u8..10,
        0u8..5,
        0u8..3,
        any::<bool>(),
        any::<bool>(),
        1u8..100,
        9_600u32..200_000,
    )
        .prop_map(
            |(data_bits, parity, stop_bits, msb_first, inverted, sample_point, baud)| UartConfig {
                line: 1,
                baud,
                data_bits,
                parity: [
                    Parity::None,
                    Parity::Even,
                    Parity::Odd,
                    Parity::Mark,
                    Parity::Space,
                ][usize::from(parity)],
                stop_bits: [StopBits::One, StopBits::OneAndAHalf, StopBits::Two]
                    [usize::from(stop_bits)],
                bit_order: if msb_first {
                    UartBitOrder::MsbFirst
                } else {
                    UartBitOrder::LsbFirst
                },
                inverted,
                sample_point,
            },
        )
}

fn uart_waveform(config: UartConfig, sent: &[Sent]) -> LogicBlock {
    let mut waveform = UartWaveform::at_rate(config, CHANNELS, RATE, Tick::ZERO);
    waveform.idle(20);
    for item in sent {
        match item {
            Sent::Character { value, faults } => {
                waveform.frame(*value, *faults);
                if faults.framing {
                    waveform.idle(30);
                }
            }
            Sent::Idle(samples) => waveform.idle(*samples),
            Sent::Break(samples) => {
                waveform.break_(*samples);
                waveform.idle(40);
            }
            Sent::Glitch(samples) => {
                waveform.glitch(*samples);
                waveform.idle(40);
            }
        }
    }
    waveform.idle(50);
    waveform.finish()
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: std::env::var("PROPTEST_CASES").ok().and_then(|cases| cases.parse().ok()).unwrap_or(48),
        ..ProptestConfig::default()
    })]

    #[test]
    fn pwm_decodes_as_generated(
        seed in 0u64..1000,
        cycles in 1u64..120,
        active_low in any::<bool>(),
        gap in prop::option::of((0.05f64..0.95, 1u64..200)),
    ) {
        let block = pulses(seed, cycles);
        let gaps = gap_in(&block, gap);
        let config = pwm_config(if active_low { Polarity::ActiveLow } else { Polarity::ActiveHigh });
        let compiled = run(compiled_pwm(config, GraphConfig::default()), &block, &gaps);
        let interpreted = run(interpreted_logic("pwm", json(&config), GraphConfig::default()), &block, &gaps);
        prop_assert_eq!(interpreted, as_dyn(&compiled));
    }

    #[test]
    fn spi_decodes_as_generated(
        config in spi_config(),
        txs in prop::collection::vec(tx(), 1..8),
        half in 1u64..4,
        gap in prop::option::of((0.05f64..0.95, 1u64..200)),
    ) {
        let block = spi_waveform(config, &txs, half);
        let gaps = gap_in(&block, gap);
        let compiled = run(compiled_spi(config, GraphConfig::default()), &block, &gaps);
        let interpreted = run(interpreted_logic("spi", json(&config), GraphConfig::default()), &block, &gaps);
        prop_assert_eq!(interpreted, as_dyn(&compiled));
    }

    #[test]
    fn uart_decodes_as_generated(
        config in uart_config(),
        sent in prop::collection::vec(sent(), 1..12),
        gap in prop::option::of((0.05f64..0.95, 1u64..200)),
    ) {
        let block = uart_waveform(config, &sent);
        let gaps = gap_in(&block, gap);
        let compiled = run(compiled_uart(config, GraphConfig::default()), &block, &gaps);
        let interpreted = run(interpreted_logic("uart", json(&config), GraphConfig::default()), &block, &gaps);
        prop_assert_eq!(interpreted, as_dyn(&compiled));
    }

    #[test]
    fn spi_nor_decodes_as_generated_whichever_side_is_interpreted(
        txs in prop::collection::vec(tx(), 1..8),
        mode in 0u8..4,
        half in 1u64..3,
        auto in any::<bool>(),
        gap in prop::option::of((0.05f64..0.95, 1u64..200)),
    ) {
        let spi = SpiConfig { mode, ..SpiConfig::default() };
        let nor = SpiNorConfig {
            address_bytes: if auto { AddressBytes::Auto } else { AddressBytes::Three },
        };
        let block = spi_waveform(spi, &txs, half);
        let gaps = gap_in(&block, gap);
        let compiled = as_dyn(&run(compiled_nor(spi, nor, GraphConfig::default()), &block, &gaps));
        for mix in MIXES {
            let interpreted = run(nor_pipeline(mix, spi, nor, GraphConfig::default()), &block, &gaps);
            prop_assert_eq!(&interpreted, &compiled, "{:?}", mix);
        }
    }
}

#[test]
fn interpreted_decoders_keep_the_block_invariant() {
    let pwm = pwm_config(Polarity::ActiveHigh);
    let block = pulses(11, 200);
    let gaps = [
        TickRange::new(Tick(127), Tick(128)),
        TickRange::new(Tick(1000), Tick(1090)),
        TickRange::new(Tick(2333), Tick(2500)),
    ];
    assert_block_invariant(
        |graph| interpreted_logic("pwm", json(&pwm), graph),
        &block,
        &gaps,
    );

    let spi = SpiConfig {
        mode: 3,
        ..SpiConfig::default()
    };
    let txs = [
        Tx::Read {
            address: 0x123456,
            len: 40,
        },
        Tx::Status {
            opcode: 0x05,
            reads: 2,
        },
        Tx::Program {
            address: 0x10,
            len: 20,
        },
        Tx::Partial {
            mosi: 0x5a,
            miso: 0xa5,
            bits: 3,
        },
        Tx::JedecId { len: 5 },
        Tx::Other {
            opcode: 0x42,
            extra: 2,
        },
        Tx::Mode { enter: true },
        Tx::Read {
            address: 0x1,
            len: 9,
        },
    ];
    let block = spi_waveform(spi, &txs, 2);
    let gaps = [
        TickRange::new(Tick(300), Tick(330)),
        TickRange::new(Tick(2000), Tick(2200)),
    ];
    assert_block_invariant(
        |graph| interpreted_logic("spi", json(&spi), graph),
        &block,
        &gaps,
    );
    let nor = SpiNorConfig {
        address_bytes: AddressBytes::Auto,
    };
    for mix in MIXES {
        assert_block_invariant(|graph| nor_pipeline(mix, spi, nor, graph), &block, &gaps);
    }

    let uart = UartConfig {
        line: 1,
        baud: 57_600,
        data_bits: 7,
        parity: Parity::Odd,
        stop_bits: StopBits::Two,
        ..UartConfig::default()
    };
    let sent = [
        Sent::Character {
            value: 0x55,
            faults: Faults::NONE,
        },
        Sent::Idle(100),
        Sent::Character {
            value: 0x7f,
            faults: Faults::PARITY,
        },
        Sent::Break(500),
        Sent::Character {
            value: 0x12,
            faults: Faults::FRAMING,
        },
        Sent::Idle(30),
        Sent::Glitch(3),
        Sent::Character {
            value: 0x00,
            faults: Faults::NONE,
        },
    ];
    let block = uart_waveform(uart, &sent);
    let gaps = [TickRange::new(Tick(700), Tick(760))];
    assert_block_invariant(
        |graph| interpreted_logic("uart", json(&uart), graph),
        &block,
        &gaps,
    );
}

#[test]
fn the_bridges_keep_the_items_of_a_stream() {
    let spi = SpiConfig::default();
    let block = spi_waveform(spi, &[Tx::Read { address: 7, len: 3 }, Tx::Empty], 2);
    let gaps = [TickRange::new(Tick(40), Tick(50))];
    let compiled = run(compiled_spi(spi, GraphConfig::default()), &block, &gaps);
    let bridged: Harness<SpiEvent> = Harness::build(
        CHANNELS,
        TimeBase::new(RATE),
        GraphConfig::default(),
        |g, source| {
            let node = g.add(Spi::new(spi).expect("configures"));
            g.connect(source.samples, Spi::input(node))?;
            let props = Spi::new(spi).expect("configures").spec().outputs[0]
                .props
                .clone();
            let to = g.add(ToDyn::<SpiEvent>::new(props.clone()));
            g.connect(Spi::events(node), InputRef::<SpiEvent>::of_node(to, 0))?;
            let from = g.add(FromDyn::<SpiEvent>::new(props));
            g.connect(
                OutputRef::<DynEvent>::of_node(to, 0),
                InputRef::<DynEvent>::of_node(from, 0),
            )?;
            Ok(OutputRef::of_node(from, 0))
        },
    )
    .expect("builds");
    let bridged = run(bridged, &block, &gaps);
    assert_eq!(bridged, compiled);
}
