// SPDX-License-Identifier: Apache-2.0

//! The 93Cxx EEPROM decoder over Microwire, on synthesized waveforms.

use std::collections::BTreeMap;

use proptest::prelude::*;
use uiharu_core::testing::{Harness, Tap, assert_block_invariant, without_progress};
use uiharu_core::{GraphConfig, GraphError, InputRef, Item, LogicBlock, Tick, TickRange, TimeBase};
use uiharu_protocols::eeprom93xx::synth::{eral, erase, ewds, ewen, read, wral, write};
use uiharu_protocols::eeprom93xx::{
    DATA_BATCH, Direction, Eeprom93xx, Eeprom93xxConfig, Eeprom93xxEvent, Organisation,
};
use uiharu_protocols::microwire::synth::MicrowireWaveform;
use uiharu_protocols::microwire::{Microwire, MicrowireConfig, MicrowireEvent};
use uiharu_protocols::spi::{Spi, SpiConfig, SpiEvent};

const CHANNELS: u8 = 4;
const HALF: u64 = 5;

/// Source → Microwire → 93Cxx; the Microwire events are a tap.
fn build(
    bus: MicrowireConfig,
    eeprom: Eeprom93xxConfig,
    graph: GraphConfig,
) -> Result<(Harness<Eeprom93xxEvent>, Tap<MicrowireEvent>), GraphError> {
    let mut exposed = None;
    let mut harness = Harness::build(
        CHANNELS,
        TimeBase::new(1_000_000),
        graph,
        |graph, source| {
            let microwire = graph.add(Microwire::new(bus).expect("a valid configuration"));
            graph.connect(source.samples, Microwire::input(microwire))?;
            let device = graph.add(Eeprom93xx::new(eeprom).expect("a valid configuration"));
            graph.connect(Microwire::events(microwire), Eeprom93xx::input(device))?;
            exposed = Some(graph.expose("microwire", Microwire::events(microwire))?);
            Ok(Eeprom93xx::events(device))
        },
    )?;
    let tap = harness.tap(exposed.expect("the bus is exposed"));
    Ok((harness, tap))
}

fn new_waveform(bus: MicrowireConfig) -> MicrowireWaveform {
    let mut waveform = MicrowireWaveform::new(bus, CHANNELS, HALF, Tick::ZERO);
    waveform.idle(20);
    waveform
}

fn at(span: TickRange, settled: Tick) -> String {
    if span.is_empty() {
        format!("@{settled}")
    } else {
        format!("[{},{})@{settled}", span.start, span.end)
    }
}

/// One line per item, progress left out: it depends on how the samples were cut.
fn describe(items: &[Item<Eeprom93xxEvent>]) -> Vec<String> {
    without_progress(items)
        .iter()
        .map(|item| match item {
            Item::Event(event) => {
                let what = match &event.payload {
                    Eeprom93xxEvent::Instruction {
                        op,
                        tx,
                        command,
                        address,
                    } => {
                        let address = address.map_or("--".to_owned(), |a| format!("{a:02x}"));
                        format!("instruction op{} tx{} {command:?} {address}", op.0, tx.0)
                    }
                    Eeprom93xxEvent::Data {
                        op,
                        direction,
                        offset,
                        values,
                    } => {
                        let direction = match direction {
                            Direction::In => "in",
                            Direction::Out => "out",
                        };
                        let values: Vec<String> =
                            values.iter().map(|value| format!("{value:04x}")).collect();
                        format!("data op{} {direction} +{offset} {}", op.0, values.join(","))
                    }
                    Eeprom93xxEvent::Status { op, tx, ready } => {
                        let state = if *ready { "ready" } else { "busy" };
                        format!("status op{} tx{} {state}", op.0, tx.0)
                    }
                    Eeprom93xxEvent::End { op, reason } => format!("end op{} {reason:?}", op.0),
                };
                format!("{what} {}", at(event.span, event.settled))
            }
            Item::Gap(range) => format!("gap {range}"),
            Item::Progress(tick) => format!("progress {tick}"),
            Item::End(tick) => format!("eof {tick}"),
        })
        .collect()
}

