// SPDX-License-Identifier: Apache-2.0

//! The I²C decoder on synthesized waveforms.
//!
//! The waveforms run at two samples per quarter of the SCL period: a byte with its acknowledge
//! bit takes 72 samples, its first bit is taken 2 samples after the byte starts and its
//! acknowledge bit 66 samples after; a START from a free bus leaves 4 samples before the first
//! byte, a repeated START and a STOP change SDA 4 samples after the byte before them ended.

use std::sync::Arc;

use proptest::prelude::*;
use uiharu_core::testing::{self, Harness};
use uiharu_core::{
    EventSchema, GraphConfig, InputRef, InputSpec, Io, Item, Levels, LogicBlock, LogicBlockBuilder,
    Node, NodeFuture, NodeId, NodeSpec, Outbox, OutputRef, OutputSpec, Props, Requires, SchemaId,
    SignalItem, Tick, TickRange, TimeBase,
};
use uiharu_protocols::i2c::synth::{
    I2cWaveform, Symbol, read_7bit, read_10bit, write_7bit, write_10bit,
};
use uiharu_protocols::i2c::{EndReason, I2c, I2cBus, I2cConfig, I2cEvent, Next, Read, TxId};

const CHANNELS: u8 = 2;
const Q: u64 = 2;

fn build(config: GraphConfig) -> Harness<I2cEvent> {
    Harness::build(
        CHANNELS,
        TimeBase::new(1_000_000),
        config,
        |graph, source| {
            let i2c = graph.add(I2c::new(I2cConfig::default()).expect("a valid configuration"));
            graph.connect(source.samples, I2c::input(i2c))?;
            Ok(I2c::events(i2c))
        },
    )
    .expect("builds")
}

fn new_waveform() -> I2cWaveform {
    let mut waveform = I2cWaveform::new(I2cConfig::default(), CHANNELS, Q, Tick::ZERO);
    waveform.idle(10);
    waveform
}

fn byte(value: u8, ack: bool) -> Symbol {
    Symbol::Byte { value, ack }
}

/// The events of `block` decoded in one piece, as lines.
fn decode(block: &LogicBlock) -> Vec<String> {
    let mut harness = build(GraphConfig::default());
    harness.run_all(block);
    describe(&harness.without_progress())
}

fn at(span: TickRange, settled: Tick) -> String {
    if span.is_empty() {
        format!("@{settled}")
    } else {
        format!("[{},{})@{settled}", span.start, span.end)
    }
}

fn ack(ack: bool) -> &'static str {
    if ack { "ACK" } else { "NACK" }
}

fn dropped(bits: u8) -> String {
    if bits == 0 {
        String::new()
    } else {
        format!(" dropped {bits}")
    }
}

/// One line per item, readable in assertions.
fn describe(items: &[Item<I2cEvent>]) -> Vec<String> {
    items
        .iter()
        .map(|item| match item {
            Item::Event(event) => {
                let what = match event.payload {
                    I2cEvent::Begin { tx, partial } => {
                        format!("begin tx{}{}", tx.0, if partial { " partial" } else { "" })
                    }
                    I2cEvent::Address {
                        tx,
                        address,
                        ten_bit,
                        read,
                        ack: acked,
                    } => {
                        let address = if ten_bit {
                            format!("10:{address:03x}")
                        } else {
                            format!("{address:02x}")
                        };
                        let direction = if read { "R" } else { "W" };
                        format!("address tx{} {address} {direction} {}", tx.0, ack(acked))
                    }
                    I2cEvent::Data {
                        tx,
                        byte,
                        ack: acked,
                    } => format!("data tx{} {byte:02x} {}", tx.0, ack(acked)),
                    I2cEvent::Restart { tx, dropped_bits } => {
                        format!("restart tx{}{}", tx.0, dropped(dropped_bits))
                    }
                    I2cEvent::End {
                        tx,
                        reason,
                        dropped_bits,
                    } => format!("end tx{} {reason:?}{}", tx.0, dropped(dropped_bits)),
                };
                format!("{what} {}", at(event.span, event.settled))
            }
            Item::Gap(range) => format!("gap {range}"),
            Item::Progress(tick) => format!("progress {tick}"),
            Item::End(tick) => format!("eof {tick}"),
        })
        .collect()
}

/// The span and settled tick of a byte whose first bit cell starts at `start`: its first
/// rising edge through its acknowledge edge.
fn byte_at(start: u64) -> String {
    let (first, ack) = (start + Q, start + 33 * Q);
    format!("[{first},{})@{ack}", ack + 1)
}

/// A condition at `tick`.
fn one(tick: u64) -> String {
    format!("[{tick},{})@{tick}", tick + 1)
}

