// SPDX-License-Identifier: Apache-2.0

//! The generated decoders against the hand-written ones they were written from: on random
//! waveforms, cut into random blocks with random gaps, both produce the same events — same
//! sequence numbers, spans, settled ticks and payloads — and the same gaps and end. Progress
//! is left out, as it depends on the blocks. The configurations cross over as JSON, so that
//! the generated configuration reads what the hand-written one writes.

use std::cell::RefCell;

use proptest::prelude::*;
use serde::Serialize;
use uiharu_core::testing::Harness;
use uiharu_core::{
    EventSchema, Graph, GraphConfig, GraphError, Item, LogicBlock, OutputRef, SourceHandle, Tick,
    TickRange, TimeBase,
};
use uiharu_dsl_protocols as generated;
use uiharu_protocols as hand;
use uiharu_protocols::spi::synth::SpiWaveform;
use uiharu_protocols::spi_nor::synth as nor;
use uiharu_protocols::uart::synth::{Faults, UartWaveform};

/// One line per item but progress.
fn lines<T: Serialize>(items: &[Item<T>]) -> Vec<String> {
    items
        .iter()
        .filter_map(|item| match item {
            Item::Event(event) => Some(format!(
                "{} {} @{} {}",
                event.seq,
                event.span,
                event.settled,
                serde_json::to_string(&event.payload).expect("serializes")
            )),
            Item::Gap(range) => Some(format!("gap {range}")),
            Item::Progress(_) => None,
            Item::End(tick) => Some(format!("end {tick}")),
        })
        .collect()
}

/// A configuration crossed over as JSON.
fn convert<A: Serialize, B: serde::de::DeserializeOwned>(config: &A) -> B {
    let json = serde_json::to_value(config).expect("serializes");
    serde_json::from_value(json).expect("the generated configuration reads it")
}

/// Runs `block` through the pipeline `wire` builds on `channels` channels at `rate`, cut into
/// blocks of `sizes` with `gaps` left out.
fn run<T, F>(
    channels: u8,
    rate: u64,
    block: &LogicBlock,
    gaps: &[TickRange],
    sizes: &[u64],
    wire: F,
) -> Vec<String>
where
    T: EventSchema + Clone + Serialize + 'static,
    F: FnOnce(&mut Graph, SourceHandle) -> Result<OutputRef<T>, GraphError>,
{
    let mut harness = Harness::build(channels, TimeBase::new(rate), GraphConfig::default(), wire)
        .expect("builds");
    harness.feed_with_gaps(block, gaps, sizes);
    harness.end();
    lines(&harness.items)
}

fn sizes() -> impl Strategy<Value = Vec<u64>> {
    prop::collection::vec(1..3000u64, 1..4)
}

// PWM

fn pwm_config() -> impl Strategy<Value = hand::pwm::PwmConfig> {
    (0..2u8, any::<bool>()).prop_map(|(channel, low)| hand::pwm::PwmConfig {
        channel,
        polarity: if low {
            hand::pwm::Polarity::ActiveLow
        } else {
            hand::pwm::Polarity::ActiveHigh
        },
    })
}

proptest! {
    #![proptest_config(ProptestConfig::default())]

    #[test]
    fn pwm_matches(
        config in pwm_config(),
        lead in 0..30u64,
        start_active in any::<bool>(),
        cycles in prop::collection::vec((2..120u64, 1..119u64), 0..12),
        tail in 0..40u64,
        cuts in sizes(),
        gap_seed in prop::collection::vec((0..2000u64, 1..50u64), 0..3),
    ) {
        let mut waveform = hand::pwm::synth::PwmWaveform::new(&config, 2, Tick::ZERO);
        waveform.level(start_active, lead + 1);
        waveform.cycles(cycles.into_iter().map(|(period, active)| (period, active.min(period - 1))));
        waveform.level(!start_active, tail + 1);
        let block = waveform.finish();
        let len = block.len();
        let mut gaps: Vec<TickRange> = gap_seed
            .into_iter()
            .map(|(start, width)| TickRange::new(Tick(start % len.max(1)), Tick((start % len.max(1)) + width)))
            .collect();
        gaps.sort_by_key(|gap| gap.start);
        gaps.dedup_by(|b, a| b.start < a.end);
        let expected = run(2, 1_000_000, &block, &gaps, &cuts, |graph, source| {
            let node = graph.add(hand::pwm::Pwm::new(config.clone()).expect("valid"));
            graph.connect(source.samples, hand::pwm::Pwm::input(node))?;
            Ok(hand::pwm::Pwm::events(node))
        });
        let generated_config: generated::pwm::PwmConfig = convert(&config);
        let actual = run(2, 1_000_000, &block, &gaps, &cuts, |graph, source| {
            let node = graph.add(generated::pwm::Pwm::new(generated_config).expect("valid"));
            graph.connect(source.samples, generated::pwm::Pwm::input(node))?;
            Ok(generated::pwm::Pwm::events(node))
        });
        prop_assert_eq!(actual, expected);
    }
}