fn decode_with(bus: MicrowireConfig, eeprom: Eeprom93xxConfig, block: &LogicBlock) -> Vec<String> {
    let (mut harness, _) = build(bus, eeprom, GraphConfig::default()).expect("builds");
    harness.run_all(block);
    describe(&harness.items)
}

fn decode(block: &LogicBlock) -> Vec<String> {
    decode_with(
        MicrowireConfig::default(),
        Eeprom93xxConfig::default(),
        block,
    )
}

/// The rising and falling edges of bit `n` of a transaction selected at `select`.
fn edges(select: u64, n: u64) -> (u64, u64) {
    let rise = select + 2 * HALF + 2 * HALF * n;
    (rise, rise + HALF)
}

/// Where a transaction of `bits` bits selected at `select` releases chip select.
fn release(select: u64, bits: u64) -> u64 {
    select + 2 * HALF + 2 * HALF * bits
}

#[test]
fn a_sequential_read_of_three_words() {
    let config = Eeprom93xxConfig::default();
    let mut waveform = new_waveform(MicrowireConfig::default());
    waveform.transaction(read(&config, 0x2a, &[0xaaaa, 0x5555, 0x1234]));
    waveform.idle(10);
    let block = waveform.finish();
    // The instruction's nine bits, then 48 bits of words; the dummy 0 rides on the ninth.
    let (_, address_end) = edges(20, 8);
    let (words_start, _) = edges(20, 9);
    let (_, words_end) = edges(20, 56);
    let end = release(20, 57);
    assert_eq!(
        decode(&block),
        vec![
            format!(
                "instruction op1 tx1 Read 2a [20,{})@{address_end}",
                address_end + 1
            ),
            format!(
                "data op1 out +0 aaaa,5555,1234 [{words_start},{})@{end}",
                words_end + 1
            ),
            format!("end op1 Complete [{end},{})@{end}", end + 1),
            format!("eof {}", end + 10),
        ]
    );
}

#[test]
fn a_write_and_the_status_polls_after_it() {
    let config = Eeprom93xxConfig::default();
    let mut waveform = new_waveform(MicrowireConfig::default());
    waveform.transaction(ewen(&config));
    waveform.idle(10);
    let write_at = waveform.position().get();
    waveform.transaction(write(&config, 0x05, 0xbeef));
    waveform.idle(10);
    // Polls: busy, then busy turning ready, then ready again.
    let busy_at = waveform.position().get();
    waveform.select();
    waveform.set_out(false);
    waveform.idle(20);
    waveform.release();
    waveform.idle(10);
    let ready_at = waveform.position().get();
    waveform.select();
    waveform.set_out(false);
    waveform.idle(10);
    waveform.set_out(true);
    waveform.idle(10);
    waveform.release();
    waveform.idle(10);
    waveform.select();
    waveform.idle(10);
    waveform.release();
    waveform.idle(10);
    let ewds_at = waveform.position().get();
    waveform.transaction(ewds(&config));
    waveform.idle(10);
    let lines = decode(&waveform.finish());
    let (_, write_address) = edges(write_at, 8);
    let (word_start, _) = edges(write_at, 9);
    let (_, word_end) = edges(write_at, 24);
    let write_end = release(write_at, 25);
    let busy_end = busy_at + 30;
    let ready_end = ready_at + 30;
    let ewds_end = release(ewds_at, 9);
    assert_eq!(
        lines,
        vec![
            "instruction op1 tx1 Ewen -- [20,116)@115".to_owned(),
            "end op1 Complete [120,121)@120".to_owned(),
            format!(
                "instruction op2 tx2 Write 05 [{write_at},{})@{write_address}",
                write_address + 1
            ),
            format!(
                "data op2 in +0 beef [{word_start},{})@{word_end}",
                word_end + 1
            ),
            format!(
                "end op2 Complete [{write_end},{})@{write_end}",
                write_end + 1
            ),
            format!(
                "status op2 tx3 busy [{busy_at},{})@{busy_end}",
                busy_end + 1
            ),
            format!(
                "status op2 tx4 ready [{ready_at},{})@{ready_end}",
                ready_end + 1
            ),
            format!(
                "instruction op3 tx6 Ewds -- [{ewds_at},{})@{}",
                edges(ewds_at, 8).1 + 1,
                edges(ewds_at, 8).1
            ),
            format!("end op3 Complete [{ewds_end},{})@{ewds_end}", ewds_end + 1),
            format!("eof {}", ewds_end + 10),
        ]
    );
}

