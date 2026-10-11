// SPDX-License-Identifier: Apache-2.0

//! The SPI decoder on synthesized waveforms.

mod common;

use std::sync::Arc;

use common::{CHANNELS, Harness};
use uiharu_core::{Levels, LogicBlockBuilder, SignalItem, Tick, TickRange, TimeBase};
use uiharu_protocols::spi::synth::SpiWaveform;
use uiharu_protocols::spi::{BitOrder, Spi, SpiConfig};

const HALF: u64 = 5;

fn waveform(config: &SpiConfig) -> SpiWaveform {
    let mut waveform = SpiWaveform::new(config.clone(), CHANNELS, HALF, Tick::ZERO);
    waveform.idle(20);
    waveform
}

#[test]
fn decodes_a_transaction_in_every_mode_and_bit_order() {
    for mode in 0..4 {
        for bit_order in [BitOrder::MsbFirst, BitOrder::LsbFirst] {
            let config = SpiConfig {
                mode,
                bit_order,
                ..SpiConfig::default()
            };
            let mut waveform = waveform(&config);
            waveform.transaction([(0x03, 0xff), (0xa5, 0x5a), (0x00, 0x12)]);
            waveform.idle(20);
            let block = waveform.finish();
            let mut harness = Harness::spi(config);
            harness.run_all(&block);
            let lines = harness.spi_lines();
            // Chip select asserts at 20; the first bit is taken half a period into the first
            // bit cell, which starts another half period later; a bit cell is a full period.
            let first_bit = 20 + HALF + HALF;
            let word = |n: u64, mosi: &str| {
                let start = first_bit + n * 8 * 2 * HALF;
                let last = start + 7 * 2 * HALF;
                format!("word tx1 {mosi} [{},{})@{}", start, last + 1, last)
            };
            let release = 20 + HALF + 3 * 8 * 2 * HALF + HALF;
            assert_eq!(
                lines,
                vec![
                    "begin tx1 [20,21)@20".to_owned(),
                    word(0, "03/ff"),
                    word(1, "a5/5a"),
                    word(2, "00/12"),
                    format!("end tx1 CsRelease [{release},{})@{release}", release + 1),
                    format!("eof {}", release + 20),
                ],
                "mode {mode}, {bit_order:?}"
            );
        }
    }
}

#[test]
fn chip_select_found_asserted_begins_a_partial_transaction() {
    let config = SpiConfig::default();
    let mut waveform = waveform(&config);
    waveform.transaction([(0x11, 0x01), (0x22, 0x02), (0x33, 0x03)]);
    waveform.idle(10);
    let block = waveform.finish();
    // Start where the second word's first bit cell begins. The decoder cannot know how many
    // bits of a transaction it missed, so only a cut at a word boundary keeps the words
    // aligned; a cut elsewhere shifts them, which is why decoders built on SPI skip partial
    // transactions.
    let from = Tick(20 + HALF + 8 * 2 * HALF);
    let tail = block
        .slice(TickRange::new(from, block.end()))
        .expect("a slice");
    let mut harness = Harness::spi(config);
    harness.run_all(&tail);
    let lines = harness.spi_lines();
    assert_eq!(
        lines[0],
        format!("begin tx1 partial [{from},{})@{from}", from.next())
    );
    assert!(lines[1].starts_with("word tx1 22/02 "), "{lines:?}");
    assert!(lines[2].starts_with("word tx1 33/03 "), "{lines:?}");
    assert!(lines[3].starts_with("end tx1 CsRelease "), "{lines:?}");
    assert_eq!(lines.len(), 5, "{lines:?}");
}

