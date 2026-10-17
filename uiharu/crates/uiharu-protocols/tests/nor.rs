// SPDX-License-Identifier: Apache-2.0

//! The SPI NOR decoder on synthesized SPI waveforms.

mod common;

use common::{CHANNELS, Harness, RATE};
use uiharu_core::testing::assert_progress_promise;
use uiharu_core::{GraphConfig, GraphError, Item, Tick, TickRange, TimeBase};
use uiharu_protocols::spi::synth::{SpiWaveform, nor_read_words};
use uiharu_protocols::spi::{Spi, SpiConfig};
use uiharu_protocols::spi_nor::{AddressBytes, NorEvent, SpiNor, SpiNorConfig, command_of, synth};

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

fn auto() -> SpiNorConfig {
    SpiNorConfig {
        address_bytes: AddressBytes::Auto,
    }
}

/// The lines without their ticks, the end of the stream left out.
fn bare(lines: &[String]) -> Vec<String> {
    lines
        .iter()
        .filter(|line| !line.starts_with("eof "))
        .map(|line| {
            line.rsplit_once(' ')
                .map_or(line.as_str(), |(what, _)| what)
        })
        .map(str::to_owned)
        .collect()
}

/// Decodes `transactions`, each in its own chip select period, and returns the lines.
fn decode_with_ticks(config: SpiNorConfig, transactions: &[Vec<(u8, u8)>]) -> Vec<String> {
    let mut waveform = waveform();
    for words in transactions {
        waveform.transaction(words.iter().copied());
        waveform.idle(10);
    }
    let block = waveform.finish();
    let mut harness = Harness::nor_with(SpiConfig::default(), config);
    harness.run_all(&block);
    harness.nor_lines()
}