// UART

fn uart_config() -> impl Strategy<Value = hand::uart::UartConfig> {
    (
        5..=9u8,
        0..5u8,
        0..3u8,
        any::<bool>(),
        any::<bool>(),
        5..95u8,
    )
        .prop_map(
            |(data_bits, parity, stop_bits, msb_first, inverted, sample_point)| {
                hand::uart::UartConfig {
                    line: 0,
                    baud: 9_600,
                    data_bits,
                    parity: match parity {
                        0 => hand::uart::Parity::None,
                        1 => hand::uart::Parity::Even,
                        2 => hand::uart::Parity::Odd,
                        3 => hand::uart::Parity::Mark,
                        _ => hand::uart::Parity::Space,
                    },
                    stop_bits: match stop_bits {
                        0 => hand::uart::StopBits::One,
                        1 => hand::uart::StopBits::OneAndAHalf,
                        _ => hand::uart::StopBits::Two,
                    },
                    bit_order: if msb_first {
                        hand::uart::BitOrder::MsbFirst
                    } else {
                        hand::uart::BitOrder::LsbFirst
                    },
                    inverted,
                    sample_point,
                }
            },
        )
}

#[derive(Clone, Debug)]
enum UartPiece {
    Byte(u16, bool, bool),
    Break(u64),
    Glitch(u64),
    Idle(u64),
}

fn uart_piece() -> impl Strategy<Value = UartPiece> {
    prop_oneof![
        6 => (0..512u16, any::<bool>(), any::<bool>()).prop_map(|(value, parity, framing)| UartPiece::Byte(value, parity, framing)),
        1 => (1..400u64).prop_map(UartPiece::Break),
        1 => (1..6u64).prop_map(UartPiece::Glitch),
        2 => (0..200u64).prop_map(UartPiece::Idle),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::default())]

    #[test]
    fn uart_matches(
        config in uart_config(),
        bit_samples in 2..24u64,
        pieces in prop::collection::vec(uart_piece(), 0..10),
        cuts in sizes(),
        gap_seed in prop::collection::vec((0..4000u64, 1..60u64), 0..3),
    ) {
        let rate = u64::from(config.baud) * bit_samples;
        let mut waveform = UartWaveform::new(config, 1, bit_samples, Tick::ZERO);
        waveform.idle(3);
        for piece in pieces {
            match piece {
                UartPiece::Byte(value, parity, framing) => {
                    let mask = (1u16 << config.data_bits) - 1;
                    waveform.frame(value & mask, Faults { parity, framing });
                    if framing {
                        waveform.idle(bit_samples * 2);
                    }
                }
                UartPiece::Break(samples) => {
                    waveform.break_(samples);
                    waveform.idle(bit_samples);
                }
                UartPiece::Glitch(samples) => {
                    waveform.glitch(samples.min(bit_samples.saturating_sub(1)).max(1));
                    waveform.idle(bit_samples);
                }
                UartPiece::Idle(samples) => waveform.idle(samples),
            }
        }
        waveform.idle(bit_samples * 4);
        let block = waveform.finish();
        let len = block.len();
        let mut gaps: Vec<TickRange> = gap_seed
            .into_iter()
            .map(|(start, width)| TickRange::new(Tick(start % len.max(1)), Tick((start % len.max(1)) + width)))
            .collect();
        gaps.sort_by_key(|gap| gap.start);
        gaps.dedup_by(|b, a| b.start < a.end);
        let expected = run(1, rate, &block, &gaps, &cuts, |graph, source| {
            let node = graph.add(hand::uart::Uart::new(config).expect("valid"));
            graph.connect(source.samples, hand::uart::Uart::input(node))?;
            Ok(hand::uart::Uart::events(node))
        });
        let generated_config: generated::uart::UartConfig = convert(&config);
        let actual = run(1, rate, &block, &gaps, &cuts, |graph, source| {
            let node = graph.add(generated::uart::Uart::new(generated_config).expect("valid"));
            graph.connect(source.samples, generated::uart::Uart::input(node))?;
            Ok(generated::uart::Uart::events(node))
        });
        prop_assert_eq!(actual, expected);
    }
}

// SPI, and SPI NOR on it