#[test]
fn leftover_bits_do_not_make_a_word() {
    let config = SpiConfig::default();
    let mut waveform = waveform(&config);
    waveform.select();
    waveform.word(0x5a, 0xa5);
    waveform.partial_word(0xff, 0xff, 5);
    waveform.release();
    waveform.idle(10);
    let block = waveform.finish();
    let mut harness = Harness::spi(config);
    harness.run_all(&block);
    let lines = harness.spi_lines();
    assert_eq!(lines.len(), 4, "{lines:?}");
    assert!(lines[1].starts_with("word tx1 5a/a5 "));
    assert!(lines[2].starts_with("end tx1 CsRelease "));
}

#[test]
fn without_chip_select_words_are_unframed() {
    let config = SpiConfig {
        cs: None,
        ..SpiConfig::default()
    };
    let mut waveform = waveform(&config);
    waveform.words([(0x01, 0x10), (0x02, 0x20)]);
    waveform.idle(10);
    let block = waveform.finish();
    let mut harness = Harness::spi(config);
    harness.run_all(&block);
    let lines = harness.spi_lines();
    assert_eq!(lines.len(), 3, "{lines:?}");
    assert!(lines[0].starts_with("word tx0 01/10 "));
    assert!(lines[1].starts_with("word tx0 02/20 "));
    assert!(lines[2].starts_with("eof "));
}

#[test]
fn a_missing_line_reads_none() {
    let config = SpiConfig {
        miso: None,
        ..SpiConfig::default()
    };
    let mut waveform = waveform(&config);
    waveform.transaction([(0x7e, 0x00)]);
    waveform.idle(10);
    let block = waveform.finish();
    let mut harness = Harness::spi(config);
    harness.run_all(&block);
    assert!(
        harness.spi_lines()[1].starts_with("word tx1 7e/-- "),
        "{:?}",
        harness.spi_lines()
    );
}

#[test]
fn a_gap_aborts_the_transaction_and_resyncs() {
    let config = SpiConfig::default();
    let mut waveform = waveform(&config);
    waveform.select();
    waveform.words([(0x01, 0x10), (0x02, 0x20), (0x03, 0x30)]);
    waveform.release();
    waveform.idle(10);
    let block = waveform.finish();
    // The gap swallows the second word and the start of the third.
    let word_start = |n: u64| 20 + HALF + n * 8 * 2 * HALF;
    let gap = TickRange::new(Tick(word_start(1) + 3), Tick(word_start(2) + 3 * 2 * HALF));
    let mut harness = Harness::spi(config);
    harness.feed_with_gap(&block, gap, 7);
    harness.end();
    let lines = harness.spi_lines();
    assert_eq!(lines[0], "begin tx1 [20,21)@20");
    assert!(lines[1].starts_with("word tx1 01/10 "), "{lines:?}");
    assert_eq!(lines[2], format!("end tx1 Aborted @{}", gap.start));
    assert_eq!(lines[3], format!("gap {gap}"));
    assert_eq!(
        lines[4],
        format!(
            "begin tx2 partial [{},{})@{}",
            gap.end,
            gap.end.next(),
            gap.end
        )
    );
    // The bits of the third word before the gap are gone; the rest do not make a word.
    assert!(lines[5].starts_with("end tx2 CsRelease "), "{lines:?}");
    assert!(lines[6].starts_with("eof "), "{lines:?}");
    assert_eq!(lines.len(), 7);
}

#[test]
fn the_end_of_the_samples_aborts_an_open_transaction() {
    let config = SpiConfig::default();
    let mut waveform = waveform(&config);
    waveform.select();
    waveform.words([(0x01, 0x10), (0x02, 0x20)]);
    waveform.partial_word(0x03, 0x30, 4);
    let block = waveform.finish();
    let end = block.end();
    let mut harness = Harness::spi(config);
    harness.run_all(&block);
    let lines = harness.spi_lines();
    assert_eq!(lines.len(), 5, "{lines:?}");
    assert_eq!(lines[3], format!("end tx1 Aborted @{end}"));
    assert_eq!(lines[4], format!("eof {end}"));
}