/// Checks that no event settles earlier than the one before it.
fn assert_settled_in_order(items: &[Item<I2cEvent>]) {
    let mut last = Tick::ZERO;
    for event in items.iter().filter_map(Item::event) {
        assert!(event.settled >= last, "{event:?} settles before {last}");
        last = event.settled;
    }
}

#[test]
fn decodes_a_seven_bit_write_with_exact_spans() {
    let mut waveform = new_waveform();
    waveform.transaction([byte(0xa0, true), byte(0x12, true), byte(0x34, false)]);
    waveform.idle(10);
    let lines = decode(&waveform.finish());
    // START at 10; the bytes start at 14, 86 and 158; the STOP sequence starts at 230 and
    // raises SDA at 234.
    assert_eq!(
        lines,
        vec![
            "begin tx1 [10,11)@10",
            "address tx1 50 W ACK [16,81)@80",
            "data tx1 12 ACK [88,153)@152",
            "data tx1 34 NACK [160,225)@224",
            "end tx1 Stop [234,235)@234",
            "eof 246",
        ]
    );
}

#[test]
fn decodes_a_seven_bit_read() {
    let mut waveform = new_waveform();
    waveform.transaction(read_7bit(0x68, &[0xab, 0xcd]));
    waveform.idle(10);
    let lines = decode(&waveform.finish());
    assert_eq!(
        lines,
        vec![
            "begin tx1 [10,11)@10".to_owned(),
            format!("address tx1 68 R ACK {}", byte_at(14)),
            format!("data tx1 ab ACK {}", byte_at(86)),
            format!("data tx1 cd NACK {}", byte_at(158)),
            format!("end tx1 Stop {}", one(234)),
            "eof 246".to_owned(),
        ]
    );
}

#[test]
fn a_nacked_address_is_reported_as_such() {
    let mut waveform = new_waveform();
    waveform.transaction([byte(0xa2, false)]);
    waveform.idle(10);
    assert_eq!(
        decode(&waveform.finish()),
        vec![
            "begin tx1 [10,11)@10".to_owned(),
            format!("address tx1 51 W NACK {}", byte_at(14)),
            format!("end tx1 Stop {}", one(90)),
            "eof 102".to_owned(),
        ]
    );
}

#[test]
fn a_start_and_a_stop_alone_make_an_empty_transaction() {
    let mut waveform = new_waveform();
    waveform.transaction([]);
    waveform.idle(10);
    // The STOP's own clock rises at 16; it is no bit.
    assert_eq!(
        decode(&waveform.finish()),
        vec!["begin tx1 [10,11)@10", "end tx1 Stop [18,19)@18", "eof 30"]
    );
}

#[test]
fn a_ten_bit_write_address_spans_both_its_bytes() {
    let mut waveform = new_waveform();
    waveform.transaction(write_10bit(0x2a5, &[0x77]));
    waveform.idle(10);
    assert_eq!(
        decode(&waveform.finish()),
        vec![
            "begin tx1 [10,11)@10".to_owned(),
            "address tx1 10:2a5 W ACK [16,153)@152".to_owned(),
            format!("data tx1 77 ACK {}", byte_at(158)),
            format!("end tx1 Stop {}", one(234)),
            "eof 246".to_owned(),
        ]
    );
}

#[test]
fn a_ten_bit_read_refers_back_to_its_write_address() {
    let mut waveform = new_waveform();
    waveform.transaction(read_10bit(0x2a5, &[0x01, 0x02]));
    waveform.idle(10);
    // The repeated START follows the second address byte (ending at 158) and lowers SDA at
    // 162; the read address byte starts at 166.
    assert_eq!(
        decode(&waveform.finish()),
        vec![
            "begin tx1 [10,11)@10".to_owned(),
            "address tx1 10:2a5 W ACK [16,153)@152".to_owned(),
            format!("restart tx1 {}", one(162)),
            format!("address tx1 10:2a5 R ACK {}", byte_at(166)),
            format!("data tx1 01 ACK {}", byte_at(238)),
            format!("data tx1 02 NACK {}", byte_at(310)),
            format!("end tx1 Stop {}", one(386)),
            "eof 398".to_owned(),
        ]
    );
}