#[test]
fn polls_that_clock_show_the_status_on_their_bits() {
    let config = Eeprom93xxConfig::default();
    let mut waveform = new_waveform(MicrowireConfig::default());
    waveform.transaction(erase(&config, 0x11));
    waveform.idle(10);
    // The controller clocks with DI low while the device is busy, then sends its next
    // instruction once DO is high: the start bit clears the status.
    let poll = waveform.position().get();
    waveform.select();
    waveform.bits([(false, false), (false, false), (false, true)]);
    waveform.bits(read(&config, 0x11, &[0xffff]));
    waveform.release();
    waveform.idle(10);
    let lines = decode(&waveform.finish());
    let (_, busy) = edges(poll, 0);
    let (rise, ready) = edges(poll, 2);
    assert_eq!(
        lines[2..5],
        [
            format!("status op1 tx2 busy [{poll},{})@{busy}", busy + 1),
            format!(
                "status op1 tx2 ready [{},{})@{ready}",
                edges(poll, 1).0,
                ready + 1
            ),
            format!(
                "instruction op2 tx2 Read 11 [{},{})@{}",
                rise + 2 * HALF,
                edges(poll, 11).1 + 1,
                edges(poll, 11).1
            ),
        ]
    );
    assert!(lines[5].starts_with("data op2 out +0 ffff "), "{lines:?}");
}

#[test]
fn every_instruction_of_the_table() {
    let config = Eeprom93xxConfig::default();
    let mut waveform = new_waveform(MicrowireConfig::default());
    for bits in [
        ewen(&config),
        erase(&config, 0x3f),
        eral(&config),
        wral(&config, 0x0f0f),
        ewds(&config),
        write(&config, 0, 0xffff),
    ] {
        waveform.transaction(bits);
        waveform.idle(10);
    }
    let lines: Vec<String> = decode(&waveform.finish())
        .iter()
        .map(|line| line.split(" [").next().unwrap_or_default().to_owned())
        .collect();
    assert_eq!(
        lines,
        vec![
            "instruction op1 tx1 Ewen --",
            "end op1 Complete",
            "instruction op2 tx2 Erase 3f",
            "end op2 Complete",
            "instruction op3 tx3 Eral --",
            "end op3 Complete",
            "instruction op4 tx4 Wral --",
            "data op4 in +0 0f0f",
            "end op4 Complete",
            "instruction op5 tx5 Ewds --",
            "end op5 Complete",
            "instruction op6 tx6 Write 00",
            "data op6 in +0 ffff",
            "end op6 Complete",
            "eof 1000",
        ]
    );
}

#[test]
fn word_and_address_widths_follow_the_configuration() {
    // A 93C46 in 8-bit organisation: 7 address bits, bytes.
    let x8 = Eeprom93xxConfig {
        address_bits: 7,
        organisation: Organisation::X8,
    };
    let mut waveform = new_waveform(MicrowireConfig::default());
    waveform.transaction(read(&x8, 0x7f, &[0x12, 0x34]));
    waveform.idle(10);
    waveform.transaction(write(&x8, 0x40, 0xa5));
    waveform.idle(10);
    let lines = decode_with(MicrowireConfig::default(), x8, &waveform.finish());
    let what: Vec<&str> = lines
        .iter()
        .map(|line| line.split(" [").next().unwrap_or_default())
        .collect();
    assert_eq!(
        what[..6],
        [
            "instruction op1 tx1 Read 7f",
            "data op1 out +0 0012,0034",
            "end op1 Complete",
            "instruction op2 tx2 Write 40",
            "data op2 in +0 00a5",
            "end op2 Complete",
        ]
    );

    // A 93C86: 10 address bits in 16-bit organisation, 11 in 8-bit.
    for (address_bits, organisation, address) in [
        (10, Organisation::X16, 0x3ff),
        (11, Organisation::X8, 0x5a5),
    ] {
        let config = Eeprom93xxConfig {
            address_bits,
            organisation,
        };
        let mut waveform = new_waveform(MicrowireConfig::default());
        waveform.transaction(read(&config, address, &[0x81]));
        waveform.idle(10);
        waveform.transaction(eral(&config));
        waveform.idle(10);
        let lines = decode_with(MicrowireConfig::default(), config, &waveform.finish());
        assert!(
            lines[0].starts_with(&format!("instruction op1 tx1 Read {address:02x} ")),
            "{lines:?}"
        );
        assert!(lines[1].starts_with("data op1 out +0 0081 "), "{lines:?}");
        assert!(
            lines[3].starts_with("instruction op2 tx2 Eral -- "),
            "{lines:?}"
        );
    }
}