fn spi_config(framed: bool) -> impl Strategy<Value = hand::spi::SpiConfig> {
    (0..4u8, any::<bool>(), any::<bool>(), 1..=8u8, any::<bool>()).prop_map(
        move |(mode, cs_active_low, lsb, word_bits, no_miso)| hand::spi::SpiConfig {
            clk: 0,
            cs: if framed { Some(1) } else { None },
            mosi: Some(2),
            miso: if no_miso && !framed { None } else { Some(3) },
            mode,
            word_bits: if framed { 8 } else { word_bits },
            bit_order: if lsb && !framed {
                hand::spi::BitOrder::LsbFirst
            } else {
                hand::spi::BitOrder::MsbFirst
            },
            cs_active_low,
        },
    )
}

#[derive(Clone, Debug)]
enum SpiPiece {
    Transaction(Vec<(u8, u8)>),
    Partial(u8, u8, u8),
    Idle(u64),
}

fn spi_piece() -> impl Strategy<Value = SpiPiece> {
    prop_oneof![
        5 => prop::collection::vec((any::<u8>(), any::<u8>()), 0..6).prop_map(SpiPiece::Transaction),
        1 => (any::<u8>(), any::<u8>(), 1..8u8).prop_map(|(mosi, miso, bits)| SpiPiece::Partial(mosi, miso, bits)),
        2 => (0..40u64).prop_map(SpiPiece::Idle),
    ]
}

fn flash_transaction() -> impl Strategy<Value = Vec<(u8, u8)>> {
    prop_oneof![
        4 => (0..0x100_0000u32, prop::collection::vec(any::<u8>(), 0..40))
            .prop_map(|(address, data)| nor::read(0x03, address, 3, 0, &data)),
        2 => (0..0x100_0000u32, prop::collection::vec(any::<u8>(), 0..20))
            .prop_map(|(address, data)| nor::read(0x0b, address, 3, 1, &data)),
        2 => (0..0x100_0000u32, prop::collection::vec(any::<u8>(), 0..20))
            .prop_map(|(address, data)| nor::program(0x02, address, 3, &data)),
        1 => prop::collection::vec(any::<u8>(), 0..4).prop_map(|values| nor::read_status(0x05, &values)),
        1 => prop::collection::vec(any::<u8>(), 0..3).prop_map(|values| nor::write_status(0x01, &values)),
        1 => prop::collection::vec(any::<u8>(), 0..20).prop_map(|id| nor::read_id(0x9f, 0, &id)),
        1 => Just(nor::command(0x06)),
        1 => Just(nor::command(0xb7)),
        1 => Just(nor::command(0x66)),
        1 => Just(nor::command(0x99)),
        1 => (any::<u8>(), prop::collection::vec((any::<u8>(), any::<u8>()), 0..4))
            .prop_map(|(opcode, rest)| { let mut words = vec![(opcode, 0)]; words.extend(rest); words }),
        1 => (0..3u8).prop_map(|bytes| nor::read(0x03, 0x1234, bytes, 0, &[]).into_iter().take(1 + usize::from(bytes)).collect()),
    ]
}

fn seeded_gaps(seed: Vec<(u64, u64)>, len: u64) -> Vec<TickRange> {
    let mut gaps: Vec<TickRange> = seed
        .into_iter()
        .map(|(start, width)| {
            TickRange::new(Tick(start % len.max(1)), Tick((start % len.max(1)) + width))
        })
        .collect();
    gaps.sort_by_key(|gap| gap.start);
    gaps.dedup_by(|b, a| b.start < a.end);
    gaps
}