#[test]
fn a_ten_bit_pattern_without_its_other_part_is_a_seven_bit_address() {
    // The first byte of a 10-bit write address, then a STOP: what it addressed is not known,
    // so it is the reserved 7-bit address it is, certain at the STOP.
    let mut waveform = new_waveform();
    waveform.transaction([byte(0xf4, true)]);
    waveform.idle(10);
    assert_eq!(
        decode(&waveform.finish()),
        vec![
            "begin tx1 [10,11)@10",
            "address tx1 7a W ACK [16,81)@90",
            "end tx1 Stop [90,91)@90",
            "eof 102",
        ]
    );
    // A 10-bit read with no write address before it.
    let mut waveform = new_waveform();
    waveform.transaction([byte(0xf5, true), byte(0x55, false)]);
    waveform.idle(10);
    let lines = decode(&waveform.finish());
    assert_eq!(lines[1], format!("address tx1 7a R ACK {}", byte_at(14)));
    assert_eq!(lines[2], format!("data tx1 55 NACK {}", byte_at(86)));
    // A 10-bit read whose high bits differ from the write address before it.
    let mut waveform = new_waveform();
    let mut symbols = write_10bit(0x1a5, &[]);
    symbols.extend([Symbol::Restart, byte(0xf5, true), byte(0x66, false)]);
    waveform.transaction(symbols);
    waveform.idle(10);
    let lines = decode(&waveform.finish());
    assert!(
        lines[1].starts_with("address tx1 10:1a5 W ACK "),
        "{lines:?}"
    );
    assert!(lines[2].starts_with("restart tx1 "), "{lines:?}");
    assert!(lines[3].starts_with("address tx1 7a R ACK "), "{lines:?}");
    // A repeated START right after the first byte of a 10-bit write address.
    let mut waveform = new_waveform();
    waveform.transaction([byte(0xf2, false), Symbol::Restart, byte(0xa1, true)]);
    waveform.idle(10);
    let lines = decode(&waveform.finish());
    assert_eq!(lines[1], "address tx1 79 W NACK [16,81)@90");
    assert_eq!(lines[2], format!("restart tx1 {}", one(90)));
    assert!(lines[3].starts_with("address tx1 50 R ACK "), "{lines:?}");
}

#[test]
fn a_repeated_start_stays_in_the_transaction() {
    let mut waveform = new_waveform();
    let mut symbols = write_7bit(0x50, &[0x00]);
    symbols.push(Symbol::Restart);
    symbols.extend(read_7bit(0x50, &[0x42]));
    waveform.transaction(symbols);
    waveform.idle(10);
    assert_eq!(
        decode(&waveform.finish()),
        vec![
            "begin tx1 [10,11)@10".to_owned(),
            format!("address tx1 50 W ACK {}", byte_at(14)),
            format!("data tx1 00 ACK {}", byte_at(86)),
            format!("restart tx1 {}", one(162)),
            format!("address tx1 50 R ACK {}", byte_at(166)),
            format!("data tx1 42 NACK {}", byte_at(238)),
            format!("end tx1 Stop {}", one(314)),
            "eof 326".to_owned(),
        ]
    );
}

#[test]
fn reserved_addresses_are_addresses() {
    // A general call, then the START byte procedure: `0000 0001`, not acknowledged, and a
    // repeated START.
    let mut waveform = new_waveform();
    waveform.transaction([byte(0x00, true), byte(0x06, true)]);
    waveform.idle(10);
    waveform.transaction([
        byte(0x01, false),
        Symbol::Restart,
        byte(0xa1, true),
        byte(0x10, false),
    ]);
    waveform.idle(10);
    let lines = decode(&waveform.finish());
    assert!(lines[1].starts_with("address tx1 00 W ACK "), "{lines:?}");
    assert!(lines[2].starts_with("data tx1 06 ACK "), "{lines:?}");
    assert!(lines[5].starts_with("address tx2 00 R NACK "), "{lines:?}");
    assert!(lines[6].starts_with("restart tx2 "), "{lines:?}");
    assert!(lines[7].starts_with("address tx2 50 R ACK "), "{lines:?}");
    assert!(lines[8].starts_with("data tx2 10 NACK "), "{lines:?}");
    assert!(lines[9].starts_with("end tx2 Stop "), "{lines:?}");
}

#[test]
fn clock_stretching_only_moves_the_edges() {
    let mut waveform = new_waveform();
    waveform.start();
    // The target holds SCL low for 25 samples before acknowledging the address, and for 40
    // after it.
    waveform.byte_stretched(0xa0, true, 25);
    waveform.idle(40);
    waveform.byte(0x12, true);
    waveform.stop();
    waveform.idle(10);
    // The address byte's eight bits end at 78, the stretch at 103; its acknowledge bit rises
    // at 105 and ends at 111; the data byte starts at 151.
    assert_eq!(
        decode(&waveform.finish()),
        vec![
            "begin tx1 [10,11)@10".to_owned(),
            "address tx1 50 W ACK [16,106)@105".to_owned(),
            format!("data tx1 12 ACK {}", byte_at(151)),
            format!("end tx1 Stop {}", one(227)),
            "eof 239".to_owned(),
        ]
    );
}