#[test]
fn clocks_before_the_start_bit_are_skipped() {
    let config = Eeprom93xxConfig::default();
    let mut waveform = new_waveform(MicrowireConfig::default());
    waveform.select();
    waveform.bits([(false, true), (false, true), (false, true)]);
    waveform.bits(read(&config, 0x01, &[0xabcd]));
    waveform.release();
    waveform.idle(10);
    // A transaction with no start bit at all carries no instruction.
    waveform.transaction([(false, true), (false, true)]);
    waveform.idle(10);
    let lines = decode(&waveform.finish());
    // The instruction spans the transaction from chip select, the skipped clocks included.
    let (_, last) = edges(20, 11);
    let end = release(20, 28);
    assert_eq!(
        lines[0],
        format!("instruction op1 tx1 Read 01 [20,{})@{last}", last + 1)
    );
    assert!(lines[1].starts_with("data op1 out +0 abcd "), "{lines:?}");
    assert_eq!(
        lines[2],
        format!("end op1 Complete [{end},{})@{end}", end + 1)
    );
    assert_eq!(lines.len(), 4, "{lines:?}");
}

#[test]
fn instructions_cut_short_by_chip_select_are_incomplete() {
    let config = Eeprom93xxConfig::default();
    let read_bits = read(&config, 0x15, &[0x1111, 0x2222]);
    let write_bits = write(&config, 0x15, 0x3333);
    let mut waveform = new_waveform(MicrowireConfig::default());
    // A read released in its address, then right after it, then inside its second word.
    let mut selects = Vec::new();
    for cut in [6, 9, 30] {
        selects.push(waveform.position().get());
        waveform.transaction(read_bits[..cut].iter().copied());
        waveform.idle(10);
    }
    // A write released inside its word, and an instruction released inside its opcode.
    selects.push(waveform.position().get());
    waveform.transaction(write_bits[..20].iter().copied());
    waveform.idle(10);
    waveform.transaction(write_bits[..2].iter().copied());
    waveform.idle(10);
    let lines = decode(&waveform.finish());
    let release_of = |n: usize, bits: u64| release(selects[n], bits);
    let (word_start, _) = edges(selects[2], 9);
    let (_, word_end) = edges(selects[2], 24);
    let first = release_of(0, 6);
    assert_eq!(
        lines,
        vec![
            format!(
                "instruction op1 tx1 Read -- [{},{})@{first}",
                selects[0],
                first + 1
            ),
            format!("end op1 Incomplete @{first}"),
            format!(
                "instruction op2 tx2 Read 15 [{},{})@{}",
                selects[1],
                edges(selects[1], 8).1 + 1,
                edges(selects[1], 8).1
            ),
            format!(
                "end op2 Incomplete [{},{})@{}",
                release_of(1, 9),
                release_of(1, 9) + 1,
                release_of(1, 9)
            ),
            format!(
                "instruction op3 tx3 Read 15 [{},{})@{}",
                selects[2],
                edges(selects[2], 8).1 + 1,
                edges(selects[2], 8).1
            ),
            format!(
                "data op3 out +0 1111 [{word_start},{})@{}",
                word_end + 1,
                release_of(2, 30)
            ),
            format!(
                "end op3 Incomplete [{},{})@{}",
                edges(selects[2], 25).0,
                release_of(2, 30) + 1,
                release_of(2, 30)
            ),
            format!(
                "instruction op4 tx4 Write 15 [{},{})@{}",
                selects[3],
                edges(selects[3], 8).1 + 1,
                edges(selects[3], 8).1
            ),
            format!(
                "end op4 Incomplete [{},{})@{}",
                edges(selects[3], 9).0,
                release_of(3, 20) + 1,
                release_of(3, 20)
            ),
            format!("eof {}", release(release_of(3, 20) + 10, 2) + 10),
        ]
    );
}