/// Decodes `transactions` and returns the lines without their ticks.
fn decode(config: SpiNorConfig, transactions: &[Vec<(u8, u8)>]) -> Vec<String> {
    bare(&decode_with_ticks(config, transactions))
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
        format!("data op1 out +0 {} {}", hex(&bytes[..32]), word_span(4, 36))
    );
    // The last batch is decided by the end of the command, where it settles.
    let release = 10 + HALF + 44 * word_len + HALF;
    let last_start = first_bit + 36 * word_len;
    let last_end = first_bit + 43 * word_len + 7 * 2 * HALF + 1;
    assert_eq!(
        lines[3],
        format!(
            "data op1 out +32 {} [{last_start},{last_end})@{release}",
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
    waveform.transaction([(0xeb, 0xff), (0x00, 0xef), (0x00, 0x40)]);
    waveform.idle(10);
    waveform.transaction(nor_read_words(0x000010, &[0xaa, 0xbb]));
    waveform.idle(10);
    let block = waveform.finish();
    let mut harness = Harness::nor(SpiConfig::default());
    harness.run_all(&block);
    let lines = harness.nor_lines();
    assert!(lines[0].starts_with("unknown tx1 eb "), "{lines:?}");
    assert!(lines[1].starts_with("begin op1 tx2 Read "), "{lines:?}");
    assert!(lines[2].starts_with("address op1 000010 "), "{lines:?}");
    assert!(lines[3].starts_with("data op1 out +0 aabb "), "{lines:?}");
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
    assert!(lines[2].starts_with("data op1 out +0 "), "{lines:?}");
    assert!(
        lines[3].starts_with(&format!("data op1 out +32 {} [", hex(&bytes[32..]))),
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
        lines[2].starts_with(&format!("data op1 out +0 {} ", hex(&bytes[..5]))),
        "{lines:?}"
    );
    assert!(lines[2].ends_with(&format!(")@{}", gap.start)), "{lines:?}");
    assert_eq!(lines[3], format!("end op1 Aborted @{}", gap.start));
    assert_eq!(lines[4], format!("gap {gap}"));
    // The transaction found under way after the gap is skipped; the next read is whole.
    assert!(lines[5].starts_with("begin op2 tx3 Read "), "{lines:?}");
    assert!(lines[6].starts_with("address op2 000200 "), "{lines:?}");
    assert!(lines[7].starts_with("data op2 out +0 42 "), "{lines:?}");
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
        lines[2].starts_with(&format!("data op1 out +0 {} ", hex(&bytes[..32]))),
        "{lines:?}"
    );
    assert!(
        lines[3].starts_with(&format!("data op1 out +32 {} ", hex(&bytes[32..64]))),
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
        lines[4].starts_with(&format!("data op1 out +64 {} ", hex(&bytes[64..96]))),
        "{lines:?}"
    );
    assert!(
        lines[5].starts_with(&format!("data op1 out +96 {} ", hex(&bytes[96..]))),
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
    let error = Harness::build(
        narrow,
        Some(SpiNorConfig::default()),
        GraphConfig::default(),
    )
    .err()
    .expect("rejected");
    assert_eq!(
        error,
        GraphError::Requirement {
            from: "spi#0.events".into(),
            to: "spi-nor#1.bus".into(),
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
        Harness::build(
            unframed,
            Some(SpiNorConfig::default()),
            GraphConfig::default()
        )
        .err(),
        Some(GraphError::Requirement { key: "cs", .. })
    ));
    let half_duplex = SpiConfig {
        miso: None,
        ..SpiConfig::default()
    };
    assert!(matches!(
        Harness::build(
            half_duplex,
            Some(SpiNorConfig::default()),
            GraphConfig::default()
        )
        .err(),
        Some(GraphError::Requirement { key: "miso", .. })
    ));
    let lsb = SpiConfig {
        bit_order: uiharu_protocols::spi::BitOrder::LsbFirst,
        ..SpiConfig::default()
    };
    assert!(matches!(
        Harness::build(lsb, Some(SpiNorConfig::default()), GraphConfig::default()).err(),
        Some(GraphError::Requirement {
            key: "bitOrder",
            ..
        })
    ));
    assert!(
        SpiNorConfig::default()
            .requires()
            .unmet(&SpiConfig::default().props())
            .is_none()
    );
}

#[test]
fn status_registers_are_read_and_written_byte_by_byte() {
    let lines = decode(
        SpiNorConfig::default(),
        &[
            synth::read_status(0x05, &[0x03, 0x02, 0x00]),
            synth::read_status(0x35, &[0x02]),
            synth::read_status(0x15, &[0x60]),
            synth::write_status(0x01, &[0x1c]),
            synth::write_status(0x01, &[0x00, 0x02, 0x99]),
            synth::write_status(0x31, &[0x02]),
            synth::write_status(0x11, &[0x60, 0x61]),
        ],
    );
    assert_eq!(
        lines,
        [
            "begin op1 tx1 ReadStatus",
            "status op1 read sr1 03",
            "status op1 read sr1 02",
            "status op1 read sr1 00",
            "end op1 Complete",
            "begin op2 tx2 ReadStatus2",
            "status op2 read sr2 02",
            "end op2 Complete",
            "begin op3 tx3 ReadStatus3",
            "status op3 read sr3 60",
            "end op3 Complete",
            "begin op4 tx4 WriteStatus",
            "status op4 write sr1 1c",
            "end op4 Complete",
            // A third byte is more than Write Status takes: it is skipped.
            "begin op5 tx5 WriteStatus",
            "status op5 write sr1 00",
            "status op5 write sr2 02",
            "end op5 Complete",
            "begin op6 tx6 WriteStatus2",
            "status op6 write sr2 02",
            "end op6 Complete",
            "begin op7 tx7 WriteStatus3",
            "status op7 write sr3 60",
            "end op7 Complete",
        ]
    );
    // Each value covers its own word and is settled at its last bit.
    let lines = decode_with_ticks(
        SpiNorConfig::default(),
        &[synth::read_status(0x05, &[0x01, 0x00])],
    );
    let word_len = 8 * 2 * HALF;
    let word = |n: u64| {
        let start = 10 + 2 * HALF + n * word_len;
        let last = start + 7 * 2 * HALF;
        format!("[{start},{})@{last}", last + 1)
    };
    assert_eq!(lines[1], format!("status op1 read sr1 01 {}", word(1)));
    assert_eq!(lines[2], format!("status op1 read sr1 00 {}", word(2)));
}

#[test]
fn fast_reads_carry_a_dummy_byte_before_the_data() {
    let lines = decode_with_ticks(
        SpiNorConfig::default(),
        &[synth::read(0x0b, 0x001000, 3, 1, &[0xa1, 0xa2])],
    );
    let word_len = 8 * 2 * HALF;
    let first_bit = 10 + 2 * HALF;
    let words = |from: u64, to: u64| {
        let start = first_bit + from * word_len;
        let last = first_bit + (to - 1) * word_len + 7 * 2 * HALF;
        (start, last)
    };
    let release = 10 + HALF + 7 * word_len + HALF;
    let (_, opcode_last) = words(0, 1);
    let (address_start, address_last) = words(1, 4);
    let (dummy_start, dummy_last) = words(4, 5);
    let (data_start, data_last) = words(5, 7);
    assert_eq!(
        lines,
        [
            format!(
                "begin op1 tx1 FastRead [10,{})@{opcode_last}",
                opcode_last + 1
            ),
            format!(
                "address op1 001000 [{address_start},{})@{address_last}",
                address_last + 1
            ),
            format!(
                "dummy op1 1 [{dummy_start},{})@{dummy_last}",
                dummy_last + 1
            ),
            // The data is decided by the end of the command, where it settles.
            format!(
                "data op1 out +0 a1a2 [{data_start},{})@{release}",
                data_last + 1
            ),
            format!("end op1 Complete [{release},{})@{release}", release + 1),
            format!("eof {}", release + 10),
        ]
    );
}

#[test]
fn reads_report_what_the_flash_sent() {
    let lines = decode(
        SpiNorConfig::default(),
        &[
            synth::read(0x5a, 0x000000, 3, 1, b"SFDP"),
            synth::read(0x48, 0x001000, 3, 1, &[0xc1, 0xc2]),
            synth::read(0x03, 0x000010, 3, 0, &[]),
        ],
    );
    assert_eq!(
        lines,
        [
            "begin op1 tx1 ReadSfdp",
            "address op1 000000",
            "dummy op1 1",
            "data op1 out +0 53464450",
            "end op1 Complete",
            "begin op2 tx2 ReadSecurityRegister",
            "address op2 001000",
            "dummy op2 1",
            "data op2 out +0 c1c2",
            "end op2 Complete",
            // A read of no bytes is whole: how much to read is the host's choice.
            "begin op3 tx3 Read",
            "address op3 000010",
            "end op3 Complete",
        ]
    );
}

#[test]
fn programs_carry_data_in_on_mosi() {
    let page: Vec<u8> = (0..40u8).collect();
    let lines = decode(
        SpiNorConfig::default(),
        &[
            synth::program(0x02, 0x002000, 3, &page),
            synth::program(0x12, 0x0100_0000, 4, &[0xb1]),
            synth::program(0x42, 0x001000, 3, &[0xd1, 0xd2]),
        ],
    );
    assert_eq!(
        lines,
        [
            "begin op1 tx1 PageProgram".to_owned(),
            "address op1 002000".to_owned(),
            format!("data op1 in +0 {}", hex(&page[..32])),
            format!("data op1 in +32 {}", hex(&page[32..])),
            "end op1 Complete".to_owned(),
            "begin op2 tx2 PageProgram4b".to_owned(),
            "address op2 01000000".to_owned(),
            "data op2 in +0 b1".to_owned(),
            "end op2 Complete".to_owned(),
            "begin op3 tx3 ProgramSecurityRegister".to_owned(),
            "address op3 001000".to_owned(),
            "data op3 in +0 d1d2".to_owned(),
            "end op3 Complete".to_owned(),
        ]
    );
}

#[test]
fn erases_and_locks_carry_an_address() {
    let lines = decode(
        SpiNorConfig::default(),
        &[
            synth::addressed(0x20, 0x003000, 3),
            synth::addressed(0x21, 0x0100_3000, 4),
            synth::addressed(0x52, 0x008000, 3),
            synth::addressed(0xd8, 0x010000, 3),
            synth::addressed(0xdc, 0x0101_0000, 4),
            synth::addressed(0x44, 0x002000, 3),
            synth::addressed(0x36, 0x004000, 3),
            synth::addressed(0x39, 0x004000, 3),
            synth::command(0xc7),
            synth::command(0x60),
        ],
    );
    assert_eq!(
        lines,
        [
            "begin op1 tx1 SectorErase",
            "address op1 003000",
            "end op1 Complete",
            "begin op2 tx2 SectorErase4b",
            "address op2 01003000",
            "end op2 Complete",
            "begin op3 tx3 BlockErase32k",
            "address op3 008000",
            "end op3 Complete",
            "begin op4 tx4 BlockErase64k",
            "address op4 010000",
            "end op4 Complete",
            "begin op5 tx5 BlockErase64k4b",
            "address op5 01010000",
            "end op5 Complete",
            "begin op6 tx6 EraseSecurityRegister",
            "address op6 002000",
            "end op6 Complete",
            "begin op7 tx7 BlockLock",
            "address op7 004000",
            "end op7 Complete",
            "begin op8 tx8 BlockUnlock",
            "address op8 004000",
            "end op8 Complete",
            "begin op9 tx9 ChipErase",
            "end op9 Complete",
            "begin op10 tx10 ChipErase",
            "end op10 Complete",
        ]
    );
}

#[test]
fn commands_without_operands_skip_what_follows() {
    let opcodes = [
        0x06, 0x04, 0x50, 0xb9, 0xb7, 0xe9, 0x66, 0x99, 0x75, 0x7a, 0x7e, 0x98,
    ];
    let transactions: Vec<Vec<(u8, u8)>> = opcodes.iter().map(|op| synth::command(*op)).collect();
    let lines = decode(SpiNorConfig::default(), &transactions);
    let mut expected = Vec::new();
    for (n, opcode) in opcodes.iter().enumerate() {
        let command = command_of(*opcode).expect("a known opcode");
        expected.push(format!("begin op{0} tx{0} {command:?}", n + 1));
        expected.push(format!("end op{} Complete", n + 1));
    }
    assert_eq!(lines, expected);
    // Bytes after Write Enable are not part of it; the end covers them, through the release.
    let lines = decode_with_ticks(
        SpiNorConfig::default(),
        &[vec![(0x06, 0xff), (0x00, 0xff), (0x55, 0xff)]],
    );
    let word_len = 8 * 2 * HALF;
    let second_word = 10 + 2 * HALF + word_len;
    let release = 10 + HALF + 3 * word_len + HALF;
    assert_eq!(
        lines[1],
        format!("end op1 Complete [{second_word},{})@{release}", release + 1)
    );
}

#[test]
fn identification_is_reported_as_read() {
    let jedec: Vec<u8> = (0..40u8).map(|i| 0x80 | i).collect();
    let lines = decode(
        SpiNorConfig::default(),
        &[
            synth::read_manufacturer_device_id(0, &[0xef, 0x16]),
            synth::read_manufacturer_device_id(1, &[0x16, 0xef, 0x16, 0xef]),
            synth::read_id(0x9f, 0, &[0xef, 0x40, 0x17]),
            synth::read_id(0x9f, 0, &jedec),
            synth::read_id(0x4b, 4, &[1, 2, 3, 4, 5, 6, 7, 8, 9, 10]),
            synth::command(0xab),
            synth::read_id(0xab, 3, &[0x16, 0x16]),
        ],
    );
    assert_eq!(
        lines,
        [
            "begin op1 tx1 ReadManufacturerDeviceId".to_owned(),
            "address op1 000000".to_owned(),
            "id op1 ManufacturerDevice ef16".to_owned(),
            "end op1 Complete".to_owned(),
            // The flash repeats the pair as long as it is clocked; the id is the first two.
            "begin op2 tx2 ReadManufacturerDeviceId".to_owned(),
            "address op2 000001".to_owned(),
            "id op2 ManufacturerDevice 16ef".to_owned(),
            "end op2 Complete".to_owned(),
            "begin op3 tx3 ReadJedecId".to_owned(),
            "id op3 Jedec ef4017".to_owned(),
            "end op3 Complete".to_owned(),
            // A JEDEC ID is however many bytes the host reads, in batches.
            "begin op4 tx4 ReadJedecId".to_owned(),
            format!("id op4 Jedec {}", hex(&jedec[..32])),
            format!("id op4 Jedec {}", hex(&jedec[32..])),
            "end op4 Complete".to_owned(),
            "begin op5 tx5 ReadUniqueId".to_owned(),
            "dummy op5 4".to_owned(),
            "id op5 Unique 0102030405060708".to_owned(),
            "end op5 Complete".to_owned(),
            // Without bytes after it, 0xAB only releases the flash from deep power-down.
            "begin op6 tx6 ReleasePowerDown".to_owned(),
            "end op6 Complete".to_owned(),
            "begin op7 tx7 ReleasePowerDown".to_owned(),
            "dummy op7 3".to_owned(),
            "id op7 ElectronicSignature 16".to_owned(),
            "end op7 Complete".to_owned(),
        ]
    );
}

#[test]
fn a_fixed_id_goes_out_once_whole() {
    // The unique ID is reported at its eighth byte, before the transaction ends.
    let lines = decode_with_ticks(
        SpiNorConfig::default(),
        &[synth::read_id(0x4b, 4, &[1, 2, 3, 4, 5, 6, 7, 8, 9])],
    );
    let word_len = 8 * 2 * HALF;
    let first_bit = 10 + 2 * HALF;
    let start = first_bit + 5 * word_len;
    let last = first_bit + 12 * word_len + 7 * 2 * HALF;
    assert_eq!(
        lines[2],
        format!(
            "id op1 Unique 0102030405060708 [{start},{})@{last}",
            last + 1
        )
    );
}

#[test]
fn what_the_host_sends_must_be_whole() {
    let lines = decode(
        SpiNorConfig::default(),
        &[
            synth::program(0x02, 0x002000, 3, &[]),
            synth::write_status(0x01, &[]),
            synth::read(0x0b, 0x001000, 3, 0, &[]),
            synth::read_id(0xab, 0, &[0x00, 0x00]),
            synth::read_id(0x4b, 0, &[0x00, 0x00]),
            synth::addressed(0x13, 0x00ab_cdef, 3),
            synth::addressed(0x20, 0x00ab_cdef, 2),
        ],
    );
    assert_eq!(
        lines,
        [
            "begin op1 tx1 PageProgram",
            "address op1 002000",
            "end op1 Incomplete",
            "begin op2 tx2 WriteStatus",
            "end op2 Incomplete",
            "begin op3 tx3 FastRead",
            "address op3 001000",
            "end op3 Incomplete",
            "begin op4 tx4 ReleasePowerDown",
            "end op4 Incomplete",
            "begin op5 tx5 ReadUniqueId",
            "end op5 Incomplete",
            "begin op6 tx6 Read4b",
            "end op6 Incomplete",
            "begin op7 tx7 SectorErase",
            "end op7 Incomplete",
        ]
    );
}

#[test]
fn what_the_flash_sends_may_stop_early() {
    let lines = decode(
        SpiNorConfig::default(),
        &[
            synth::read_id(0x9f, 0, &[]),
            synth::read_manufacturer_device_id(0, &[0xef]),
            synth::read_id(0x4b, 4, &[1, 2, 3]),
            synth::read_id(0xab, 3, &[]),
            synth::read_status(0x05, &[]),
        ],
    );
    assert_eq!(
        lines,
        [
            "begin op1 tx1 ReadJedecId",
            "end op1 Complete",
            "begin op2 tx2 ReadManufacturerDeviceId",
            "address op2 000000",
            "id op2 ManufacturerDevice ef",
            "end op2 Complete",
            "begin op3 tx3 ReadUniqueId",
            "dummy op3 4",
            "id op3 Unique 010203",
            "end op3 Complete",
            "begin op4 tx4 ReleasePowerDown",
            "dummy op4 3",
            "end op4 Complete",
            "begin op5 tx5 ReadStatus",
            "end op5 Complete",
        ]
    );
}

#[test]
fn four_byte_commands_and_sfdp_keep_their_widths() {
    let lines = decode(
        SpiNorConfig::default(),
        &[
            synth::read(0x13, 0x0123_4567, 4, 0, &[0x11]),
            synth::read(0x0c, 0x0123_4567, 4, 1, &[0x22]),
        ],
    );
    assert_eq!(
        lines,
        [
            "begin op1 tx1 Read4b",
            "address op1 01234567",
            "data op1 out +0 11",
            "end op1 Complete",
            "begin op2 tx2 FastRead4b",
            "address op2 01234567",
            "dummy op2 1",
            "data op2 out +0 22",
            "end op2 Complete",
        ]
    );
    let four = SpiNorConfig {
        address_bytes: AddressBytes::Four,
    };
    let lines = decode(
        four,
        &[
            synth::read(0x03, 0x0123_4567, 4, 0, &[0x33]),
            synth::command(0xe9),
            // A flash kept in four-byte mode: Exit 4-Byte Address Mode changes nothing.
            synth::addressed(0x20, 0x0100_0000, 4),
            synth::read(0x5a, 0x000030, 3, 1, &[0x44]),
            synth::read_manufacturer_device_id(0, &[0xef, 0x18]),
        ],
    );
    assert_eq!(
        lines,
        [
            "begin op1 tx1 Read",
            "address op1 01234567",
            "data op1 out +0 33",
            "end op1 Complete",
            "begin op2 tx2 Exit4bMode",
            "end op2 Complete",
            "begin op3 tx3 SectorErase",
            "address op3 01000000",
            "end op3 Complete",
            "begin op4 tx4 ReadSfdp",
            "address op4 000030",
            "dummy op4 1",
            "data op4 out +0 44",
            "end op4 Complete",
            "begin op5 tx5 ReadManufacturerDeviceId",
            "address op5 000000",
            "id op5 ManufacturerDevice ef18",
            "end op5 Complete",
        ]
    );
}

#[test]
fn auto_follows_enter_and_exit_four_byte_mode() {
    let lines = decode(
        auto(),
        &[
            synth::read(0x03, 0x123456, 3, 0, &[0x01]),
            synth::command(0xb7),
            synth::read(0x03, 0x1234_5678, 4, 0, &[0x02]),
            synth::program(0x02, 0x0100_0000, 4, &[0x03]),
            synth::read(0x0b, 0x0100_0000, 4, 1, &[0x04]),
            synth::command(0xe9),
            synth::read(0x03, 0x123456, 3, 0, &[0x05]),
        ],
    );
    assert_eq!(
        lines,
        [
            "begin op1 tx1 Read",
            "address op1 123456",
            "data op1 out +0 01",
            "end op1 Complete",
            "begin op2 tx2 Enter4bMode",
            "end op2 Complete",
            "begin op3 tx3 Read",
            "address op3 12345678",
            "data op3 out +0 02",
            "end op3 Complete",
            "begin op4 tx4 PageProgram",
            "address op4 01000000",
            "data op4 in +0 03",
            "end op4 Complete",
            "begin op5 tx5 FastRead",
            "address op5 01000000",
            "dummy op5 1",
            "data op5 out +0 04",
            "end op5 Complete",
            "begin op6 tx6 Exit4bMode",
            "end op6 Complete",
            "begin op7 tx7 Read",
            "address op7 123456",
            "data op7 out +0 05",
            "end op7 Complete",
        ]
    );
}

#[test]
fn auto_falls_back_to_three_bytes_after_a_gap() {
    let mut waveform = waveform();
    waveform.transaction(synth::command(0xb7));
    waveform.idle(10);
    let gap_start = waveform.position();
    waveform.idle(20);
    let gap = TickRange::new(gap_start, waveform.position());
    waveform.idle(10);
    waveform.transaction(synth::read(0x03, 0x123456, 3, 0, &[0x01]));
    waveform.idle(10);
    let block = waveform.finish();
    let mut harness = Harness::nor_with(SpiConfig::default(), auto());
    harness.feed_with_gap(&block, gap, 64);
    harness.end();
    assert_eq!(
        bare(&harness.nor_lines()),
        [
            "begin op1 tx1 Enter4bMode",
            "end op1 Complete",
            "gap",
            "begin op2 tx2 Read",
            "address op2 123456",
            "data op2 out +0 01",
            "end op2 Complete",
        ]
    );
    // Without the gap the same read takes four address bytes: the host's byte of the first
    // data word completes the address.
    let mut harness = Harness::nor_with(SpiConfig::default(), auto());
    harness.run_all(&block);
    assert_eq!(bare(&harness.nor_lines())[3], "address op2 12345600");
}

#[test]
fn a_software_reset_returns_auto_to_three_bytes() {
    let lines = decode(
        auto(),
        &[
            synth::command(0xb7),
            synth::command(0x66),
            synth::command(0x99),
            synth::addressed(0x20, 0x003000, 3),
            synth::command(0xb7),
            // Another command between Reset Enable and Reset disarms the reset.
            synth::command(0x66),
            synth::command(0x06),
            synth::command(0x99),
            synth::addressed(0x20, 0x0100_3000, 4),
        ],
    );
    assert_eq!(lines[6], "begin op4 tx4 SectorErase");
    assert_eq!(lines[7], "address op4 003000");
    assert_eq!(lines[17], "begin op9 tx9 SectorErase");
    assert_eq!(lines[18], "address op9 01003000");
}

#[test]
fn dual_and_quad_commands_are_unknown() {
    let opcodes = [0x3b, 0x6b, 0xbb, 0xeb, 0x32, 0x38, 0x92, 0x94];
    let transactions: Vec<Vec<(u8, u8)>> = opcodes
        .iter()
        .map(|opcode| synth::read(*opcode, 0x001000, 3, 1, &[0xaa, 0xbb]))
        .collect();
    let lines = decode(SpiNorConfig::default(), &transactions);
    let expected: Vec<String> = opcodes
        .iter()
        .enumerate()
        .map(|(n, opcode)| format!("unknown tx{} {opcode:02x}", n + 1))
        .collect();
    assert_eq!(lines, expected);
}

#[test]
fn a_program_cut_off_by_the_end_of_the_samples_is_aborted() {
    let mut waveform = waveform();
    waveform.select();
    waveform.words(synth::program(0x02, 0x000100, 3, &[0x10, 0x20, 0x30]));
    waveform.partial_word(0x40, 0xff, 5);
    let block = waveform.finish();
    let mut harness = Harness::nor(SpiConfig::default());
    harness.run_all(&block);
    let lines = harness.nor_lines();
    assert!(lines[2].starts_with("data op1 in +0 102030 ["), "{lines:?}");
    assert!(
        lines[2].ends_with(&format!(")@{}", block.end())),
        "{lines:?}"
    );
    assert_eq!(lines[3], format!("end op1 Aborted @{}", block.end()));
    assert_eq!(lines[4], format!("eof {}", block.end()));
}

#[test]
fn a_status_poll_cut_by_a_gap_is_aborted() {
    let mut waveform = waveform();
    waveform.transaction(synth::read_status(0x05, &[0x03, 0x03, 0x03, 0x02]));
    waveform.idle(10);
    let block = waveform.finish();
    let word_len = 8 * 2 * HALF;
    let gap_start = 10 + 2 * HALF + 3 * word_len + 4;
    let gap = TickRange::new(Tick(gap_start), Tick(gap_start + 5));
    let mut harness = Harness::nor(SpiConfig::default());
    harness.feed_with_gap(&block, gap, 17);
    harness.end();
    let lines = harness.nor_lines();
    assert_eq!(
        bare(&lines),
        [
            "begin op1 tx1 ReadStatus",
            "status op1 read sr1 03",
            "status op1 read sr1 03",
            "end op1 Aborted",
            "gap",
        ]
    );
    assert_eq!(lines[3], format!("end op1 Aborted @{gap_start}"));
}

/// Every command family, some cut short, some unknown, in a four-byte mode that comes and
/// goes.
fn mixed_traffic(waveform: &mut SpiWaveform) {
    let page: Vec<u8> = (0..70u8).map(|i| i.wrapping_mul(37)).collect();
    let transactions = [
        synth::command(0x06),
        synth::program(0x02, 0x001000, 3, &page),
        synth::read_status(0x05, &[0x03, 0x03, 0x01, 0x00]),
        synth::read(0x03, 0x001000, 3, 0, &page[..45]),
        synth::read(0x0b, 0x001010, 3, 1, &page[..9]),
        synth::command(0xb7),
        synth::read(0x03, 0x0100_1000, 4, 0, &page[..33]),
        synth::addressed(0x20, 0x0100_0000, 4),
        synth::read(0x5a, 0x000000, 3, 1, b"SFDP\x06\x01"),
        synth::write_status(0x01, &[0x00, 0x02]),
        synth::read_id(0x9f, 0, &[0xef, 0x40, 0x19]),
        synth::read(0xeb, 0x001000, 3, 3, &[0x5a; 6]),
        synth::read_id(0x4b, 4, &[9, 8, 7, 6, 5, 4, 3, 2]),
        synth::command(0x66),
        synth::command(0x99),
        synth::read_manufacturer_device_id(0, &[0xef, 0x18, 0xef]),
        synth::program(0x12, 0x0100_0100, 4, &page[..3]),
        synth::read_id(0xab, 3, &[0x18]),
        synth::read(0x03, 0x002000, 3, 0, &[]),
        synth::program(0x02, 0x002000, 3, &[]),
        synth::read(0x13, 0x0100_2000, 4, 0, &page[..40]),
    ];
    for (n, words) in transactions.iter().enumerate() {
        waveform.transaction(words.iter().copied());
        waveform.idle(5 + (n as u64 % 4) * 7);
    }
}

#[test]
fn a_long_mixed_waveform_decodes_alike_however_it_is_cut() {
    let mut waveform = waveform();
    mixed_traffic(&mut waveform);
    let block = waveform.finish();
    // One gap inside the first program's data, one later on.
    let gaps = [
        TickRange::new(Tick(block.len() / 9), Tick(block.len() / 9 + 31)),
        TickRange::new(Tick(block.len() / 2), Tick(block.len() / 2 + 300)),
    ];
    let build = |graph_config| {
        uiharu_core::testing::Harness::build(
            CHANNELS,
            TimeBase::new(RATE),
            graph_config,
            |graph, source| {
                let spi = graph.add(Spi::new(SpiConfig::default()).expect("a valid configuration"));
                graph.connect(source.samples, Spi::input(spi))?;
                let nor = graph.add(SpiNor::new(auto()).expect("a valid configuration"));
                graph.connect(Spi::events(spi), SpiNor::input(nor))?;
                Ok(SpiNor::events(nor))
            },
        )
        .expect("builds")
    };
    let mut reference = build(GraphConfig::default());
    reference.feed_with_gaps(&block, &gaps, &[u64::MAX]);
    reference.end();
    assert!(
        reference.events().len() > 60,
        "{} events",
        reference.events().len()
    );
    assert!(
        reference
            .without_progress()
            .iter()
            .any(|item| matches!(item, uiharu_core::Item::Gap(_)))
    );
    uiharu_core::testing::assert_block_invariant(build, &block, &gaps);
}

#[test]
fn progress_moves_inside_a_long_read() {
    let bytes = data(240);
    let mut waveform = waveform();
    waveform.transaction(nor_read_words(0x000100, &bytes));
    waveform.idle(10);
    let block = waveform.finish();
    let mut harness = Harness::nor(SpiConfig::default());
    harness.feed_sizes(&block, &[64]);
    harness.end();
    let items = harness.nor_items();
    let address = items
        .iter()
        .position(|item| matches!(item, Item::Event(event) if matches!(event.payload, NorEvent::Address { .. })))
        .expect("the address is reported");
    let end = items
        .iter()
        .rposition(|item| matches!(item, Item::Event(event) if matches!(event.payload, NorEvent::End { .. })))
        .expect("the end is reported");
    // The data is read in batches of 32 bytes: while a batch fills, the cursor passes the
    // progress of every block on held back to the batch's first byte, so the watermark moves
    // once per batch instead of standing still for the whole read.
    let inside: Vec<Tick> = items[address..end]
        .iter()
        .filter_map(|item| match item {
            Item::Progress(tick) => Some(*tick),
            _ => None,
        })
        .collect();
    assert!(inside.len() >= 240 / 32 - 1, "{inside:?}");
    assert!(
        inside.windows(2).all(|pair| pair[0] <= pair[1]),
        "{inside:?}"
    );
    assert_progress_promise(&items);
}
