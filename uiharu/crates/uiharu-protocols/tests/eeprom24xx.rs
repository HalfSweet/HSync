// SPDX-License-Identifier: Apache-2.0

//! The 24xx EEPROM decoder on synthesized I²C waveforms.
//!
//! The waveforms run at two samples per quarter of the SCL period, as in `tests/i2c.rs`: a
//! transaction's bytes start 4 samples after its START and 72 samples apart, a byte's first bit
//! is taken 2 samples after it starts and its acknowledge bit 66 samples after; a repeated
//! START or a STOP changes SDA 4 samples after the byte before it ended.

use std::collections::BTreeMap;
use std::sync::Arc;

use proptest::prelude::*;
use uiharu_core::testing::{self, Harness};
use uiharu_core::{
    Change, ChannelMask, GraphConfig, GraphError, InputRef, InputSpec, Io, Item, LogicBlock, Node,
    NodeFuture, NodeId, NodeSpec, Outbox, OutputRef, OutputSpec, Props, Provenance, Requires,
    SignalCursor, Tick, TickRange, TimeBase,
};
use uiharu_protocols::eeprom24xx::synth::{ack_poll, current_address_read, random_read, write};
use uiharu_protocols::eeprom24xx::{
    Command, DATA_BATCH, Eeprom24xx, Eeprom24xxConfig, EepromEnd, EepromEvent, OpId,
};
use uiharu_protocols::i2c::synth::{I2cWaveform, Symbol, write_7bit, write_10bit};
use uiharu_protocols::i2c::{EndReason, I2c, I2cConfig, I2cEvent, TxId};
use uiharu_protocols::spi::{Spi, SpiConfig};

const CHANNELS: u8 = 4;
const Q: u64 = 2;

fn build_with(config: Eeprom24xxConfig, graph_config: GraphConfig) -> Harness<EepromEvent> {
    Harness::build(
        CHANNELS,
        TimeBase::new(1_000_000),
        graph_config,
        |graph, source| {
            let i2c = graph.add(I2c::new(I2cConfig::default()).expect("a valid configuration"));
            graph.connect(source.samples, I2c::input(i2c))?;
            let eeprom = graph.add(Eeprom24xx::new(config).expect("a valid configuration"));
            graph.connect(I2c::events(i2c), Eeprom24xx::input(eeprom))?;
            Ok(Eeprom24xx::operations(eeprom))
        },
    )
    .expect("builds")
}

fn build(graph_config: GraphConfig) -> Harness<EepromEvent> {
    build_with(Eeprom24xxConfig::default(), graph_config)
}

fn new_waveform() -> I2cWaveform {
    let mut waveform = I2cWaveform::new(I2cConfig::default(), CHANNELS, Q, Tick::ZERO);
    waveform.idle(10);
    waveform
}

fn byte(value: u8, ack: bool) -> Symbol {
    Symbol::Byte { value, ack }
}

fn decode_with(config: Eeprom24xxConfig, block: &LogicBlock) -> Vec<String> {
    let mut harness = build_with(config, GraphConfig::default());
    harness.run_all(block);
    describe(&harness.without_progress())
}

fn decode(block: &LogicBlock) -> Vec<String> {
    decode_with(Eeprom24xxConfig::default(), block)
}