#[test]
fn a_gap_aborts_the_instruction_and_the_transaction_after_it_is_skipped() {
    let config = Eeprom93xxConfig::default();
    let mut waveform = new_waveform(MicrowireConfig::default());
    waveform.transaction(read(&config, 0x30, &[0x0102, 0x0304, 0x0506]));
    waveform.idle(10);
    let next = waveform.position().get();
    waveform.transaction(erase(&config, 0x30));
    waveform.idle(10);
    let block = waveform.finish();
    // The gap swallows the end of the second word and the start of the third.
    let gap = TickRange::new(Tick(edges(20, 30).0), Tick(edges(20, 45).0));
    let (mut harness, _) =
        build(MicrowireConfig::default(), config, GraphConfig::default()).expect("builds");
    harness.feed_with_gaps(&block, &[gap], &[17]);
    harness.end();
    let lines = describe(&harness.items);
    let (word_start, _) = edges(20, 9);
    let (_, word_end) = edges(20, 24);
    assert_eq!(
        lines[1..4],
        [
            format!(
                "data op1 out +0 0102 [{word_start},{})@{}",
                word_end + 1,
                gap.start
            ),
            format!(
                "end op1 Aborted [{},{})@{}",
                edges(20, 25).0,
                gap.start,
                gap.start
            ),
            format!("gap {gap}"),
        ]
    );
    // The transaction found under way after the gap is skipped; the next one is read.
    assert!(
        lines[4].starts_with(&format!("instruction op2 tx3 Erase 30 [{next},")),
        "{lines:?}"
    );
    assert_eq!(lines.len(), 7, "{lines:?}");
}

#[test]
fn reads_without_a_do_line_report_no_words() {
    let config = Eeprom93xxConfig::default();
    let bus = MicrowireConfig {
        r#do: None,
        ..MicrowireConfig::default()
    };
    let mut waveform = new_waveform(bus);
    waveform.transaction(read(&config, 0x01, &[0xffff, 0x0000]));
    waveform.idle(10);
    let lines = decode_with(bus, config, &waveform.finish());
    let end = release(20, 41);
    assert_eq!(
        lines,
        vec![
            "instruction op1 tx1 Read 01 [20,111)@110".to_owned(),
            format!("end op1 Complete [120,{})@{end}", end + 1),
            format!("eof {}", end + 10),
        ]
    );
}

#[test]
fn long_reads_go_out_in_batches_as_they_stream() {
    let config = Eeprom93xxConfig::default();
    let words: Vec<u16> = (0..70).map(|n| n * 0x0101).collect();
    let mut waveform = new_waveform(MicrowireConfig::default());
    waveform.transaction(read(&config, 0, &words));
    waveform.idle(10);
    let block = waveform.finish();
    let (mut harness, _) =
        build(MicrowireConfig::default(), config, GraphConfig::default()).expect("builds");
    harness.run_all(&block);
    let batches: Vec<(u32, usize, Tick)> = harness
        .events()
        .iter()
        .filter_map(|event| match &event.payload {
            Eeprom93xxEvent::Data { offset, values, .. } => {
                Some((*offset, values.len(), event.settled))
            }
            _ => None,
        })
        .collect();
    let full = |words: u64| Tick(edges(20, 8 + 16 * words).1);
    assert_eq!(
        batches,
        vec![
            (0, DATA_BATCH, full(32)),
            (32, DATA_BATCH, full(64)),
            (64, 6, Tick(release(20, 9 + 16 * 70))),
        ]
    );
}

#[test]
fn partial_transactions_are_skipped() {
    let config = Eeprom93xxConfig::default();
    let mut waveform = new_waveform(MicrowireConfig::default());
    waveform.transaction(write(&config, 0x2b, 0x7777));
    waveform.idle(10);
    let next = waveform.position().get();
    waveform.transaction(read(&config, 0x2b, &[0x7777]));
    waveform.idle(10);
    let block = waveform.finish();
    let tail = block
        .slice(TickRange::new(Tick(23), block.end()))
        .expect("a slice");
    let lines = decode(&tail);
    assert!(
        lines[0].starts_with(&format!("instruction op1 tx2 Read 2b [{next},")),
        "{lines:?}"
    );
    assert_eq!(lines.len(), 4, "{lines:?}");
}