#[test]
fn bits_cut_short_by_a_condition_make_no_byte() {
    // Three bits, then a STOP: the STOP's own clock is no fourth bit.
    let mut waveform = new_waveform();
    waveform.start();
    waveform.byte(0xa0, true);
    waveform.bits(0x5a, 3);
    waveform.stop();
    waveform.idle(10);
    assert_eq!(
        decode(&waveform.finish()),
        vec![
            "begin tx1 [10,11)@10".to_owned(),
            format!("address tx1 50 W ACK {}", byte_at(14)),
            format!("end tx1 Stop dropped 3 {}", one(114)),
            "eof 126".to_owned(),
        ]
    );
    // Five bits, then a repeated START.
    let mut waveform = new_waveform();
    waveform.start();
    waveform.byte(0xa0, true);
    waveform.bits(0xff, 5);
    waveform.restart();
    waveform.byte(0xa1, true);
    waveform.byte(0x42, false);
    waveform.stop();
    waveform.idle(10);
    assert_eq!(
        decode(&waveform.finish()),
        vec![
            "begin tx1 [10,11)@10".to_owned(),
            format!("address tx1 50 W ACK {}", byte_at(14)),
            format!("restart tx1 dropped 5 {}", one(130)),
            format!("address tx1 50 R ACK {}", byte_at(134)),
            format!("data tx1 42 NACK {}", byte_at(206)),
            format!("end tx1 Stop {}", one(282)),
            "eof 294".to_owned(),
        ]
    );
}

/// A write of three bytes (START at 10, STOP at 306) and a read (START at 318, STOP at 470).
fn two_transactions() -> LogicBlock {
    let mut waveform = new_waveform();
    waveform.transaction(write_7bit(0x50, &[1, 2, 3]));
    waveform.idle(10);
    waveform.transaction(read_7bit(0x51, &[9]));
    waveform.idle(10);
    waveform.finish()
}

fn the_read(tx: u64) -> Vec<String> {
    vec![
        format!("begin tx{tx} {}", one(318)),
        format!("address tx{tx} 51 R ACK {}", byte_at(322)),
        format!("data tx{tx} 09 NACK {}", byte_at(394)),
        format!("end tx{tx} Stop {}", one(470)),
        "eof 482".to_owned(),
    ]
}

#[test]
fn a_busy_bus_at_the_first_sample_begins_a_partial_transaction() {
    let block = two_transactions();
    // At 100 the second byte is under way with SCL low: the transaction is found busy.
    let tail = block
        .slice(TickRange::new(Tick(100), block.end()))
        .expect("a slice");
    let mut expected = vec![
        "begin tx1 partial [100,101)@100".to_owned(),
        format!("end tx1 Stop {}", one(306)),
    ];
    expected.extend(the_read(2));
    assert_eq!(decode(&tail), expected);
}

#[test]
fn clocks_before_any_start_begin_a_partial_transaction() {
    let block = two_transactions();
    // At 17 the address's first bit is a one with SCL high: the bus looks free until SCL
    // falls at 20.
    let tail = block
        .slice(TickRange::new(Tick(17), block.end()))
        .expect("a slice");
    let mut expected = vec![
        "begin tx1 partial [20,21)@20".to_owned(),
        format!("end tx1 Stop {}", one(306)),
    ];
    expected.extend(the_read(2));
    assert_eq!(decode(&tail), expected);
}

#[test]
fn a_start_ends_a_partial_transaction_and_begins_the_next() {
    let mut waveform = new_waveform();
    let mut symbols = write_7bit(0x50, &[0x00]);
    symbols.push(Symbol::Restart);
    symbols.extend(read_7bit(0x50, &[0x42]));
    waveform.transaction(symbols);
    waveform.idle(10);
    let block = waveform.finish();
    let tail = block
        .slice(TickRange::new(Tick(100), block.end()))
        .expect("a slice");
    assert_eq!(
        decode(&tail),
        vec![
            "begin tx1 partial [100,101)@100".to_owned(),
            format!("end tx1 Restart {}", one(162)),
            format!("begin tx2 {}", one(162)),
            format!("address tx2 50 R ACK {}", byte_at(166)),
            format!("data tx2 42 NACK {}", byte_at(238)),
            format!("end tx2 Stop {}", one(314)),
            "eof 326".to_owned(),
        ]
    );
}

#[test]
fn a_gap_aborts_the_transaction_and_resyncs() {
    let block = two_transactions();
    // The gap starts as the second byte's fifth bit is set up (four bits taken, SCL low) and
    // ends as the third byte's sixth bit rises, SDA low: the bus is busy after it.
    let gap = TickRange::new(Tick(120), Tick(200));
    let mut harness = build(GraphConfig::default());
    harness.feed_with_gaps(&block, &[gap], &[33]);
    harness.end();
    let mut expected = vec![
        "begin tx1 [10,11)@10".to_owned(),
        format!("address tx1 50 W ACK {}", byte_at(14)),
        "end tx1 Aborted dropped 4 @120".to_owned(),
        "gap 120..200".to_owned(),
        "begin tx2 partial [200,201)@200".to_owned(),
        format!("end tx2 Stop {}", one(306)),
    ];
    expected.extend(the_read(3));
    assert_eq!(describe(&harness.without_progress()), expected);
}