fn at(span: TickRange, settled: Tick) -> String {
    if span.is_empty() {
        format!("@{settled}")
    } else {
        format!("[{},{})@{settled}", span.start, span.end)
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// One line per item, readable in assertions.
fn describe(items: &[Item<EepromEvent>]) -> Vec<String> {
    items
        .iter()
        .map(|item| match item {
            Item::Event(event) => {
                let what = match &event.payload {
                    EepromEvent::Begin {
                        op,
                        tx,
                        device,
                        command,
                    } => format!("begin op{} tx{} {device:02x} {command:?}", op.0, tx),
                    EepromEvent::Address { op, word_address } => {
                        format!("address op{} {word_address:04x}", op.0)
                    }
                    EepromEvent::Data { op, offset, bytes } => {
                        format!("data op{} +{offset} {}", op.0, hex(bytes))
                    }
                    EepromEvent::End { op, reason, length } => {
                        format!("end op{} {reason:?} {length}", op.0)
                    }
                    EepromEvent::AckPoll { tx, device, acked } => format!(
                        "ack-poll tx{} {device:02x} {}",
                        tx,
                        if *acked { "ready" } else { "busy" }
                    ),
                    EepromEvent::Unknown { tx, device } => {
                        format!("unknown tx{} {device:02x}", tx)
                    }
                };
                format!("{what} {}", at(event.span, event.settled))
            }
            Item::Gap(range) => format!("gap {range}"),
            Item::Progress(tick) => format!("progress {tick}"),
            Item::End(tick) => format!("eof {tick}"),
        })
        .collect()
}

/// The span and settled tick of the byte whose bit cells start at `start`: its first rising
/// edge through its acknowledge edge.
fn byte_at(start: u64) -> String {
    format!("[{},{})@{}", start + 2, start + 67, start + 66)
}

/// The bytes starting at `first` through the one starting at `last`, settled at `settled`.
fn bytes_at(first: u64, last: u64, settled: u64) -> String {
    format!("[{},{})@{settled}", first + 2, last + 67)
}

/// A condition at `tick`.
fn one(tick: u64) -> String {
    format!("[{tick},{})@{tick}", tick + 1)
}

fn assert_settled_in_order(items: &[Item<EepromEvent>]) {
    let mut last = Tick::ZERO;
    for event in items.iter().filter_map(Item::event) {
        assert!(event.settled >= last, "{event:?} settles before {last}");
        last = event.settled;
    }
}

#[test]
fn decodes_a_byte_write() {
    let mut waveform = new_waveform();
    waveform.transaction(write(0x50, 2, 0x0123, &[0xaa]));
    waveform.idle(10);
    // The START is at 10; the control byte, the two address bytes and the data byte start at
    // 14, 86, 158 and 230; the STOP is at 306 and decides the command.
    assert_eq!(
        decode(&waveform.finish()),
        vec![
            "begin op1 tx1 50 ByteWrite [10,81)@306",
            "address op1 0123 [88,225)@306",
            "data op1 +0 aa [232,297)@306",
            "end op1 Complete 1 [306,307)@306",
            "eof 318",
        ]
    );
}

#[test]
fn decodes_a_page_write_once_its_second_byte_is_seen() {
    let mut waveform = new_waveform();
    waveform.transaction(write(0x50, 2, 0x0040, &[1, 2, 3, 4, 5]));
    waveform.idle(10);
    // The second data byte (starting at 302) tells a page write; the data goes out at the
    // STOP, at 594.
    assert_eq!(
        decode(&waveform.finish()),
        vec![
            "begin op1 tx1 50 PageWrite [10,81)@368".to_owned(),
            "address op1 0040 [88,225)@368".to_owned(),
            format!("data op1 +0 0102030405 {}", bytes_at(230, 518, 594)),
            format!("end op1 Complete 5 {}", one(594)),
            "eof 606".to_owned(),
        ]
    );
}

#[test]
fn decodes_a_set_address() {
    let mut waveform = new_waveform();
    waveform.transaction(write(0x50, 2, 0x0100, &[]));
    waveform.idle(10);
    assert_eq!(
        decode(&waveform.finish()),
        vec![
            "begin op1 tx1 50 SetAddress [10,81)@234",
            "address op1 0100 [88,225)@234",
            "end op1 Complete 0 [234,235)@234",
            "eof 246",
        ]
    );
    // Ended by a repeated START (at 234) that only a STOP (at 242) follows: no random read
    // came, which the STOP decides.
    let mut waveform = new_waveform();
    let mut symbols = write(0x50, 2, 0x0102, &[]);
    symbols.push(Symbol::Restart);
    waveform.transaction(symbols);
    waveform.idle(10);
    assert_eq!(
        decode(&waveform.finish()),
        vec![
            "begin op1 tx1 50 SetAddress [10,81)@242",
            "address op1 0102 [88,225)@242",
            "end op1 Complete 0 [234,235)@242",
            "eof 254",
        ]
    );
}

#[test]
fn decodes_a_current_address_read() {
    let mut waveform = new_waveform();
    waveform.transaction(current_address_read(0x50, &[0x11]));
    waveform.idle(10);
    // A read is told by its control byte.
    assert_eq!(
        decode(&waveform.finish()),
        vec![
            "begin op1 tx1 50 CurrentAddressRead [10,81)@80".to_owned(),
            format!("data op1 +0 11 {}", bytes_at(86, 86, 162)),
            format!("end op1 Complete 1 {}", one(162)),
            "eof 174".to_owned(),
        ]
    );
}

#[test]
fn decodes_a_random_read_across_its_repeated_start() {
    let mut waveform = new_waveform();
    waveform.transaction(random_read(0x50, 2, 0x1234, &[0x99, 0x98]));
    waveform.idle(10);
    // The repeated START lowers SDA at 234; the read's control byte starts at 238 and tells
    // the random read. The begin spans both control bytes.
    assert_eq!(
        decode(&waveform.finish()),
        vec![
            "begin op1 tx1 50 RandomRead [10,305)@304".to_owned(),
            "address op1 1234 [88,225)@304".to_owned(),
            format!("data op1 +0 9998 {}", bytes_at(310, 382, 458)),
            format!("end op1 Complete 2 {}", one(458)),
            "eof 470".to_owned(),
        ]
    );
}

#[test]
fn a_sequential_read_goes_out_in_batches() {
    let bytes: Vec<u8> = (0..40).map(|i| i * 3).collect();
    let mut waveform = new_waveform();
    waveform.transaction(current_address_read(0x50, &bytes));
    waveform.idle(10);
    // Data byte `i` starts at 86 + 72 i; the first batch is full at the 32nd byte's
    // acknowledge bit, the rest goes out at the STOP.
    let data = |i: u64| 86 + 72 * i;
    assert_eq!(
        decode(&waveform.finish()),
        vec![
            "begin op1 tx1 50 CurrentAddressRead [10,81)@80".to_owned(),
            format!(
                "data op1 +0 {} {}",
                hex(&bytes[..32]),
                byte_range(data(0), data(31))
            ),
            format!(
                "data op1 +32 {} {}",
                hex(&bytes[32..]),
                bytes_at(data(32), data(39), 2970)
            ),
            format!("end op1 Complete 40 {}", one(2970)),
            "eof 2982".to_owned(),
        ]
    );
    assert_eq!(DATA_BATCH, 32);
    // A random read batches alike.
    let mut waveform = new_waveform();
    waveform.transaction(random_read(0x53, 2, 0x7ff0, &bytes));
    waveform.idle(10);
    let lines = decode(&waveform.finish());
    assert!(
        lines[0].starts_with("begin op1 tx1 53 RandomRead "),
        "{lines:?}"
    );
    assert!(lines[2].starts_with(&format!("data op1 +0 {} ", hex(&bytes[..32]))));
    assert!(lines[3].starts_with(&format!("data op1 +32 {} ", hex(&bytes[32..]))));
    assert!(lines[4].starts_with("end op1 Complete 40 "), "{lines:?}");
}

/// The bytes starting at `first` through `last`, settled at the last one's acknowledge edge.
fn byte_range(first: u64, last: u64) -> String {
    bytes_at(first, last, last + 66)
}

#[test]
fn acknowledge_polls_are_reported() {
    let mut waveform = new_waveform();
    // Two polls while the device is busy, one that finds it ready, then a write.
    waveform.transaction(ack_poll(0x50, false));
    waveform.idle(10);
    waveform.transaction(ack_poll(0x50, false));
    waveform.idle(10);
    waveform.transaction(ack_poll(0x50, true));
    waveform.idle(10);
    waveform.transaction(write(0x50, 2, 0x0000, &[0x5a]));
    waveform.idle(10);
    assert_eq!(
        decode(&waveform.finish()),
        vec![
            "ack-poll tx1 50 busy [10,81)@80",
            "ack-poll tx2 50 busy [102,173)@172",
            "ack-poll tx3 50 ready [194,275)@274",
            "begin op1 tx4 50 ByteWrite [286,357)@582",
            "address op1 0000 [364,501)@582",
            "data op1 +0 5a [508,573)@582",
            "end op1 Complete 1 [582,583)@582",
            "eof 594",
        ]
    );
    // Polling with repeated STARTs: the poll that is acknowledged begins the command.
    let mut waveform = new_waveform();
    let mut symbols = ack_poll(0x50, false);
    symbols.push(Symbol::Restart);
    symbols.extend(ack_poll(0x50, false));
    symbols.push(Symbol::Restart);
    symbols.extend(write(0x50, 2, 0x0010, &[0x77]));
    waveform.transaction(symbols);
    waveform.idle(10);
    assert_eq!(
        decode(&waveform.finish()),
        vec![
            "ack-poll tx1 50 busy [10,81)@80".to_owned(),
            format!("ack-poll tx1 50 busy {}", byte_at(94)),
            "begin op1 tx1 50 ByteWrite [176,241)@466".to_owned(),
            "address op1 0010 [248,385)@466".to_owned(),
            "data op1 +0 77 [392,457)@466".to_owned(),
            format!("end op1 Complete 1 {}", one(466)),
            "eof 478".to_owned(),
        ]
    );
    // A poll that finds the device ready, ended by a repeated START (at 90) and followed by
    // a read.
    let mut waveform = new_waveform();
    let mut symbols = ack_poll(0x50, true);
    symbols.push(Symbol::Restart);
    symbols.extend(current_address_read(0x50, &[0x21]));
    waveform.transaction(symbols);
    waveform.idle(10);
    assert_eq!(
        decode(&waveform.finish()),
        vec![
            "ack-poll tx1 50 ready [10,91)@90".to_owned(),
            format!("begin op1 tx1 50 CurrentAddressRead {}", byte_at(94)),
            format!("data op1 +0 21 {}", bytes_at(166, 166, 242)),
            format!("end op1 Complete 1 {}", one(242)),
            "eof 254".to_owned(),
        ]
    );
}

/// Traffic to devices other than `0x50`, then a byte write to `0x50`.
fn mixed_devices() -> LogicBlock {
    let mut waveform = new_waveform();
    waveform.transaction(write(0x51, 2, 0x0001, &[1]));
    waveform.idle(10);
    waveform.transaction(current_address_read(0x51, &[2]));
    waveform.idle(10);
    waveform.transaction(write_7bit(0x68, &[0x00, 0x01]));
    waveform.idle(10);
    waveform.transaction(write_10bit(0x050, &[3]));
    waveform.idle(10);
    // A segment to another device, then a read from 0x50 in the same transaction.
    let mut symbols = write_7bit(0x68, &[0x10]);
    symbols.push(Symbol::Restart);
    symbols.extend(current_address_read(0x50, &[0x44]));
    waveform.transaction(symbols);
    waveform.idle(10);
    waveform.transaction(write(0x50, 2, 0x0002, &[4]));
    waveform.idle(10);
    waveform.finish()
}

#[test]
fn traffic_to_other_devices_is_ignored() {
    let block = mixed_devices();
    let lines = decode_with(
        Eeprom24xxConfig {
            device_address: Some(0x50),
            ..Eeprom24xxConfig::default()
        },
        &block,
    );
    let summary: Vec<&str> = lines
        .iter()
        .map(|line| line.split(" [").next().unwrap_or(line))
        .collect();
    assert_eq!(
        summary,
        vec![
            "begin op1 tx5 50 CurrentAddressRead",
            "data op1 +0 44",
            "end op1 Complete 1",
            "begin op2 tx6 50 ByteWrite",
            "address op2 0002",
            "data op2 +0 04",
            "end op2 Complete 1",
            lines[7].as_str(),
        ]
    );
    assert!(lines[7].starts_with("eof "), "{lines:?}");
    // Following any 24xx device, 0x51 is one too; 0x68 and the 10-bit address are not.
    let lines = decode(&block);
    assert!(
        lines[0].starts_with("begin op1 tx1 51 ByteWrite "),
        "{lines:?}"
    );
    assert!(
        lines[4].starts_with("begin op2 tx2 51 CurrentAddressRead "),
        "{lines:?}"
    );
    assert!(
        lines[7].starts_with("begin op3 tx5 50 CurrentAddressRead "),
        "{lines:?}"
    );
    assert_eq!(lines.len(), 15, "{lines:?}");
}

#[test]
fn commands_ended_too_soon_are_incomplete() {
    // A read with no byte, a STOP right after its control byte.
    let mut waveform = new_waveform();
    waveform.transaction(current_address_read(0x50, &[]));
    waveform.idle(10);
    assert_eq!(
        decode(&waveform.finish()),
        vec![
            "begin op1 tx1 50 CurrentAddressRead [10,81)@80",
            "end op1 Incomplete 0 [90,91)@90",
            "eof 102",
        ]
    );
    // A random read with a STOP right after its read control byte.
    let mut waveform = new_waveform();
    waveform.transaction(random_read(0x50, 2, 0x0102, &[]));
    waveform.idle(10);
    assert_eq!(
        decode(&waveform.finish()),
        vec![
            "begin op1 tx1 50 RandomRead [10,305)@304",
            "address op1 0102 [88,225)@304",
            "end op1 Incomplete 0 [314,315)@314",
            "eof 326",
        ]
    );
    // A byte write ended by a repeated START, which the device does not program, then a
    // current-address read in the same transaction.
    let mut waveform = new_waveform();
    let mut symbols = write(0x50, 2, 0x0010, &[0x77]);
    symbols.push(Symbol::Restart);
    symbols.extend(current_address_read(0x50, &[0x33]));
    waveform.transaction(symbols);
    waveform.idle(10);
    assert_eq!(
        decode(&waveform.finish()),
        vec![
            "begin op1 tx1 50 ByteWrite [10,81)@306".to_owned(),
            "address op1 0010 [88,225)@306".to_owned(),
            "data op1 +0 77 [232,297)@306".to_owned(),
            format!("end op1 Incomplete 1 {}", one(306)),
            format!("begin op2 tx1 50 CurrentAddressRead {}", byte_at(310)),
            format!("data op2 +0 33 {}", bytes_at(382, 382, 458)),
            format!("end op2 Complete 1 {}", one(458)),
            "eof 470".to_owned(),
        ]
    );
}

#[test]
fn a_write_with_its_word_address_cut_short_fits_no_command() {
    let mut waveform = new_waveform();
    waveform.transaction([byte(0xa0, true), byte(0x01, true)]);
    waveform.idle(10);
    assert_eq!(
        decode(&waveform.finish()),
        vec!["unknown tx1 50 [10,163)@162", "eof 174"]
    );
    // Cut short by a repeated START, after which a read follows.
    let mut waveform = new_waveform();
    let mut symbols = vec![byte(0xa0, true), byte(0x01, true), Symbol::Restart];
    symbols.extend(current_address_read(0x50, &[0x44]));
    waveform.transaction(symbols);
    waveform.idle(10);
    assert_eq!(
        decode(&waveform.finish()),
        vec![
            "unknown tx1 50 [10,163)@162".to_owned(),
            format!("begin op1 tx1 50 CurrentAddressRead {}", byte_at(166)),
            format!("data op1 +0 44 {}", bytes_at(238, 238, 314)),
            format!("end op1 Complete 1 {}", one(314)),
            "eof 326".to_owned(),
        ]
    );
}

#[test]
fn a_gap_aborts_a_read_and_the_next_transaction_decodes() {
    let data: Vec<u8> = (0..10).collect();
    let mut waveform = new_waveform();
    waveform.transaction(current_address_read(0x50, &data));
    waveform.idle(10);
    waveform.transaction(random_read(0x50, 2, 0x0200, &[0x42]));
    waveform.idle(10);
    let block = waveform.finish();
    // From the middle of the sixth data byte to the middle of the eighth: the bus is busy
    // after the gap, and the transaction found under way is skipped.
    let gap = TickRange::new(Tick(86 + 72 * 5 + 30), Tick(86 + 72 * 7 + 30));
    let mut harness = build(GraphConfig::default());
    harness.feed_with_gaps(&block, &[gap], &[41]);
    harness.end();
    assert_eq!(
        describe(&harness.without_progress()),
        vec![
            "begin op1 tx1 50 CurrentAddressRead [10,81)@80".to_owned(),
            format!("data op1 +0 0001020304 {}", bytes_at(86, 374, 476)),
            "end op1 Aborted 5 @476".to_owned(),
            "gap 476..620".to_owned(),
            "begin op2 tx3 50 RandomRead [822,1117)@1116".to_owned(),
            "address op2 0200 [900,1037)@1116".to_owned(),
            format!("data op2 +0 42 {}", bytes_at(1122, 1122, 1198)),
            format!("end op2 Complete 1 {}", one(1198)),
            "eof 1210".to_owned(),
        ]
    );
}

#[test]
fn a_write_cut_off_before_its_command_is_told_is_a_write() {
    // The samples end after one data byte: a byte write or a page write.
    let mut waveform = new_waveform();
    waveform.start();
    waveform.symbols(write(0x50, 2, 0x0010, &[0x77]));
    waveform.bits(0x55, 3);
    assert_eq!(
        decode(&waveform.finish()),
        vec![
            "begin op1 tx1 50 Write [10,81)@326",
            "address op1 0010 [88,225)@326",
            "data op1 +0 77 [232,297)@326",
            "end op1 Aborted 1 @326",
            "eof 326",
        ]
    );
    // The samples end right after the control byte.
    let mut waveform = new_waveform();
    waveform.start();
    waveform.byte(0xa0, true);
    waveform.bits(0x00, 2);
    assert_eq!(
        decode(&waveform.finish()),
        vec![
            "begin op1 tx1 50 Write [10,81)@102",
            "end op1 Aborted 0 @102",
            "eof 102",
        ]
    );
    // A set-address that a repeated START ended stays one when the samples end before the
    // address after it.
    let mut waveform = new_waveform();
    waveform.start();
    waveform.symbols(write(0x50, 2, 0x0102, &[]));
    waveform.restart();
    waveform.bits(0xa1, 4);
    assert_eq!(
        decode(&waveform.finish()),
        vec![
            "begin op1 tx1 50 SetAddress [10,81)@270",
            "address op1 0102 [88,225)@270",
            "end op1 Complete 0 [234,235)@270",
            "eof 270",
        ]
    );
}

#[test]
fn one_byte_word_addresses() {
    let config = Eeprom24xxConfig {
        address_bytes: 1,
        device_address: None,
    };
    let mut waveform = new_waveform();
    waveform.transaction(write(0x50, 1, 0x7f, &[1, 2]));
    waveform.idle(10);
    waveform.transaction(random_read(0x50, 1, 0x10, &[5]));
    waveform.idle(10);
    assert_eq!(
        decode_with(config, &waveform.finish()),
        vec![
            "begin op1 tx1 50 PageWrite [10,81)@296".to_owned(),
            format!("address op1 007f {}", bytes_at(86, 86, 296)),
            format!("data op1 +0 0102 {}", bytes_at(158, 230, 306)),
            format!("end op1 Complete 2 {}", one(306)),
            "begin op2 tx2 50 RandomRead [318,541)@540".to_owned(),
            format!("address op2 0010 {}", bytes_at(394, 394, 540)),
            format!("data op2 +0 05 {}", bytes_at(546, 546, 622)),
            format!("end op2 Complete 1 {}", one(622)),
            "eof 634".to_owned(),
        ]
    );
}

#[test]
fn data_shows_up_before_a_long_read_ends() {
    let bytes: Vec<u8> = (0..100).collect();
    let mut waveform = new_waveform();
    waveform.transaction(current_address_read(0x50, &bytes));
    waveform.idle(10);
    let block = waveform.finish();
    // Up to the middle of the 70th data byte.
    let cut = Tick(86 + 72 * 69 + 30);
    let head = block
        .slice(TickRange::new(Tick::ZERO, cut))
        .expect("a slice");
    let mut harness = build(GraphConfig::default());
    harness.feed(&head, &[53]);
    harness.collect();
    let lines = describe(&harness.without_progress());
    assert_eq!(lines.len(), 3, "{lines:?}");
    assert!(lines[2].starts_with(&format!("data op1 +32 {} ", hex(&bytes[32..64]))));
    let tail = block
        .slice(TickRange::new(cut, block.end()))
        .expect("a slice");
    harness.feed(&tail, &[53]);
    harness.end();
    let lines = describe(&harness.without_progress());
    assert!(lines[3].starts_with(&format!("data op1 +64 {} ", hex(&bytes[64..96]))));
    assert!(lines[4].starts_with(&format!("data op1 +96 {} ", hex(&bytes[96..]))));
    assert!(lines[5].starts_with("end op1 Complete 100 "), "{lines:?}");
}

/// Every command, polls, other devices, a short word address and bytes cut short.
fn rich_traffic() -> LogicBlock {
    let mut waveform = new_waveform();
    waveform.transaction(write(0x50, 2, 0x0100, &[9, 8, 7]));
    waveform.idle(5);
    waveform.transaction(ack_poll(0x50, false));
    waveform.idle(3);
    waveform.transaction(ack_poll(0x50, true));
    waveform.idle(7);
    waveform.transaction(random_read(0x52, 2, 0x0abc, &[1, 2, 3, 4, 5, 6]));
    waveform.idle(4);
    waveform.transaction(write(0x50, 2, 0x0200, &[]));
    waveform.idle(6);
    waveform.transaction(current_address_read(0x50, &[0x10, 0x20]));
    waveform.idle(2);
    waveform.transaction(write_7bit(0x68, &[1, 2, 3]));
    waveform.idle(8);
    waveform.transaction([byte(0xa0, true), byte(0x02, true)]);
    waveform.idle(5);
    waveform.transaction(write(0x51, 2, 0x0300, &[0x5a]));
    waveform.idle(9);
    let mut symbols = write(0x50, 2, 0x0004, &[0x01]);
    symbols.push(Symbol::Restart);
    symbols.extend(random_read(0x50, 2, 0x0005, &[0x66]));
    waveform.transaction(symbols);
    waveform.idle(5);
    waveform.transaction(current_address_read(0x50, &(0..45).collect::<Vec<u8>>()));
    waveform.idle(3);
    waveform.finish()
}

#[test]
fn every_cut_and_pause_yields_the_same_events() {
    let block = rich_traffic();
    // In the random read's data, on the free bus before the short word address (2131 to
    // 2139), and in the long read after its first batch (its data byte `i` starts at
    // 3360 + 72 i).
    let gaps = [
        TickRange::new(Tick(1101), Tick(1151)),
        TickRange::new(Tick(2132), Tick(2136)),
        TickRange::new(Tick(3360 + 72 * 36 + 20), Tick(6100)),
    ];
    let mut harness = build(GraphConfig::default());
    harness.feed_with_gaps(&block, &gaps, &[u64::MAX]);
    harness.end();
    let items = harness.without_progress();
    assert_settled_in_order(&items);
    let lines = describe(&items);
    for kind in ["begin ", "ack-poll ", "unknown ", "data ", "end ", "gap "] {
        assert!(
            lines.iter().any(|line| line.starts_with(kind)),
            "no {kind}in {lines:?}"
        );
    }
    for expected in [
        "end op2 Aborted 2 @1101",
        "gap 2132..2136",
        "end op8 Aborted 36 @5972",
    ] {
        assert!(
            lines.iter().any(|line| line == expected),
            "no {expected} in {lines:?}"
        );
    }
    testing::assert_block_invariant(build, &block, &gaps);
}

/// A node that plays a fixed list of I²C events, as another producer of `i2c.events` might
/// send them, then ends with its samples.
struct Script {
    events: Vec<(Provenance, I2cEvent)>,
    props: Props,
}

impl Node for Script {
    fn spec(&self) -> NodeSpec {
        NodeSpec {
            name: "script".into(),
            inputs: vec![InputSpec::new::<Arc<LogicBlock>>("logic", Requires::new())],
            outputs: vec![OutputSpec::new::<I2cEvent>("events", self.props.clone())],
        }
    }

    fn start(self: Box<Self>, mut io: Io) -> NodeFuture {
        Box::pin(play(io.signal(0), io.output(0), self.events))
    }
}

async fn play(
    mut signal: SignalCursor,
    mut out: Outbox<I2cEvent>,
    events: Vec<(Provenance, I2cEvent)>,
) {
    for (at, event) in events {
        out.emit(at, event).await;
    }
    loop {
        if let Change::End(tick) = signal.next_change(ChannelMask::EMPTY).await {
            out.end(tick).await;
            return;
        }
    }
}

fn script_events(node: NodeId) -> OutputRef<I2cEvent> {
    OutputRef::of_node(node, 0)
}

#[test]
fn shapes_this_i2c_decoder_never_sends_are_read_gracefully() {
    let tx = 1;
    let address = |address: u16, read: bool| I2cEvent::Address {
        tx,
        address,
        ten_bit: false,
        read,
        ack: true,
    };
    let data = |byte: u8| I2cEvent::Data {
        tx,
        byte,
        ack: true,
    };
    // A byte write to 0x0010 followed by a read address with no repeated START before it, and
    // a data byte with no address before it, then a read.
    let script = [
        I2cEvent::Begin { tx, partial: false },
        address(0x50, false),
        data(0x00),
        data(0x10),
        data(0x77),
        address(0x50, true),
        data(0x33),
        I2cEvent::Restart {
            tx,
            dropped_bits: 0,
        },
        data(0x55),
        address(0x50, true),
        data(0x66),
        I2cEvent::End {
            tx,
            reason: EndReason::Stop,
            dropped_bits: 0,
        },
    ];
    let events = script
        .into_iter()
        .enumerate()
        .map(|(i, event)| {
            let start = 10 * i as u64;
            (
                Provenance::over(
                    TickRange::new(Tick(start), Tick(start + 5)),
                    Tick(start + 4),
                ),
                event,
            )
        })
        .collect();
    let mut harness = Harness::build(
        CHANNELS,
        TimeBase::new(1_000_000),
        GraphConfig::default(),
        |graph, source| {
            let script = graph.add(Script {
                events,
                props: I2cConfig::default().props(),
            });
            graph.connect(source.samples, InputRef::of_node(script, 0))?;
            let eeprom = graph.add(Eeprom24xx::new(Eeprom24xxConfig::default()).expect("valid"));
            graph.connect(script_events(script), Eeprom24xx::input(eeprom))?;
            Ok(Eeprom24xx::operations(eeprom))
        },
    )
    .expect("builds");
    let samples = I2cWaveform::new(I2cConfig::default(), CHANNELS, Q, Tick::ZERO);
    let mut samples = samples;
    samples.idle(200);
    harness.run_all(&samples.finish());
    assert_eq!(
        describe(&harness.without_progress()),
        vec![
            // The write ends, incomplete, where the unannounced address begins.
            "begin op1 tx1 50 ByteWrite [0,15)@50",
            "address op1 0010 [20,35)@50",
            "data op1 +0 77 [40,45)@50",
            "end op1 Incomplete 1 @50",
            "begin op2 tx1 50 CurrentAddressRead [50,55)@54",
            "data op2 +0 33 [60,65)@74",
            "end op2 Complete 1 [70,75)@74",
            // The byte after the repeated START had no address before it and was skipped.
            "begin op3 tx1 50 CurrentAddressRead [90,95)@94",
            "data op3 +0 66 [100,105)@114",
            "end op3 Complete 1 [110,115)@114",
            "eof 200",
        ]
    );
}

#[test]
fn the_bus_must_meet_the_decoders_requirements() {
    // The SPI decoder's events are another schema: wiring them in is refused before any
    // requirement is looked at.
    let refused = Harness::build(
        CHANNELS,
        TimeBase::new(1_000_000),
        GraphConfig::default(),
        |graph, source| {
            let spi = graph.add(Spi::new(SpiConfig::default()).expect("valid"));
            graph.connect(source.samples, Spi::input(spi))?;
            let eeprom = graph.add(Eeprom24xx::new(Eeprom24xxConfig::default()).expect("valid"));
            graph.connect(
                OutputRef::<I2cEvent>::of_node(spi, 0),
                Eeprom24xx::input(eeprom),
            )?;
            Ok(Eeprom24xx::operations(eeprom))
        },
    )
    .expect_err("refused");
    assert!(
        matches!(
            &refused,
            GraphError::SchemaMismatch { from, to, output, input }
                if from == "spi#0.events"
                    && to == "eeprom24xx#1.bus"
                    && output.name == "spi.events"
                    && input.name == "i2c.events"
        ),
        "{refused:?}"
    );
    // A stream of I²C events that does not declare what the decoder needs.
    let requirement = |props: Props| {
        Harness::build(
            CHANNELS,
            TimeBase::new(1_000_000),
            GraphConfig::default(),
            |graph, source| {
                let script = graph.add(Script {
                    events: Vec::new(),
                    props,
                });
                graph.connect(source.samples, InputRef::of_node(script, 0))?;
                let eeprom =
                    graph.add(Eeprom24xx::new(Eeprom24xxConfig::default()).expect("valid"));
                graph.connect(script_events(script), Eeprom24xx::input(eeprom))?;
                Ok(Eeprom24xx::operations(eeprom))
            },
        )
        .expect_err("refused")
    };
    assert_eq!(
        requirement(Props::new().with("framing", "start-stop")),
        GraphError::Requirement {
            from: "script#0.events".into(),
            to: "eeprom24xx#1.bus".into(),
            key: "bus",
            requirement: "= \"i2c\"".into(),
            found: "nothing".into(),
        }
    );
    assert!(matches!(
        requirement(Props::new().with("bus", "i2c").with("framing", "none")),
        GraphError::Requirement { key: "framing", .. }
    ));
}

/// One transaction of the random traffic below.
#[derive(Clone, Debug)]
enum Cmd {
    Write {
        device: u8,
        word: u16,
        data: Vec<u8>,
    },
    CurrentRead {
        device: u8,
        data: Vec<u8>,
    },
    RandomRead {
        device: u8,
        word: u16,
        data: Vec<u8>,
    },
    Poll {
        device: u8,
        acked: bool,
    },
    ShortAddress {
        device: u8,
    },
}

impl Cmd {
    fn device(&self) -> u8 {
        match self {
            Cmd::Write { device, .. }
            | Cmd::CurrentRead { device, .. }
            | Cmd::RandomRead { device, .. }
            | Cmd::Poll { device, .. }
            | Cmd::ShortAddress { device } => *device,
        }
    }

    fn symbols(&self) -> Vec<Symbol> {
        match self {
            Cmd::Write { device, word, data } => write(*device, 2, *word, data),
            Cmd::CurrentRead { device, data } => current_address_read(*device, data),
            Cmd::RandomRead { device, word, data } => random_read(*device, 2, *word, data),
            Cmd::Poll { device, acked } => ack_poll(*device, *acked),
            Cmd::ShortAddress { device } => vec![byte(*device << 1, true), byte(0x12, true)],
        }
    }
}

fn cmd() -> impl Strategy<Value = Cmd> {
    let device = prop_oneof![6 => 0x50u8..=0x57, 1 => Just(0x68u8)];
    let data = || prop::collection::vec(any::<u8>(), 0..70);
    prop_oneof![
        4 => (device.clone(), any::<u16>(), data())
            .prop_map(|(device, word, data)| Cmd::Write { device, word, data }),
        3 => (device.clone(), data()).prop_map(|(device, data)| Cmd::CurrentRead { device, data }),
        3 => (device.clone(), any::<u16>(), data())
            .prop_map(|(device, word, data)| Cmd::RandomRead { device, word, data }),
        2 => (device.clone(), any::<bool>()).prop_map(|(device, acked)| Cmd::Poll { device, acked }),
        1 => device.prop_map(|device| Cmd::ShortAddress { device }),
    ]
}

fn synthesize(cmds: &[Cmd], idle: u64) -> LogicBlock {
    let mut waveform = I2cWaveform::new(I2cConfig::default(), CHANNELS, 1, Tick::ZERO);
    waveform.idle(idle);
    for cmd in cmds {
        waveform.transaction(cmd.symbols());
        waveform.idle(idle);
    }
    waveform.finish()
}

/// What the decoder must report for random commands, worked out from the rules directly.
#[derive(Default)]
struct Model {
    events: Vec<EepromEvent>,
    ops: u64,
}

impl Model {
    fn command(
        &mut self,
        (tx, device): (TxId, u8),
        command: Command,
        word: Option<u16>,
        data: &[u8],
        reason: EepromEnd,
    ) {
        self.ops += 1;
        let op = OpId(self.ops);
        self.events.push(EepromEvent::Begin {
            op,
            tx,
            device,
            command,
        });
        if let Some(word_address) = word {
            self.events.push(EepromEvent::Address { op, word_address });
        }
        for (i, bytes) in data.chunks(DATA_BATCH).enumerate() {
            self.events.push(EepromEvent::Data {
                op,
                offset: (i * DATA_BATCH) as u32,
                bytes: bytes.to_vec(),
            });
        }
        self.events.push(EepromEvent::End {
            op,
            reason,
            length: data.len() as u32,
        });
    }

    fn of(cmds: &[Cmd]) -> Vec<EepromEvent> {
        let mut model = Model::default();
        for (n, cmd) in cmds.iter().enumerate() {
            let (tx, device) = (n as u64 + 1, cmd.device());
            if !(0x50..=0x57).contains(&device) {
                continue;
            }
            // A read that read nothing is incomplete.
            let read = |data: &[u8]| {
                if data.is_empty() {
                    EepromEnd::Incomplete
                } else {
                    EepromEnd::Complete
                }
            };
            match cmd {
                Cmd::Write { word, data, .. } => {
                    let command = match data.len() {
                        0 => Command::SetAddress,
                        1 => Command::ByteWrite,
                        _ => Command::PageWrite,
                    };
                    let at = (tx, device);
                    model.command(at, command, Some(*word), data, EepromEnd::Complete);
                }
                Cmd::CurrentRead { data, .. } => {
                    let reason = read(data);
                    let command = Command::CurrentAddressRead;
                    model.command((tx, device), command, None, data, reason);
                }
                Cmd::RandomRead { word, data, .. } => {
                    let reason = read(data);
                    let command = Command::RandomRead;
                    model.command((tx, device), command, Some(*word), data, reason);
                }
                Cmd::Poll { acked, .. } => model.events.push(EepromEvent::AckPoll {
                    tx,
                    device,
                    acked: *acked,
                }),
                Cmd::ShortAddress { .. } => {
                    model.events.push(EepromEvent::Unknown { tx, device });
                }
            }
        }
        model.events
    }
}

/// The events of `block`, less `gap`, in one uninterrupted run.
fn reference(block: &LogicBlock, gap: Option<TickRange>) -> Vec<Item<EepromEvent>> {
    let mut harness = build(GraphConfig::default());
    harness.feed_with_gaps(block, gap.as_slice(), &[u64::MAX]);
    harness.end();
    harness.without_progress()
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 48, ..ProptestConfig::default() })]

    /// Random commands decode as the rules say, and the same however the samples are cut
    /// into blocks, read and queued.
    #[test]
    fn random_commands_decode_as_the_rules_say(
        cmds in prop::collection::vec(cmd(), 1..8),
        idle in 1u64..9,
        sizes in prop::collection::vec(1u64..400, 1..5),
        pauses in prop::collection::vec(0usize..9, 1..6),
        queue_capacity in 1usize..5,
        budget in 1u32..30,
    ) {
        let block = synthesize(&cmds, idle);
        let items = reference(&block, None);
        let payloads: Vec<EepromEvent> =
            items.iter().filter_map(Item::event).map(|event| event.payload.clone()).collect();
        prop_assert_eq!(payloads, Model::of(&cmds));
        assert_settled_in_order(&items);

        let mut harness = build(GraphConfig { queue_capacity, budget });
        harness.drain_on_full = 1;
        harness.feed_cut(&block, &[], &sizes, &pauses);
        harness.end();
        prop_assert_eq!(harness.without_progress(), items);
    }

    /// Cut-off and lost samples: every operation that begins ends exactly once, batches stay
    /// within their bound and follow each other, and the events are the same however the
    /// samples are cut into blocks.
    #[test]
    fn every_operation_ends_once_whatever_is_lost(
        cmds in prop::collection::vec(cmd(), 1..6),
        cut in 0.05f64..1.0,
        gap in prop::option::of((0.05f64..0.95, 1u64..300)),
        sizes in prop::collection::vec(1u64..300, 1..4),
        pauses in prop::collection::vec(0usize..6, 1..4),
    ) {
        let whole = synthesize(&cmds, 3);
        let end = Tick(((whole.len() as f64) * cut) as u64).max(Tick(1));
        let block = whole.slice(TickRange::new(Tick::ZERO, end)).expect("a slice");
        let gap = gap.and_then(|(at, len)| {
            let start = ((block.len() as f64) * at) as u64;
            let end = (start + len).min(block.len());
            (start < end).then(|| TickRange::new(Tick(start), Tick(end)))
        });
        let items = reference(&block, gap);
        assert_settled_in_order(&items);
        let mut open: BTreeMap<OpId, u32> = BTreeMap::new();
        for event in items.iter().filter_map(Item::event) {
            match &event.payload {
                EepromEvent::Begin { op, .. } => {
                    prop_assert!(open.insert(*op, 0).is_none(), "{:?} began twice", op);
                }
                EepromEvent::Data { op, offset, bytes } => {
                    prop_assert!(bytes.len() <= DATA_BATCH);
                    let length = open.get_mut(op).expect("data of an open operation");
                    prop_assert_eq!(*offset, *length);
                    *length += bytes.len() as u32;
                }
                EepromEvent::End { op, length, .. } => {
                    prop_assert_eq!(open.remove(op), Some(*length), "{:?}", op);
                }
                EepromEvent::Address { op, .. } => prop_assert!(open.contains_key(op)),
                EepromEvent::AckPoll { .. } | EepromEvent::Unknown { .. } => {}
            }
        }
        prop_assert!(open.is_empty(), "operations left open: {:?}", open);

        let mut harness = build(GraphConfig { queue_capacity: 2, budget: 5 });
        harness.drain_on_full = 1;
        harness.feed_cut(&block, gap.as_slice(), &sizes, &pauses);
        harness.end();
        prop_assert_eq!(harness.without_progress(), items);
    }
}