proptest! {
    #![proptest_config(ProptestConfig::default())]

    #[test]
    fn spi_matches(
        framed in any::<bool>(),
        config_seed in (0..4u8, any::<bool>(), any::<bool>(), 1..=8u8, any::<bool>()),
        half_period in 1..5u64,
        pieces in prop::collection::vec(spi_piece(), 0..8),
        cuts in sizes(),
        gap_seed in prop::collection::vec((0..3000u64, 1..40u64), 0..3),
    ) {
        let (mode, cs_active_low, lsb, word_bits, no_miso) = config_seed;
        let config = hand::spi::SpiConfig {
            clk: 0,
            cs: if framed { Some(1) } else { None },
            mosi: Some(2),
            miso: if no_miso { None } else { Some(3) },
            mode,
            word_bits,
            bit_order: if lsb { hand::spi::BitOrder::LsbFirst } else { hand::spi::BitOrder::MsbFirst },
            cs_active_low,
        };
        let mut waveform = SpiWaveform::new(config.clone(), 4, half_period, Tick::ZERO);
        waveform.idle(2);
        for piece in pieces {
            match piece {
                SpiPiece::Transaction(words) => {
                    let mask = if word_bits == 8 { 0xff } else { (1u8 << word_bits) - 1 };
                    waveform.transaction(words.into_iter().map(|(mosi, miso)| (mosi & mask, miso & mask)));
                }
                SpiPiece::Partial(mosi, miso, bits) => {
                    waveform.select();
                    waveform.partial_word(mosi, miso, bits.min(word_bits));
                    waveform.release();
                }
                SpiPiece::Idle(samples) => waveform.idle(samples),
            }
        }
        waveform.idle(3);
        let block = waveform.finish();
        let gaps = seeded_gaps(gap_seed, block.len());
        let expected = run(4, 1_000_000, &block, &gaps, &cuts, |graph, source| {
            let node = graph.add(hand::spi::Spi::new(config.clone()).expect("valid"));
            graph.connect(source.samples, hand::spi::Spi::input(node))?;
            Ok(hand::spi::Spi::events(node))
        });
        let generated_config: generated::spi::SpiConfig = convert(&config);
        let actual = run(4, 1_000_000, &block, &gaps, &cuts, |graph, source| {
            let node = graph.add(generated::spi::Spi::new(generated_config).expect("valid"));
            graph.connect(source.samples, generated::spi::Spi::input(node))?;
            Ok(generated::spi::Spi::events(node))
        });
        prop_assert_eq!(actual, expected);
    }

    #[test]
    fn spi_nor_matches(
        config in spi_config(true),
        auto in any::<bool>(),
        half_period in 1..4u64,
        transactions in prop::collection::vec((flash_transaction(), 0..30u64), 0..8),
        cuts in sizes(),
        gap_seed in prop::collection::vec((0..6000u64, 1..60u64), 0..3),
    ) {
        let nor_config = hand::spi_nor::SpiNorConfig {
            address_bytes: if auto { hand::spi_nor::AddressBytes::Auto } else { hand::spi_nor::AddressBytes::Three },
        };
        let mut waveform = SpiWaveform::new(config.clone(), 4, half_period, Tick::ZERO);
        waveform.idle(2);
        for (words, idle) in transactions {
            waveform.transaction(words);
            waveform.idle(idle);
        }
        waveform.idle(3);
        let block = waveform.finish();
        let gaps = seeded_gaps(gap_seed, block.len());
        let spi_hand = RefCell::new(Vec::new());
        let expected = {
            let mut harness = Harness::build(4, TimeBase::new(1_000_000), GraphConfig::default(), |graph, source| {
                let spi = graph.add(hand::spi::Spi::new(config.clone()).expect("valid"));
                graph.connect(source.samples, hand::spi::Spi::input(spi))?;
                let flash = graph.add(hand::spi_nor::SpiNor::new(nor_config.clone()));
                graph.connect(hand::spi::Spi::events(spi), hand::spi_nor::SpiNor::input(flash))?;
                let exposed = graph.expose("spi", hand::spi::Spi::events(spi))?;
                spi_hand.borrow_mut().push(exposed);
                Ok(hand::spi_nor::SpiNor::events(flash))
            }).expect("builds");
            let tap = harness.tap(spi_hand.borrow_mut().pop().expect("exposed"));
            harness.feed_with_gaps(&block, &gaps, &cuts);
            harness.end();
            (lines(&harness.items), lines(&tap.items()))
        };
        let generated_config: generated::spi::SpiConfig = convert(&config);
        let generated_nor: generated::spi_nor::SpiNorConfig = convert(&nor_config);
        let spi_generated = RefCell::new(Vec::new());
        let actual = {
            let mut harness = Harness::build(4, TimeBase::new(1_000_000), GraphConfig::default(), |graph, source| {
                let spi = graph.add(generated::spi::Spi::new(generated_config).expect("valid"));
                graph.connect(source.samples, generated::spi::Spi::input(spi))?;
                let flash = graph.add(generated::spi_nor::SpiNor::new(generated_nor).expect("valid"));
                graph.connect(generated::spi::Spi::events(spi), generated::spi_nor::SpiNor::input(flash))?;
                let exposed = graph.expose("spi", generated::spi::Spi::events(spi))?;
                spi_generated.borrow_mut().push(exposed);
                Ok(generated::spi_nor::SpiNor::events(flash))
            }).expect("builds");
            let tap = harness.tap(spi_generated.borrow_mut().pop().expect("exposed"));
            harness.feed_with_gaps(&block, &gaps, &cuts);
            harness.end();
            (lines(&harness.items), lines(&tap.items()))
        };
        prop_assert_eq!(&actual.1, &expected.1, "spi");
        prop_assert_eq!(&actual.0, &expected.0, "nor");
    }
}