#[test]
fn the_end_of_the_samples_aborts_an_open_transaction() {
    let block = two_transactions();
    // Cut while the second byte's fourth bit has SCL high: three bits are settled.
    let head = block
        .slice(TickRange::new(Tick::ZERO, Tick(114)))
        .expect("a slice");
    assert_eq!(
        decode(&head),
        vec![
            "begin tx1 [10,11)@10".to_owned(),
            format!("address tx1 50 W ACK {}", byte_at(14)),
            "end tx1 Aborted dropped 3 @114".to_owned(),
            "eof 114".to_owned(),
        ]
    );
    // A 10-bit write address cut off after its first byte.
    let mut waveform = new_waveform();
    waveform.start();
    waveform.byte(0xf6, true);
    waveform.bits(0x12, 2);
    let lines = decode(&waveform.finish());
    assert_eq!(
        lines,
        vec![
            "begin tx1 [10,11)@10",
            "address tx1 7b W ACK [16,81)@102",
            "end tx1 Aborted dropped 2 @102",
            "eof 102",
        ]
    );
}

#[test]
fn an_sda_change_on_a_clock_edge_is_data() {
    // SCL on channel 0, SDA on channel 1. After a START, every clock edge changes SDA at the
    // same tick: rising edges take SDA's new level, and no change is a START or STOP.
    let mut builder = LogicBlockBuilder::new(Tick::ZERO, CHANNELS);
    builder.push_repeated(Levels(0b11), 4);
    builder.push_repeated(Levels(0b01), 2); // START at 4
    builder.push_repeated(Levels(0b00), 2);
    for bit in [1u64, 0, 1, 0, 0, 1, 0, 1, 0] {
        builder.push_repeated(Levels(1 | (bit << 1)), 2); // SCL rises as SDA takes the bit
        builder.push_repeated(Levels((1 - bit) << 1), 2); // SCL falls as SDA flips
    }
    builder.push_repeated(Levels(0b00), 2);
    builder.push_repeated(Levels(0b01), 2);
    builder.push_repeated(Levels(0b11), 2); // STOP at 48
    let lines = decode(&builder.finish().expect("a block"));
    assert_eq!(
        lines,
        vec![
            "begin tx1 [4,5)@4",
            "address tx1 52 R ACK [8,41)@40",
            "end tx1 Stop [48,49)@48",
            "eof 50",
        ]
    );
}

#[test]
fn bytes_show_up_before_the_transaction_ends() {
    let data: Vec<u8> = (0..20).collect();
    let mut waveform = new_waveform();
    waveform.transaction(write_7bit(0x50, &data));
    waveform.idle(10);
    let block = waveform.finish();
    // Up to the middle of the sixth data byte.
    let cut = Tick(14 + 6 * 72 + 30);
    let head = block
        .slice(TickRange::new(Tick::ZERO, cut))
        .expect("a slice");
    let mut harness = build(GraphConfig::default());
    harness.feed(&head, &[17]);
    harness.collect();
    let lines = describe(&harness.without_progress());
    assert_eq!(lines.len(), 7, "{lines:?}");
    assert!(lines[6].starts_with("data tx1 04 ACK "), "{lines:?}");
    let tail = block
        .slice(TickRange::new(cut, block.end()))
        .expect("a slice");
    harness.feed(&tail, &[17]);
    harness.end();
    let lines = describe(&harness.without_progress());
    assert_eq!(lines.len(), 24, "{lines:?}");
    assert!(lines[22].starts_with("end tx1 Stop "), "{lines:?}");
}

/// Traffic of every shape the decoder knows, with stretching and bytes cut short.
fn rich_waveform() -> LogicBlock {
    let mut waveform = new_waveform();
    waveform.transaction(write_7bit(0x50, &[0x00, 0x10, 0xff]));
    waveform.idle(7);
    waveform.transaction(read_10bit(0x3c1, &[0xaa, 0x55, 0x0f]));
    waveform.idle(3);
    waveform.start();
    waveform.byte_stretched(0xa1, true, 19);
    waveform.idle(23);
    waveform.byte(0x80, true);
    waveform.byte_stretched(0x7e, false, 5);
    waveform.stop();
    waveform.idle(11);
    waveform.transaction([byte(0x01, false), Symbol::Restart, byte(0xf4, true)]);
    waveform.idle(5);
    waveform.start();
    waveform.byte(0xa0, true);
    waveform.bits(0xc3, 6);
    waveform.restart();
    waveform.byte(0x00, false);
    waveform.bits(0x3c, 4);
    waveform.stop();
    waveform.idle(9);
    waveform.transaction(write_10bit(0x0ff, &[1, 2]));
    waveform.idle(13);
    waveform.transaction([]);
    waveform.idle(4);
    waveform.finish()
}