#[test]
fn streams_that_do_not_frame_bits_with_di_are_refused() {
    let unframed = MicrowireConfig {
        cs: None,
        ..MicrowireConfig::default()
    };
    match build(
        unframed,
        Eeprom93xxConfig::default(),
        GraphConfig::default(),
    ) {
        Err(GraphError::Requirement { key, .. }) => assert_eq!(key, "cs"),
        other => panic!("expected the bus to be refused, got {:?}", other.err()),
    }
    // An SPI stream is not a Microwire stream.
    let result = Harness::build(
        CHANNELS,
        TimeBase::new(1_000_000),
        GraphConfig::default(),
        |graph, source| {
            let spi = graph.add(Spi::new(SpiConfig::default()).expect("a valid configuration"));
            graph.connect(source.samples, Spi::input(spi))?;
            let device = graph.add(Eeprom93xx::new(Eeprom93xxConfig::default()).expect("valid"));
            // The typed helpers would not compile; the graph checks the schema at run time.
            let input = InputRef::<SpiEvent>::of_node(device, 0);
            graph.connect(Spi::events(spi), input)?;
            Ok(Eeprom93xx::events(device))
        },
    );
    assert!(
        matches!(result, Err(GraphError::SchemaMismatch { .. })),
        "{:?}",
        result.err()
    );
}

/// Instructions of every kind, with polls, and where each transaction begins.
fn busy_bus() -> (LogicBlock, Vec<u64>) {
    let config = Eeprom93xxConfig::default();
    let mut waveform = new_waveform(MicrowireConfig::default());
    let mut starts = Vec::new();
    for n in 0..4u16 {
        for bits in [
            ewen(&config),
            write(&config, n, 0x1111 * n),
            read(&config, n, &[n, n + 1, n + 2, n + 3]),
            erase(&config, n + 7),
            ewds(&config),
        ] {
            starts.push(waveform.position().get());
            waveform.transaction(bits);
            waveform.idle(7 + u64::from(n));
            if starts.len() % 5 == 2 {
                // A poll after the write.
                waveform.select();
                waveform.set_out(n % 2 == 0);
                waveform.idle(12);
                waveform.release();
                waveform.idle(5);
            }
        }
    }
    (waveform.finish(), starts)
}

#[test]
fn every_cut_and_pause_yields_the_same_events_in_both_layers() {
    let (block, starts) = busy_bus();
    let gaps = [
        // Inside a read's words, inside an instruction, and between transactions.
        TickRange::new(Tick(starts[7] + 150), Tick(starts[7] + 190)),
        TickRange::new(Tick(starts[11] + 40), Tick(starts[11] + 44)),
        TickRange::new(Tick(starts[15] - 4), Tick(starts[15] - 2)),
    ];
    let config = Eeprom93xxConfig::default();
    let (mut harness, _) =
        build(MicrowireConfig::default(), config, GraphConfig::default()).expect("builds");
    harness.feed_with_gaps(&block, &gaps, &[u64::MAX]);
    harness.end();
    let reference = describe(&harness.items);
    for kind in [
        "status op",
        " busy ",
        " ready ",
        "data op",
        " in +0 ",
        " Aborted ",
        " Complete ",
    ] {
        assert!(
            reference.iter().any(|line| line.contains(kind)),
            "{kind} in {reference:?}"
        );
    }
    assert_block_invariant(
        |graph| {
            build(MicrowireConfig::default(), config, graph)
                .expect("builds")
                .0
        },
        &block,
        &gaps,
    );
    assert_block_invariant(
        |graph| {
            Harness::build(
                CHANNELS,
                TimeBase::new(1_000_000),
                graph,
                |graph, source| {
                    let microwire = graph.add(
                        Microwire::new(MicrowireConfig::default()).expect("a valid configuration"),
                    );
                    graph.connect(source.samples, Microwire::input(microwire))?;
                    Ok(Microwire::events(microwire))
                },
            )
            .expect("builds")
        },
        &block,
        &gaps,
    );
}