#[test]
fn a_clock_edge_as_chip_select_changes_belongs_to_no_transaction() {
    // Mode 0, active-low chip select on channel 1, clock on 0, MOSI on 2. Eight rising edges
    // would make a word; the eighth coincides with the release of chip select.
    let config = SpiConfig {
        miso: None,
        ..SpiConfig::default()
    };
    let mut builder = LogicBlockBuilder::new(Tick::ZERO, CHANNELS);
    let cs_high = 0b0010;
    let mosi = 0b0100;
    builder.push_repeated(Levels(cs_high), 4);
    builder.push_repeated(Levels(0), 4); // selected, clock low
    for _ in 0..7 {
        builder.push_repeated(Levels(mosi | 1), 2); // rising edge with MOSI high
        builder.push_repeated(Levels(mosi), 2);
    }
    builder.push_repeated(Levels(cs_high | 1), 4); // the eighth rising edge, as CS releases
    builder.push_repeated(Levels(cs_high), 4);
    let block = builder.finish().expect("a block");
    let mut harness = Harness::spi(config);
    harness.run_all(&block);
    let lines = harness.spi_lines();
    assert_eq!(
        lines,
        vec![
            "begin tx1 [4,5)@4",
            "end tx1 CsRelease [36,37)@36",
            "eof 44"
        ]
    );
}

#[test]
fn blocks_of_any_size_decode_alike() {
    let config = SpiConfig {
        mode: 3,
        ..SpiConfig::default()
    };
    let mut waveform = waveform(&config);
    for n in 0..20u8 {
        waveform.transaction((0..(n % 5 + 1)).map(|i| (n.wrapping_mul(7) ^ i, i.wrapping_mul(13))));
        waveform.idle(u64::from(n) * 3 + 1);
    }
    let block = waveform.finish();
    let mut whole = Harness::spi(config.clone());
    whole.run_all(&block);
    let reference = common::without_progress(whole.spi_items());
    assert!(reference.len() > 100);
    for sizes in [&[1u64][..], &[2, 3, 5, 7], &[64], &[63, 65, 1], &[1000, 1]] {
        let mut harness = Harness::spi(config.clone());
        harness.feed_sizes(&block, sizes);
        harness.end();
        assert_eq!(
            common::without_progress(harness.spi_items()),
            reference,
            "sizes {sizes:?}"
        );
    }
}

#[test]
fn every_cut_and_pause_yields_the_same_events() {
    let config = SpiConfig {
        mode: 1,
        ..SpiConfig::default()
    };
    let mut waveform = waveform(&config);
    for n in 0..12u8 {
        waveform
            .transaction((0..(n % 4 + 1)).map(|i| (n.wrapping_mul(29) ^ i, i.wrapping_mul(17))));
        waveform.idle(u64::from(n) * 2 + 3);
    }
    let block = waveform.finish();
    let gap = TickRange::new(Tick(block.len() / 3), Tick(block.len() / 3 + 37));
    let build = |graph_config| {
        uiharu_core::testing::Harness::build(
            CHANNELS,
            TimeBase::new(1_000_000),
            graph_config,
            |graph, source| {
                let spi = graph.add(Spi::new(config.clone()).expect("a valid configuration"));
                graph.connect(source.samples, Spi::input(spi))?;
                Ok(Spi::events(spi))
            },
        )
        .expect("builds")
    };
    uiharu_core::testing::assert_block_invariant(build, &block, &[gap]);
}

#[test]
fn progress_is_reported_as_blocks_are_used_up() {
    let config = SpiConfig::default();
    let mut harness = Harness::spi(config);
    let mut builder = LogicBlockBuilder::new(Tick::ZERO, CHANNELS);
    builder.push_repeated(Levels(0b0010), 100);
    let block = Arc::new(builder.finish().expect("a block"));
    harness.push(SignalItem::Data(block));
    harness.push(SignalItem::Progress(Tick(100)));
    harness.collect();
    assert_eq!(harness.spi_lines(), vec!["progress 100"]);
}