#[test]
fn every_cut_and_pause_yields_the_same_events() {
    let block = rich_waveform();
    // A gap inside the first write's last byte, one on the free bus after the STOP at 1212,
    // and one over the STOP at 1459 and the START at 1470.
    let gaps = [
        TickRange::new(Tick(256), Tick(301)),
        TickRange::new(Tick(1214), Tick(1217)),
        TickRange::new(Tick(1436), Tick(1537)),
    ];
    let mut harness = build(GraphConfig::default());
    harness.feed_with_gaps(&block, &gaps, &[u64::MAX]);
    harness.end();
    let items = harness.without_progress();
    assert_settled_in_order(&items);
    let lines = describe(&items);
    // The bus is busy after the first and the last gap, free after the second.
    for (gap, after) in [
        ("gap 256..301", "begin tx2 partial "),
        ("gap 1214..1217", "begin tx6 [1219,"),
        ("gap 1436..1537", "begin tx7 partial "),
    ] {
        let at = lines
            .iter()
            .position(|line| line == gap)
            .unwrap_or_else(|| panic!("{gap} in {lines:?}"));
        assert!(lines[at + 1].starts_with(after), "{lines:?}");
    }
    assert_eq!(lines.len(), 38, "{lines:?}");
    testing::assert_block_invariant(build, &block, &gaps);
}

#[test]
fn progress_is_passed_on() {
    let mut harness = build(GraphConfig::default());
    let mut builder = LogicBlockBuilder::new(Tick::ZERO, CHANNELS);
    builder.push_repeated(Levels(0b11), 100);
    harness.push(SignalItem::Data(Arc::new(
        builder.finish().expect("a block"),
    )));
    harness.push(SignalItem::Progress(Tick(100)));
    harness.collect();
    assert_eq!(harness.items, vec![Item::Progress(Tick(100))]);
}

#[test]
fn progress_is_held_back_to_the_byte_under_way() {
    let mut waveform = new_waveform();
    waveform.transaction(write_10bit(0x2a5, &[0x77]));
    waveform.idle(10);
    let block = waveform.finish();
    let mut harness = build(GraphConfig::default());
    let mut from = Tick::ZERO;
    // The address spans both its bytes, [16,153); the data byte's first bit rises at 160.
    for (cut, held) in [(50, 16), (120, 16), (153, 153), (190, 160)] {
        let piece = block
            .slice(TickRange::new(from, Tick(cut)))
            .expect("a slice");
        harness.feed(&piece, &[u64::MAX]);
        harness.collect();
        assert_eq!(
            harness.items.last(),
            Some(&Item::Progress(Tick(held))),
            "up to {cut}"
        );
        from = Tick(cut);
    }
    harness.assert_progress_promise();
}

/// What a reader on the transaction cursor found, one line per read.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Seen(String);

impl EventSchema for Seen {
    const SCHEMA: SchemaId = SchemaId::new("test.seen", 1);
}

/// A decoder built on I²C that reads every transaction through the cursor and says what each
/// read found: the first `strict` reads of a transaction with `event()`, the others with
/// `event_or_end()`, until the transaction ends.
struct Reader {
    strict: usize,
}

impl Reader {
    fn seen(node: NodeId) -> OutputRef<Seen> {
        OutputRef::of_node(node, 0)
    }
}

impl Node for Reader {
    fn spec(&self) -> NodeSpec {
        NodeSpec {
            name: "reader".into(),
            inputs: vec![InputSpec::new::<I2cEvent>(
                "bus",
                Requires::new().equals("framing", "start-stop"),
            )],
            outputs: vec![OutputSpec::new::<Seen>("seen", Props::new())],
        }
    }

    fn start(self: Box<Self>, mut io: Io) -> NodeFuture {
        let out = io.output(0);
        let bus = I2cBus::new(io.input(0), out.progress_sink());
        Box::pin(read_through_the_cursor(bus, out, self.strict))
    }
}

async fn read_through_the_cursor(mut bus: I2cBus, mut out: Outbox<Seen>, strict: usize) {
    loop {
        match bus.next().await {
            Next::Transaction(mut tx) => {
                let begin = format!(
                    "tx{}{}",
                    tx.id().0,
                    if tx.is_partial() { " partial" } else { "" }
                );
                let at = tx.consumed();
                out.emit(at, Seen(begin)).await;
                for reads in 0.. {
                    let found = if reads < strict {
                        tx.event().await.map(Read::Event)
                    } else {
                        tx.event_or_end().await
                    };
                    let line = match found {
                        Ok(Read::Event(event)) => format!("{event:?}"),
                        Ok(Read::End) => "end".to_owned(),
                        Err(error) => format!("{error:?}"),
                    };
                    let at = tx.consumed();
                    out.emit(at, Seen(line)).await;
                    if !matches!(found, Ok(Read::Event(_))) {
                        break;
                    }
                }
                assert!(tx.is_ended());
                assert_eq!(bus.finish().await, Ok(()));
            }
            Next::Gap(range) => out.gap(range).await,
            Next::Progress(tick) => out.progress(tick).await,
            Next::End(tick) => {
                out.end(tick).await;
                return;
            }
        }
    }
}