#[test]
fn progress_never_passes_an_operation_not_yet_reported() {
    let (block, _) = busy_bus();
    for sizes in [&[1u64][..], &[7], &[64, 3]] {
        let (mut harness, _) = build(
            MicrowireConfig::default(),
            Eeprom93xxConfig::default(),
            GraphConfig::default(),
        )
        .expect("builds");
        harness.feed_cut(&block, &[], sizes, &[1]);
        harness.end();
        let mut seen = Tick::ZERO;
        for item in &harness.items {
            match item {
                Item::Progress(tick) => seen = seen.max(*tick),
                Item::Event(event) => {
                    assert!(event.span.start >= seen, "{event:?} after {seen}");
                }
                Item::Gap(_) | Item::End(_) => {}
            }
        }
        assert!(seen > Tick::ZERO, "blocks of {sizes:?} announce progress");
    }
}

// Random traffic.

/// One transaction of a random scenario.
#[derive(Clone, Debug)]
enum Step {
    Read {
        address: u16,
        words: Vec<u16>,
    },
    Write {
        address: u16,
        value: u16,
    },
    Erase {
        address: u16,
    },
    Ewen,
    Ewds,
    Eral,
    Wral {
        value: u16,
    },
    /// A read released after `keep` of its bits.
    Short {
        keep: usize,
    },
    /// Chip select asserted with DO at `level`, and `clocks` pulses with DI low.
    Poll {
        level: bool,
        clocks: u8,
    },
    /// Pulses with DI low before a read's start bit.
    Leading {
        clocks: u8,
    },
}

fn step() -> impl Strategy<Value = Step> {
    prop_oneof![
        4 => (any::<u16>(), prop::collection::vec(any::<u16>(), 0..5))
            .prop_map(|(address, words)| Step::Read { address, words }),
        3 => (any::<u16>(), any::<u16>()).prop_map(|(address, value)| Step::Write { address, value }),
        2 => any::<u16>().prop_map(|address| Step::Erase { address }),
        1 => Just(Step::Ewen),
        1 => Just(Step::Ewds),
        1 => Just(Step::Eral),
        1 => any::<u16>().prop_map(|value| Step::Wral { value }),
        2 => (0usize..40).prop_map(|keep| Step::Short { keep }),
        3 => (any::<bool>(), 0u8..4).prop_map(|(level, clocks)| Step::Poll { level, clocks }),
        1 => (1u8..5).prop_map(|clocks| Step::Leading { clocks }),
    ]
}

fn eeprom_config(wide: u8) -> Eeprom93xxConfig {
    let (address_bits, organisation) = match wide % 4 {
        0 => (6, Organisation::X16),
        1 => (7, Organisation::X8),
        2 => (8, Organisation::X16),
        _ => (11, Organisation::X8),
    };
    Eeprom93xxConfig {
        address_bits,
        organisation,
    }
}

