// SPDX-License-Identifier: Apache-2.0

//! The SPI NOR decoder on synthesized SPI waveforms.

mod common;

use common::{CHANNELS, Harness};
use uiharu_core::{GraphConfig, GraphError, Tick, TickRange};
use uiharu_protocols::spi::SpiConfig;
use uiharu_protocols::spi::synth::{SpiWaveform, nor_read_words};
use uiharu_protocols::spi_nor::SpiNorRead24;

const HALF: u64 = 3;

fn waveform() -> SpiWaveform {
    let mut waveform = SpiWaveform::new(SpiConfig::default(), CHANNELS, HALF, Tick::ZERO);
    waveform.idle(10);
    waveform
}

fn data(len: usize) -> Vec<u8> {
    (0..len).map(|i| (i * 7 % 251) as u8).collect()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[test]
fn decodes_a_read_in_batches() {
    let bytes = data(40);
    let mut waveform = waveform();
    waveform.transaction(nor_read_words(0x123456, &bytes));
    waveform.idle(10);
    let block = waveform.finish();
    let mut harness = Harness::nor(SpiConfig::default());
    harness.run_all(&block);
    let lines = harness.nor_lines();
    let word_len = 8 * 2 * HALF;
    let first_bit = 10 + 2 * HALF;
    let word_span = |from: u64, to: u64| {
        let start = first_bit + from * word_len;
        let last = first_bit + (to - 1) * word_len + 7 * 2 * HALF;
        format!("[{},{})@{}", start, last + 1, last)
    };
    // The operation begins where chip select asserts and runs through its opcode.
    assert_eq!(
        lines[0],
        format!("begin op1 tx1 Read [10,{}", &word_span(0, 1)[4..])
    );
    assert_eq!(lines[1], format!("address op1 123456 {}", word_span(1, 4)));
    assert_eq!(
        lines[2],
        format!("data op1 +0 {} {}", hex(&bytes[..32]), word_span(4, 36))
    );
    // The last batch is decided by the end of the command, where it settles.
    let release = 10 + HALF + 44 * word_len + HALF;
    let last_start = first_bit + 36 * word_len;
    let last_end = first_bit + 43 * word_len + 7 * 2 * HALF + 1;
    assert_eq!(
        lines[3],
        format!(
            "data op1 +32 {} [{last_start},{last_end})@{release}",
            hex(&bytes[32..])
        )
    );
    assert_eq!(
        lines[4],
        format!("end op1 Complete [{release},{})@{release}", release + 1)
    );
    assert!(lines[5].starts_with("eof "), "{lines:?}");
    assert_eq!(lines.len(), 6);
}

#[test]
fn an_unknown_opcode_is_reported_and_its_words_skipped() {
    let mut waveform = waveform();
    waveform.transaction([(0x9f, 0xff), (0x00, 0xef), (0x00, 0x40)]);
    waveform.idle(10);
    waveform.transaction(nor_read_words(0x000010, &[0xaa, 0xbb]));
    waveform.idle(10);
    let block = waveform.finish();
    let mut harness = Harness::nor(SpiConfig::default());
    harness.run_all(&block);
    let lines = harness.nor_lines();
    assert!(lines[0].starts_with("unknown tx1 9f "), "{lines:?}");
    assert!(lines[1].starts_with("begin op1 tx2 Read "), "{lines:?}");
    assert!(lines[2].starts_with("address op1 000010 "), "{lines:?}");
    assert!(lines[3].starts_with("data op1 +0 aabb "), "{lines:?}");
    assert!(lines[4].starts_with("end op1 Complete "), "{lines:?}");
    assert_eq!(lines.len(), 6, "{lines:?}");
}

#[test]
fn a_short_command_ends_incomplete() {
    let mut waveform = waveform();
    waveform.transaction([(0x03, 0xff), (0x12, 0xff)]);
    waveform.idle(10);
    waveform.transaction([(0x03, 0xff)]);
    waveform.idle(10);
    waveform.transaction([]);
    waveform.idle(10);
    waveform.transaction(nor_read_words(0x0000ff, &[]));
    waveform.idle(10);
    let block = waveform.finish();
    let mut harness = Harness::nor(SpiConfig::default());
    harness.run_all(&block);
    let lines = harness.nor_lines();
    assert!(lines[0].starts_with("begin op1 tx1 Read "), "{lines:?}");
    assert!(lines[1].starts_with("end op1 Incomplete "), "{lines:?}");
    assert!(lines[2].starts_with("begin op2 tx2 Read "), "{lines:?}");
    assert!(lines[3].starts_with("end op2 Incomplete "), "{lines:?}");
    // An empty transaction has no opcode and is nothing to report; a read of no bytes is whole.
    assert!(lines[4].starts_with("begin op3 tx4 Read "), "{lines:?}");
    assert!(lines[5].starts_with("address op3 0000ff "), "{lines:?}");
    assert!(lines[6].starts_with("end op3 Complete "), "{lines:?}");
    assert_eq!(lines.len(), 8, "{lines:?}");
    // The incomplete end covers the address byte that was read without making an address,
    // through the chip select release that cut the command short.
    let word_len = 8 * 2 * HALF;
    let second_word = 10 + 2 * HALF + word_len;
    let release = 10 + HALF + 2 * word_len + HALF;
    assert_eq!(
        lines[1],
        format!(
            "end op1 Incomplete [{second_word},{})@{release}",
            release + 1
        )
    );
}

#[test]
fn a_read_cut_off_by_the_end_of_the_samples_is_aborted() {
    let bytes = data(50);
    let mut waveform = waveform();
    waveform.select();
    waveform.words(nor_read_words(0x00ab00, &bytes));
    waveform.partial_word(0, 0x11, 3);
    let block = waveform.finish();
    let mut harness = Harness::nor(SpiConfig::default());
    harness.run_all(&block);
    let lines = harness.nor_lines();
    assert!(lines[2].starts_with("data op1 +0 "), "{lines:?}");
    assert!(
        lines[3].starts_with(&format!("data op1 +32 {} [", hex(&bytes[32..]))),
        "{lines:?}"
    );
    assert!(
        lines[3].ends_with(&format!(")@{}", block.end())),
        "{lines:?}"
    );
    assert_eq!(lines[4], format!("end op1 Aborted @{}", block.end()));
    assert_eq!(lines[5], format!("eof {}", block.end()));
}

#[test]
fn a_gap_aborts_the_read_and_skips_the_rest_of_the_transaction() {
    let bytes = data(10);
    let mut waveform = waveform();
    waveform.transaction(nor_read_words(0x000100, &bytes));
    waveform.idle(10);
    waveform.transaction(nor_read_words(0x000200, &[0x42]));
    waveform.idle(10);
    let block = waveform.finish();
    // Lose the sixth and seventh data byte.
    let word_len = 8 * 2 * HALF;
    let gap_start = 10 + 2 * HALF + 9 * word_len + 2;
    let gap = TickRange::new(Tick(gap_start), Tick(gap_start + 2 * word_len));
    let mut harness = Harness::nor(SpiConfig::default());
    harness.feed_with_gap(&block, gap, 11);
    harness.end();
    let lines = harness.nor_lines();
    assert!(lines[0].starts_with("begin op1 tx1 Read "), "{lines:?}");
    assert!(lines[1].starts_with("address op1 000100 "), "{lines:?}");
    assert!(
        lines[2].starts_with(&format!("data op1 +0 {} ", hex(&bytes[..5]))),
        "{lines:?}"
    );
    assert!(lines[2].ends_with(&format!(")@{}", gap.start)), "{lines:?}");
    assert_eq!(lines[3], format!("end op1 Aborted @{}", gap.start));
    assert_eq!(lines[4], format!("gap {gap}"));
    // The transaction found under way after the gap is skipped; the next read is whole.
    assert!(lines[5].starts_with("begin op2 tx3 Read "), "{lines:?}");
    assert!(lines[6].starts_with("address op2 000200 "), "{lines:?}");
    assert!(lines[7].starts_with("data op2 +0 42 "), "{lines:?}");
    assert!(lines[8].starts_with("end op2 Complete "), "{lines:?}");
    assert_eq!(lines.len(), 10, "{lines:?}");
}

#[test]
fn data_shows_up_before_the_read_ends() {
    let bytes = data(100);
    let mut waveform = waveform();
    waveform.transaction(nor_read_words(0x000000, &bytes));
    waveform.idle(10);
    let block = waveform.finish();
    // Feed up to the middle of the 70th data byte.
    let word_len = 8 * 2 * HALF;
    let cut = Tick(10 + 2 * HALF + (4 + 70) * word_len + 5);
    let head = block
        .slice(TickRange::new(Tick::ZERO, cut))
        .expect("a slice");
    let mut harness = Harness::nor(SpiConfig::default());
    harness.feed(&head, 37);
    harness.collect();
    let lines = harness.nor_lines();
    assert_eq!(lines.len(), 4, "{lines:?}");
    assert!(
        lines[2].starts_with(&format!("data op1 +0 {} ", hex(&bytes[..32]))),
        "{lines:?}"
    );
    assert!(
        lines[3].starts_with(&format!("data op1 +32 {} ", hex(&bytes[32..64]))),
        "{lines:?}"
    );
    // The SPI words are there too, 74 of them.
    assert_eq!(
        harness
            .spi_lines()
            .iter()
            .filter(|line| line.starts_with("word "))
            .count(),
        74
    );
    let tail = block
        .slice(TickRange::new(cut, block.end()))
        .expect("a slice");
    harness.feed(&tail, 37);
    harness.end();
    let lines = harness.nor_lines();
    assert!(
        lines[4].starts_with(&format!("data op1 +64 {} ", hex(&bytes[64..96]))),
        "{lines:?}"
    );
    assert!(
        lines[5].starts_with(&format!("data op1 +96 {} ", hex(&bytes[96..]))),
        "{lines:?}"
    );
    assert!(lines[6].starts_with("end op1 Complete "), "{lines:?}");
}

#[test]
fn the_bus_must_meet_the_decoders_requirements() {
    let narrow = SpiConfig {
        word_bits: 4,
        ..SpiConfig::default()
    };
    let error = Harness::build(narrow, true, GraphConfig::default())
        .err()
        .expect("rejected");
    assert_eq!(
        error,
        GraphError::Requirement {
            from: "spi#0.events".into(),
            to: "spi-nor-read24#1.bus".into(),
            key: "wordBits",
            requirement: "= 8".into(),
            found: "4".into(),
        }
    );
    let unframed = SpiConfig {
        cs: None,
        ..SpiConfig::default()
    };
    assert!(matches!(
        Harness::build(unframed, true, GraphConfig::default()).err(),
        Some(GraphError::Requirement { key: "cs", .. })
    ));
    let half_duplex = SpiConfig {
        miso: None,
        ..SpiConfig::default()
    };
    assert!(matches!(
        Harness::build(half_duplex, true, GraphConfig::default()).err(),
        Some(GraphError::Requirement { key: "miso", .. })
    ));
    let lsb = SpiConfig {
        bit_order: uiharu_protocols::spi::BitOrder::LsbFirst,
        ..SpiConfig::default()
    };
    assert!(matches!(
        Harness::build(lsb, true, GraphConfig::default()).err(),
        Some(GraphError::Requirement {
            key: "bitOrder",
            ..
        })
    ));
    assert!(
        SpiNorRead24::requires()
            .unmet(&SpiConfig::default().props())
            .is_none()
    );
}