/// What the reader finds in `block`, less `gap`, read with `strict` strict reads.
fn read(block: &LogicBlock, gap: Option<TickRange>, strict: usize) -> Vec<String> {
    let mut harness = Harness::build(
        CHANNELS,
        TimeBase::new(1_000_000),
        GraphConfig {
            queue_capacity: 2,
            budget: 3,
        },
        |graph, source| {
            let i2c = graph.add(I2c::new(I2cConfig::default()).expect("a valid configuration"));
            graph.connect(source.samples, I2c::input(i2c))?;
            let reader = graph.add(Reader { strict });
            graph.connect(I2c::events(i2c), InputRef::of_node(reader, 0))?;
            Ok(Reader::seen(reader))
        },
    )
    .expect("builds");
    harness.feed_with_gaps(block, gap.as_slice(), &[29]);
    harness.end();
    harness
        .without_progress()
        .iter()
        .map(|item| match item {
            Item::Event(event) => format!("{} {}", event.payload.0, at(event.span, event.settled)),
            Item::Gap(range) => format!("gap {range}"),
            Item::Progress(tick) => format!("progress {tick}"),
            Item::End(tick) => format!("eof {tick}"),
        })
        .collect()
}

#[test]
fn the_cursor_reads_a_transaction_event_by_event() {
    let mut waveform = new_waveform();
    let mut symbols = write_7bit(0x50, &[0x12]);
    symbols.push(Symbol::Restart);
    symbols.extend(read_7bit(0x50, &[0x34]));
    waveform.transaction(symbols);
    waveform.idle(10);
    // What a read consumed includes what came before it unreported: the begin before the
    // address.
    assert_eq!(
        read(&waveform.finish(), None, 0),
        vec![
            "tx1 [10,11)@10".to_owned(),
            format!(
                "Address(Address {{ address: 80, ten_bit: false, read: false, ack: true }}) {}",
                byte_at(14)
            ),
            format!("Data {{ byte: 18, ack: true }} {}", byte_at(86)),
            format!("Restart {{ dropped_bits: 0 }} {}", one(162)),
            format!(
                "Address(Address {{ address: 80, ten_bit: false, read: true, ack: true }}) {}",
                byte_at(166)
            ),
            format!("Data {{ byte: 52, ack: false }} {}", byte_at(238)),
            format!("end {}", one(314)),
            "eof 326".to_owned(),
        ]
    );
}

#[test]
fn the_cursor_says_how_a_transaction_ended() {
    let block = two_transactions();
    // A strict read that finds the normal end: the transaction is whole, the read is not.
    let lines = read(&block, None, 6);
    assert_eq!(
        lines[4],
        format!("Data {{ byte: 3, ack: true }} {}", byte_at(230))
    );
    assert_eq!(lines[5], format!("Incomplete {}", one(306)));
    assert_eq!(lines[6], format!("tx2 {}", one(318)));
    // A gap: the aborted end is consumed and the read says the transaction was cut off; the
    // transaction found under way after the gap reads as partial, with no events.
    let lines = read(&block, Some(TickRange::new(Tick(120), Tick(200))), 0);
    assert_eq!(
        lines[..6],
        [
            "tx1 [10,11)@10".to_owned(),
            format!(
                "Address(Address {{ address: 80, ten_bit: false, read: false, ack: true }}) {}",
                byte_at(14)
            ),
            "Truncated @120".to_owned(),
            "gap 120..200".to_owned(),
            "tx2 partial [200,201)@200".to_owned(),
            format!("end {}", one(306)),
        ]
    );
    // The end of the samples.
    let head = block
        .slice(TickRange::new(Tick::ZERO, Tick(114)))
        .expect("a slice");
    let lines = read(&head, None, 0);
    assert_eq!(lines[2..], ["Truncated @114", "eof 114"]);
}

/// The address of a segment in the random traffic below.
#[derive(Clone, Debug)]
enum Addressing {
    /// A 7-bit address below the reserved `1111 xxx` group.
    Seven { address: u8, read: bool, ack: bool },
    /// A 10-bit write address, each byte acknowledged or not.
    TenWrite { address: u16, acks: (bool, bool) },
    /// `1111 0xx 1`: a 10-bit read.
    TenRead { high: u8, ack: bool },
}

/// One address and the data bytes after it.
#[derive(Clone, Debug)]
struct Segment {
    addressing: Addressing,
    data: Vec<(u8, bool)>,
    stretch: u64,
}