fn traffic(steps: &[Step], config: &Eeprom93xxConfig, half: u64) -> LogicBlock {
    let mask = |value: u16, bits: u8| value & (u16::MAX >> (16 - bits));
    let address = |value: u16| mask(value, config.address_bits);
    let word = |value: u16| mask(value, config.organisation.data_bits());
    let mut waveform =
        MicrowireWaveform::new(MicrowireConfig::default(), CHANNELS, half, Tick::ZERO);
    waveform.idle(5);
    for (n, step) in steps.iter().enumerate() {
        match step {
            Step::Read { address: at, words } => {
                let words: Vec<u16> = words.iter().map(|value| word(*value)).collect();
                waveform.transaction(read(config, address(*at), &words));
            }
            Step::Write { address: at, value } => {
                waveform.transaction(write(config, address(*at), word(*value)));
            }
            Step::Erase { address: at } => waveform.transaction(erase(config, address(*at))),
            Step::Ewen => waveform.transaction(ewen(config)),
            Step::Ewds => waveform.transaction(ewds(config)),
            Step::Eral => waveform.transaction(eral(config)),
            Step::Wral { value } => waveform.transaction(wral(config, word(*value))),
            Step::Short { keep } => {
                let bits = read(config, 0x15, &[0x1234]);
                waveform.transaction(bits.into_iter().take(*keep));
            }
            Step::Poll { level, clocks } => {
                waveform.select();
                waveform.set_out(*level);
                waveform.idle(3);
                waveform.bits((0..*clocks).map(|_| (false, *level)));
                waveform.release();
            }
            Step::Leading { clocks } => {
                waveform.select();
                waveform.bits((0..*clocks).map(|_| (false, true)));
                waveform.bits(read(config, 0x2a, &[0xabcd]));
                waveform.release();
            }
        }
        waveform.idle(2 + (n as u64 % 4) * 3);
    }
    waveform.finish()
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: std::env::var("PROPTEST_CASES").ok().and_then(|cases| cases.parse().ok()).unwrap_or(48),
        ..ProptestConfig::default()
    })]

    /// Blocks of any size, a reader that pauses at random and tiny queues and budgets yield the
    /// events of one uninterrupted run in both layers; every operation that begins ends once,
    /// and the streams settle in order without progress passing what they still report.
    #[test]
    fn random_traffic_decodes_alike_however_it_comes(
        steps in prop::collection::vec(step(), 1..10),
        wide in any::<u8>(),
        half in 2u64..6,
        sizes in prop::collection::vec(1u64..300, 1..6),
        pauses in prop::collection::vec(0usize..12, 1..8),
        queue_capacity in 1usize..6,
        budget in 1u32..40,
        cut in prop::option::of(0.05f64..1.0),
        gap in prop::option::of((0.05f64..0.95, 1u64..200)),
    ) {
        let config = eeprom_config(wide);
        let whole = traffic(&steps, &config, half);
        let block = match cut {
            Some(cut) => {
                let end = Tick(((whole.len() as f64) * cut) as u64).max(Tick(1));
                whole.slice(TickRange::new(Tick::ZERO, end)).expect("a slice")
            }
            None => whole,
        };
        let gaps: Vec<TickRange> = gap
            .and_then(|(at, len)| {
                let start = ((block.len() as f64) * at) as u64;
                let end = (start + len).min(block.len());
                (start < end).then(|| TickRange::new(Tick(start), Tick(end)))
            })
            .into_iter()
            .collect();
        let bus = MicrowireConfig::default();
        let (reference, bus_reference) = {
            let (mut harness, tap) = build(bus, config, GraphConfig::default()).expect("builds");
            harness.feed_with_gaps(&block, &gaps, &[u64::MAX]);
            harness.end();
            (harness.without_progress(), tap.without_progress())
        };

        let (mut harness, tap) =
            build(bus, config, GraphConfig { queue_capacity, budget }).expect("builds");
        harness.drain_on_full = 1;
        harness.feed_cut(&block, &gaps, &sizes, &pauses);
        harness.end();
        prop_assert_eq!(harness.without_progress(), reference.clone());
        prop_assert_eq!(tap.without_progress(), bus_reference);
        let stats = harness.pipeline().stats();
        prop_assert!(stats.within_capacity(), "{stats:?}");

        let mut seen = Tick::ZERO;
        for item in &harness.items {
            match item {
                Item::Progress(tick) => seen = seen.max(*tick),
                Item::Event(event) => prop_assert!(event.span.start >= seen, "{event:?} after {seen}"),
                Item::Gap(_) | Item::End(_) => {}
            }
        }
        let mut settled = Tick::ZERO;
        let mut open = BTreeMap::new();
        for event in reference.iter().filter_map(Item::event) {
            prop_assert!(event.settled >= settled, "{event:?} settled before {settled}");
            settled = event.settled;
            match &event.payload {
                Eeprom93xxEvent::Instruction { op, .. } => {
                    prop_assert!(open.insert(*op, ()).is_none(), "{op:?} began twice");
                }
                Eeprom93xxEvent::Data { op, values, .. } => {
                    prop_assert!(open.contains_key(op));
                    prop_assert!(!values.is_empty() && values.len() <= DATA_BATCH);
                }
                Eeprom93xxEvent::End { op, .. } => {
                    prop_assert!(open.remove(op).is_some(), "{op:?} ended twice");
                }
                Eeprom93xxEvent::Status { .. } => {}
            }
        }
        prop_assert!(open.is_empty(), "operations left open: {open:?}");
    }
}