fn segment() -> impl Strategy<Value = Segment> {
    let addressing = prop_oneof![
        5 => (0u8..0x78, any::<bool>(), any::<bool>())
            .prop_map(|(address, read, ack)| Addressing::Seven { address, read, ack }),
        2 => (0u16..0x400, any::<(bool, bool)>())
            .prop_map(|(address, acks)| Addressing::TenWrite { address, acks }),
        2 => (0u8..4, any::<bool>()).prop_map(|(high, ack)| Addressing::TenRead { high, ack }),
    ];
    (
        addressing,
        prop::collection::vec(any::<(u8, bool)>(), 0..5),
        prop_oneof![3 => Just(0u64), 1 => 1u64..30],
    )
        .prop_map(|(addressing, data, stretch)| Segment {
            addressing,
            data,
            stretch,
        })
}

fn synthesize(transactions: &[Vec<Segment>], idle: u64) -> LogicBlock {
    let mut waveform = I2cWaveform::new(I2cConfig::default(), CHANNELS, 1, Tick::ZERO);
    waveform.idle(idle);
    for segments in transactions {
        waveform.start();
        for (i, segment) in segments.iter().enumerate() {
            if i > 0 {
                waveform.restart();
            }
            match segment.addressing {
                Addressing::Seven { address, read, ack } => {
                    waveform.byte_stretched((address << 1) | u8::from(read), ack, segment.stretch);
                }
                Addressing::TenWrite { address, acks } => {
                    waveform.byte(0xf0 | (((address >> 8) as u8) << 1), acks.0);
                    waveform.byte_stretched((address & 0xff) as u8, acks.1, segment.stretch);
                }
                Addressing::TenRead { high, ack } => {
                    waveform.byte_stretched(0xf1 | (high << 1), ack, segment.stretch);
                }
            }
            for (value, ack) in &segment.data {
                waveform.byte(*value, *ack);
            }
        }
        waveform.stop();
        waveform.idle(idle);
    }
    waveform.finish()
}

/// What the decoder must report for `transactions`, worked out from the rules directly.
fn model(transactions: &[Vec<Segment>]) -> Vec<I2cEvent> {
    let mut events = Vec::new();
    for (n, segments) in transactions.iter().enumerate() {
        let tx = TxId(n as u64 + 1);
        events.push(I2cEvent::Begin { tx, partial: false });
        let mut remembered: Option<u16> = None;
        for (i, segment) in segments.iter().enumerate() {
            if i > 0 {
                events.push(I2cEvent::Restart {
                    tx,
                    dropped_bits: 0,
                });
            }
            let (address, ten_bit, read, ack) = match segment.addressing {
                Addressing::Seven { address, read, ack } => {
                    remembered = None;
                    (u16::from(address), false, read, ack)
                }
                Addressing::TenWrite { address, acks } => {
                    remembered = Some(address);
                    (address, true, false, acks.0 && acks.1)
                }
                Addressing::TenRead { high, ack } => match remembered {
                    Some(address) if address >> 8 == u16::from(high) => (address, true, true, ack),
                    _ => {
                        remembered = None;
                        (0x78 | u16::from(high), false, true, ack)
                    }
                },
            };
            events.push(I2cEvent::Address {
                tx,
                address,
                ten_bit,
                read,
                ack,
            });
            for (byte, ack) in &segment.data {
                events.push(I2cEvent::Data {
                    tx,
                    byte: *byte,
                    ack: *ack,
                });
            }
        }
        events.push(I2cEvent::End {
            tx,
            reason: EndReason::Stop,
            dropped_bits: 0,
        });
    }
    events
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 48, ..ProptestConfig::default() })]

    /// Random traffic decodes as the rules say, and the same however it is cut into blocks,
    /// read and queued.
    #[test]
    fn random_traffic_decodes_as_the_rules_say(
        transactions in prop::collection::vec(prop::collection::vec(segment(), 1..4), 1..5),
        idle in 1u64..9,
        sizes in prop::collection::vec(1u64..300, 1..5),
        pauses in prop::collection::vec(0usize..9, 1..6),
        queue_capacity in 1usize..5,
        budget in 1u32..30,
    ) {
        let block = synthesize(&transactions, idle);
        let mut reference = build(GraphConfig::default());
        reference.run_all(&block);
        let items = reference.without_progress();
        let payloads: Vec<I2cEvent> =
            items.iter().filter_map(Item::event).map(|event| event.payload).collect();
        prop_assert_eq!(payloads, model(&transactions));
        assert_settled_in_order(&items);

        let mut harness = build(GraphConfig { queue_capacity, budget });
        harness.drain_on_full = 1;
        harness.feed_cut(&block, &[], &sizes, &pauses);
        harness.end();
        harness.assert_progress_promise();
        prop_assert_eq!(harness.without_progress(), items);
    }
}
