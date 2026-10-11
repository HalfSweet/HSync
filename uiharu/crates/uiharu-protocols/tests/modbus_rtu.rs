// SPDX-License-Identifier: Apache-2.0

//! The Modbus RTU decoder on synthesized UART waveforms.

use std::collections::BTreeMap;
use std::sync::Arc;

use proptest::prelude::*;
use uiharu_core::testing::{Harness, assert_block_invariant, without_progress};
use uiharu_core::{
    GraphConfig, GraphError, Item, LogicBlock, SignalItem, Tick, TickRange, TimeBase,
};
use uiharu_protocols::modbus_rtu::{DATA_BATCH, EndReason, ModbusEvent, ModbusRtu};
use uiharu_protocols::uart::synth::UartWaveform;
use uiharu_protocols::uart::{BitOrder, Parity, Uart, UartConfig};

const CHANNELS: u8 = 1;
/// Ten samples per bit: 9600 baud sampled at 96 kHz.
const BIT: u64 = 10;
const BAUD: u32 = 9_600;
const RATE: u64 = 96_000;
/// A character of 8E1: eleven bits.
const CHAR: u64 = 11 * BIT;
/// 1.5 and 3.5 characters at this rate.
const T15: u64 = 165;
const T35: u64 = 385;

/// 8E1 at 9600 baud, the default of a Modbus serial line.
fn line() -> UartConfig {
    UartConfig {
        baud: BAUD,
        parity: Parity::Even,
        ..UartConfig::default()
    }
}

/// Source → UART → Modbus RTU, the Modbus frames being the output.
fn build(
    uart: UartConfig,
    rate: u64,
    graph_config: GraphConfig,
) -> Result<Harness<ModbusEvent>, GraphError> {
    Harness::build(
        CHANNELS,
        TimeBase::new(rate),
        graph_config,
        |graph, source| {
            let uart = graph.add(Uart::new(uart).expect("a valid configuration"));
            graph.connect(source.samples, Uart::input(uart))?;
            let modbus = graph.add(ModbusRtu::new());
            graph.connect(Uart::events(uart), ModbusRtu::input(modbus))?;
            Ok(ModbusRtu::events(modbus))
        },
    )
}

fn harness(uart: UartConfig, rate: u64) -> Harness<ModbusEvent> {
    build(uart, rate, GraphConfig::default()).expect("builds")
}

/// CRC-16/MODBUS the textbook way: the polynomial x¹⁶ + x¹⁵ + x² + 1 (0x8005) shifted most
/// significant bit first over bit-reversed bytes, the result reversed. It shares nothing with
/// the decoder's loop but the definition.
fn reference_crc(bytes: &[u8]) -> u16 {
    let mut crc: u16 = 0xffff;
    for byte in bytes {
        crc ^= u16::from(byte.reverse_bits()) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 {
                (crc << 1) ^ 0x8005
            } else {
                crc << 1
            };
        }
    }
    crc.reverse_bits()
}

/// An RTU frame: the address, the PDU, the reference CRC low byte first.
fn frame(address: u8, pdu: &[u8]) -> Vec<u8> {
    let mut bytes = vec![address];
    bytes.extend_from_slice(pdu);
    let crc = reference_crc(&bytes);
    bytes.extend_from_slice(&crc.to_le_bytes());
    bytes
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn at(span: TickRange, settled: Tick) -> String {
    if span.is_empty() {
        format!("@{settled}")
    } else {
        format!("[{},{})@{settled}", span.start, span.end)
    }
}

/// One line per item, readable in assertions.
fn describe(items: &[Item<ModbusEvent>]) -> Vec<String> {
    items
        .iter()
        .map(|item| match item {
            Item::Event(event) => {
                let what = match &event.payload {
                    ModbusEvent::Begin {
                        frame,
                        address,
                        function,
                        exception,
                        partial,
                    } => format!(
                        "begin f{} {address:02x} {}{}{}",
                        frame.0,
                        function.map_or("--".to_owned(), |code| format!("{code:02x}")),
                        if *exception { " exception" } else { "" },
                        if *partial { " partial" } else { "" },
                    ),
                    ModbusEvent::Data {
                        frame,
                        offset,
                        bytes,
                    } => format!("data f{} +{offset} {}", frame.0, hex(bytes)),
                    ModbusEvent::Crc { frame, value, ok } => format!(
                        "crc f{} {value:04x} {}",
                        frame.0,
                        if *ok { "ok" } else { "bad" }
                    ),
                    ModbusEvent::End { frame, reason } => format!("end f{} {reason:?}", frame.0),
                };
                format!("{what} {}", at(event.span, event.settled))
            }
            Item::Gap(range) => format!("gap {range}"),
            Item::Progress(tick) => format!("progress {tick}"),
            Item::End(tick) => format!("eof {tick}"),
        })
        .collect()
}

/// Decodes `block` in one piece: the lines, progress left out.
fn decode(block: &LogicBlock) -> Vec<String> {
    let mut harness = harness(line(), RATE);
    harness.run_all(block);
    describe(&harness.without_progress())
}

/// A waveform on the line, idle until `idle`.
fn waveform(idle: u64) -> UartWaveform {
    let mut waveform = UartWaveform::new(line(), CHANNELS, BIT, Tick::ZERO);
    waveform.idle(idle);
    waveform
}

/// The ticks of bytes `from..to` of a frame sent back to back from `start`.
fn bytes_at(start: u64, from: u64, to: u64) -> String {
    format!("[{},{})", start + from * CHAR, start + to * CHAR)
}

#[test]
fn the_reference_crc_matches_the_published_check_value() {
    assert_eq!(reference_crc(b"123456789"), 0x4b37);
}

#[test]
fn a_request_and_its_response() {
    let request = frame(0x11, &[0x03, 0x00, 0x6b, 0x00, 0x03]);
    let response = frame(0x11, &[0x03, 0x06, 0xae, 0x41, 0x56, 0x52, 0x43, 0x40]);
    // Server 17, three registers from 108, and its answer: the CRCs the specification shows.
    assert_eq!(hex(&request[6..]), "7687");
    assert_eq!(hex(&response[9..]), "49ad");
    let mut waveform = waveform(500);
    waveform.bytes(request);
    waveform.idle(400);
    waveform.bytes(response);
    waveform.idle(1000);
    let block = waveform.finish();
    // The request ends at 1380 and its silence at 1380 + 385; the response runs from 1780.
    assert_eq!(
        decode(&block),
        vec![
            format!("begin f1 11 03 {}@715", bytes_at(500, 0, 2)),
            format!("data f1 +0 006b0003 {}@1765", bytes_at(500, 2, 6)),
            format!("crc f1 8776 ok {}@1765", bytes_at(500, 6, 8)),
            "end f1 Complete @1765".to_owned(),
            format!("begin f2 11 03 {}@1995", bytes_at(1780, 0, 2)),
            format!("data f2 +0 06ae4156524340 {}@3375", bytes_at(1780, 2, 9)),
            format!("crc f2 ad49 ok {}@3375", bytes_at(1780, 9, 11)),
            "end f2 Complete @3375".to_owned(),
            "eof 3990".to_owned(),
        ]
    );
}

#[test]
fn an_exception_response() {
    // Illegal data address, in answer to function 3.
    let mut waveform = waveform(500);
    waveform.bytes(frame(0x11, &[0x83, 0x02]));
    waveform.idle(500);
    let crc = reference_crc(&[0x11, 0x83, 0x02]);
    assert_eq!(
        decode(&waveform.finish()),
        vec![
            format!("begin f1 11 83 exception {}@715", bytes_at(500, 0, 2)),
            format!("data f1 +0 02 {}@1435", bytes_at(500, 2, 3)),
            format!("crc f1 {crc:04x} ok {}@1435", bytes_at(500, 3, 5)),
            "end f1 Complete @1435".to_owned(),
            "eof 1550".to_owned(),
        ]
    );
}

#[test]
fn a_bad_crc_is_reported() {
    let mut bytes = frame(0x01, &[0x06, 0x00, 0x01, 0x00, 0x03]);
    bytes[7] ^= 0x01;
    let sent = u16::from_le_bytes([bytes[6], bytes[7]]);
    let mut waveform = waveform(500);
    waveform.bytes(bytes);
    waveform.idle(500);
    let lines = decode(&waveform.finish());
    assert_eq!(
        lines[2],
        format!("crc f1 {sent:04x} bad {}@1765", bytes_at(500, 6, 8))
    );
    assert_eq!(lines[3], "end f1 BadCrc @1765");
    assert_eq!(lines.len(), 5, "{lines:?}");
}

#[test]
fn frames_shorter_than_four_bytes_end_short() {
    let mut waveform = waveform(500);
    let mut starts = Vec::new();
    for bytes in [
        &[0x05][..],
        &[0x05, 0x01],
        &[0x05, 0x01, 0x02],
        &frame(0x05, &[0x07]),
    ] {
        starts.push(waveform.position().0);
        waveform.bytes(bytes.iter().copied());
        waveform.idle(T35 + 15);
    }
    let block = waveform.finish();
    let end = |n: usize, bytes: u64| starts[n] + bytes * CHAR + T35;
    let crc = reference_crc(&[0x05, 0x07]);
    assert_eq!(
        decode(&block),
        vec![
            // A single byte has no function code; its begin goes out when it ends.
            format!("begin f1 05 -- {}@{}", bytes_at(starts[0], 0, 1), end(0, 1)),
            format!("end f1 Short @{}", end(0, 1)),
            format!(
                "begin f2 05 01 {}@{}",
                bytes_at(starts[1], 0, 2),
                starts[1] + CHAR + 105
            ),
            format!("end f2 Short @{}", end(1, 2)),
            format!(
                "begin f3 05 01 {}@{}",
                bytes_at(starts[2], 0, 2),
                starts[2] + CHAR + 105
            ),
            format!("data f3 +0 02 {}@{}", bytes_at(starts[2], 2, 3), end(2, 3)),
            format!("end f3 Short @{}", end(2, 3)),
            format!(
                "begin f4 05 07 {}@{}",
                bytes_at(starts[3], 0, 2),
                starts[3] + CHAR + 105
            ),
            format!(
                "crc f4 {crc:04x} ok {}@{}",
                bytes_at(starts[3], 2, 4),
                end(3, 4)
            ),
            format!("end f4 Complete @{}", end(3, 4)),
            format!("eof {}", block.end()),
        ]
    );
}

#[test]
fn data_goes_out_in_batches_while_the_frame_is_under_way() {
    let data: Vec<u8> = (0..70u8).map(|i| i.wrapping_mul(37)).collect();
    let mut pdu = vec![0x10];
    pdu.extend_from_slice(&data);
    let bytes = frame(0x01, &pdu);
    let mut waveform = waveform(500);
    waveform.bytes(bytes.iter().copied());
    waveform.idle(1000);
    let block = waveform.finish();
    // Feed up to the middle of byte 40 of the frame.
    let cut = Tick(500 + 40 * CHAR + 50);
    let head = block
        .slice(TickRange::new(Tick::ZERO, cut))
        .expect("a slice");
    let mut harness = harness(line(), RATE);
    harness.feed(&head, &[97]);
    harness.collect();
    let lines = describe(&harness.without_progress());
    // The first batch, data bytes 0 to 31 (bytes 2 to 33 of the frame), is known to be data
    // once byte 35 has arrived, and settles with it.
    assert_eq!(
        lines,
        vec![
            format!("begin f1 01 10 {}@715", bytes_at(500, 0, 2)),
            format!(
                "data f1 +0 {} {}@{}",
                hex(&data[..32]),
                bytes_at(500, 2, 34),
                500 + 35 * CHAR + 105
            ),
        ]
    );
    let tail = block
        .slice(TickRange::new(cut, block.end()))
        .expect("a slice");
    harness.feed(&tail, &[97]);
    harness.end();
    let lines = describe(&harness.without_progress());
    let end = 500 + 74 * CHAR + T35;
    let crc = u16::from_le_bytes([bytes[72], bytes[73]]);
    assert_eq!(
        lines[2..],
        [
            format!(
                "data f1 +32 {} {}@{}",
                hex(&data[32..64]),
                bytes_at(500, 34, 66),
                500 + 67 * CHAR + 105
            ),
            format!(
                "data f1 +64 {} {}@{end}",
                hex(&data[64..]),
                bytes_at(500, 66, 72)
            ),
            format!("crc f1 {crc:04x} ok {}@{end}", bytes_at(500, 72, 74)),
            format!("end f1 Complete @{end}"),
            format!("eof {}", block.end()),
        ]
    );
    let batches: Vec<usize> = harness
        .events()
        .iter()
        .filter_map(|event| match &event.payload {
            ModbusEvent::Data { bytes, .. } => Some(bytes.len()),
            _ => None,
        })
        .collect();
    assert!(batches.iter().all(|len| *len <= DATA_BATCH));
}

#[test]
fn a_frame_ends_on_progress_before_the_next_frame_arrives() {
    let mut waveform = waveform(500);
    waveform.bytes(frame(0x11, &[0x03, 0x00, 0x6b, 0x00, 0x03]));
    waveform.idle(600);
    let response_at = waveform.position();
    waveform.bytes(frame(0x11, &[0x83, 0x02]));
    waveform.idle(600);
    let block = waveform.finish();
    let reference = decode(&block);
    // Up to just before the response: the request's silence passed at 1765, which progress
    // tells the decoder although no character follows yet.
    let head = block
        .slice(TickRange::new(Tick::ZERO, response_at))
        .expect("a slice");
    let mut harness = harness(line(), RATE);
    harness.feed(&head, &[200]);
    harness.collect();
    let lines = describe(&harness.without_progress());
    assert_eq!(
        lines.last().map(String::as_str),
        Some("end f1 Complete @1765")
    );
    assert_eq!(lines[..], reference[..4]);
    // In one piece the response's first byte is what ends the request; the events are the same.
    let tail = block
        .slice(TickRange::new(response_at, block.end()))
        .expect("a slice");
    harness.feed(&tail, &[200]);
    harness.end();
    assert_eq!(describe(&harness.without_progress()), reference);
}

/// A request whose fourth byte follows the third after `silence` ticks.
fn request_with_a_pause(silence: u64) -> (LogicBlock, Vec<u8>) {
    let bytes = frame(0x11, &[0x03, 0x00, 0x6b, 0x00, 0x03]);
    let mut waveform = waveform(500);
    waveform.bytes(bytes[..3].iter().copied());
    waveform.idle(silence);
    waveform.bytes(bytes[3..].iter().copied());
    waveform.idle(1000);
    (waveform.finish(), bytes)
}

#[test]
fn a_pause_of_one_and_a_half_characters_breaks_the_frame() {
    // Less than t1.5: a whole frame.
    let (block, _) = request_with_a_pause(T15 - 1);
    let lines = decode(&block);
    assert_eq!(
        lines[3],
        format!("end f1 Complete @{}", 500 + 8 * CHAR + T15 - 1 + T35)
    );
    // t1.5 and up to just under t3.5: one frame, broken, its CRC still checked.
    for silence in [T15, T35 - 1] {
        let (block, _) = request_with_a_pause(silence);
        let lines = decode(&block);
        let end = 500 + 8 * CHAR + silence + T35;
        assert_eq!(
            lines[2],
            format!("crc f1 8776 ok {}@{end}", bytes_at(500 + silence, 6, 8))
        );
        assert_eq!(lines[3], format!("end f1 Broken @{end}"));
        assert_eq!(lines.len(), 5, "{lines:?}");
    }
    // t3.5: two frames, both too short or wrong.
    let (block, _) = request_with_a_pause(T35);
    let first_end = 500 + 3 * CHAR + T35;
    let second = 500 + 3 * CHAR + T35;
    let second_end = second + 5 * CHAR + T35;
    assert_eq!(
        decode(&block),
        vec![
            format!("begin f1 11 03 {}@715", bytes_at(500, 0, 2)),
            format!("data f1 +0 00 {}@{first_end}", bytes_at(500, 2, 3)),
            format!("end f1 Short @{first_end}"),
            format!(
                "begin f2 6b 00 {}@{}",
                bytes_at(second, 0, 2),
                second + CHAR + 105
            ),
            format!("data f2 +0 03 {}@{second_end}", bytes_at(second, 2, 3)),
            format!("crc f2 8776 bad {}@{second_end}", bytes_at(second, 3, 5)),
            format!("end f2 BadCrc @{second_end}"),
            format!("eof {}", block.end()),
        ]
    );
}

#[test]
fn above_19200_baud_the_silences_are_fixed() {
    // 115 200 baud at ten samples per bit: 750 µs and 1.75 ms are 864 and 2016 ticks, where
    // counting characters would give 165 and 385.
    let fast = UartConfig {
        baud: 115_200,
        ..line()
    };
    let rate = 1_152_000;
    let bytes = frame(0x11, &[0x03, 0x00, 0x6b, 0x00, 0x03]);
    let decode_with_pause = |silence: u64| {
        let mut waveform = UartWaveform::new(fast, CHANNELS, BIT, Tick::ZERO);
        waveform.idle(3000);
        waveform.bytes(bytes[..3].iter().copied());
        waveform.idle(silence);
        waveform.bytes(bytes[3..].iter().copied());
        waveform.idle(3000);
        let mut harness = harness(fast, rate);
        harness.run_all(&waveform.finish());
        harness
            .events()
            .iter()
            .filter_map(|event| match event.payload {
                ModbusEvent::End { reason, .. } => Some(reason),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(decode_with_pause(863), vec![EndReason::Complete]);
    assert_eq!(decode_with_pause(864), vec![EndReason::Broken]);
    assert_eq!(decode_with_pause(2015), vec![EndReason::Broken]);
    assert_eq!(
        decode_with_pause(2016),
        vec![EndReason::Short, EndReason::BadCrc]
    );
}

#[test]
fn a_break_inside_a_frame_breaks_it() {
    let bytes = frame(0x11, &[0x03, 0x00, 0x6b, 0x00, 0x03]);
    let mut waveform = waveform(500);
    waveform.bytes(bytes[..4].iter().copied());
    waveform.break_(300);
    waveform.idle(50);
    waveform.bytes(bytes[4..].iter().copied());
    // A break after the silence leaves the frame before it alone.
    waveform.idle(T35 + 100);
    waveform.break_(300);
    waveform.idle(500);
    waveform.bytes(bytes.iter().copied());
    waveform.idle(500);
    let block = waveform.finish();
    let reasons: Vec<EndReason> = {
        let mut harness = harness(line(), RATE);
        harness.run_all(&block);
        harness
            .events()
            .iter()
            .filter_map(|event| match event.payload {
                ModbusEvent::End { reason, .. } => Some(reason),
                _ => None,
            })
            .collect()
    };
    assert_eq!(reasons, vec![EndReason::Broken, EndReason::Complete]);
    // The broken frame is measured from the end of the break: it ends t3.5 after its last byte.
    let lines = decode(&block);
    let last_byte_end = 500 + 4 * CHAR + 300 + 50 + 4 * CHAR;
    assert_eq!(lines[3], format!("end f1 Broken @{}", last_byte_end + T35));
}

#[test]
fn a_gap_aborts_the_frame_and_the_next_one_is_partial() {
    let mut waveform = waveform(500);
    waveform.bytes(frame(0x11, &[0x03, 0x00, 0x6b, 0x00, 0x03]));
    waveform.idle(1000);
    let block = waveform.finish();
    // Inside byte 4, 0x00, after its first data bit: the rest of it reads as no start bit.
    let gap = TickRange::new(Tick(960), Tick(990));
    let mut harness = harness(line(), RATE);
    harness.feed_with_gaps(&block, &[gap], &[u64::MAX]);
    harness.end();
    assert_eq!(
        describe(&harness.without_progress()),
        vec![
            format!("begin f1 11 03 {}@715", bytes_at(500, 0, 2)),
            format!("data f1 +0 006b {}@960", bytes_at(500, 2, 4)),
            "end f1 Aborted @960".to_owned(),
            "gap 960..990".to_owned(),
            // Bytes 5 to 7 begin 60 ticks after the gap: less than t3.5.
            format!("begin f2 03 76 partial {}@1265", bytes_at(1050, 0, 2)),
            format!("data f2 +0 87 {}@1765", bytes_at(1050, 2, 3)),
            "end f2 Short @1765".to_owned(),
            "eof 2380".to_owned(),
        ]
    );
}

#[test]
fn a_gap_after_the_silence_leaves_the_frame_whole() {
    let mut waveform = waveform(500);
    waveform.bytes(frame(0x11, &[0x83, 0x02]));
    waveform.idle(1000);
    waveform.bytes(frame(0x11, &[0x83, 0x02]));
    waveform.idle(500);
    let block = waveform.finish();
    // The first frame's silence passes at 1435; the gap starts later.
    let gap = TickRange::new(Tick(1500), Tick(1600));
    let mut harness = harness(line(), RATE);
    harness.feed_with_gaps(&block, &[gap], &[u64::MAX]);
    harness.end();
    let lines = describe(&harness.without_progress());
    assert_eq!(lines[3], "end f1 Complete @1435");
    assert_eq!(lines[4], "gap 1500..1600");
    // The next frame begins at 2050, well after the gap.
    assert!(
        lines[5].starts_with("begin f2 11 83 exception ["),
        "{lines:?}"
    );
    assert_eq!(lines[8], "end f2 Complete @2985");
}

#[test]
fn the_end_of_the_samples_before_the_silence_aborts_the_frame() {
    let bytes = frame(0x11, &[0x03, 0x00, 0x6b, 0x00, 0x03]);
    let mut waveform = waveform(500);
    waveform.bytes(bytes.iter().copied());
    waveform.idle(1000);
    let block = waveform.finish();
    // The CRC ends at 1380; its silence would pass at 1765.
    let short = block
        .slice(TickRange::new(Tick::ZERO, Tick(1480)))
        .expect("a slice");
    assert_eq!(
        decode(&short),
        vec![
            format!("begin f1 11 03 {}@715", bytes_at(500, 0, 2)),
            // Without the silence the last two bytes are not known to be a CRC.
            format!("data f1 +0 006b00037687 {}@1480", bytes_at(500, 2, 8)),
            "end f1 Aborted @1480".to_owned(),
            "eof 1480".to_owned(),
        ]
    );
    let long_enough = block
        .slice(TickRange::new(Tick::ZERO, Tick(1765)))
        .expect("a slice");
    let lines = decode(&long_enough);
    assert_eq!(lines[3], "end f1 Complete @1765");
}

#[test]
fn a_frame_under_way_when_the_samples_begin_is_partial() {
    let mut waveform = waveform(500);
    waveform.bytes(frame(0x11, &[0x03, 0x00, 0x6b, 0x00, 0x03]));
    waveform.idle(1000);
    waveform.bytes(frame(0x11, &[0x83, 0x02]));
    waveform.idle(500);
    let block = waveform.finish();
    // From the middle of byte 2 (0x00, at space); the samples are counted from there.
    let from = Tick(500 + 2 * CHAR + 55);
    let tail = block
        .slice(TickRange::new(from, block.end()))
        .expect("a slice")
        .rebased(Tick::ZERO);
    let lines = decode(&tail);
    // Byte 3 is the first whole character, 55 ticks in.
    assert_eq!(
        lines[0],
        format!(
            "begin f1 6b 00 partial [55,{})@{}",
            55 + 2 * CHAR,
            55 + CHAR + 105
        )
    );
    assert_eq!(lines[3], format!("end f1 BadCrc @{}", 55 + 5 * CHAR + T35));
    assert!(
        lines[4].starts_with("begin f2 11 83 exception ["),
        "{lines:?}"
    );
    // A frame that starts t3.5 or more into the samples is whole.
    assert!(!decode(&block)[0].contains("partial"));
}

#[test]
fn the_uart_stream_must_carry_8_bit_characters_lsb_first() {
    let seven = UartConfig {
        data_bits: 7,
        ..line()
    };
    let error = build(seven, RATE, GraphConfig::default()).expect_err("rejected");
    assert_eq!(
        error,
        GraphError::Requirement {
            from: "uart#0.events".into(),
            to: "modbus-rtu#1.uart".into(),
            key: "dataBits",
            requirement: "= 8".into(),
            found: "7".into(),
        }
    );
    let nine = UartConfig {
        data_bits: 9,
        ..line()
    };
    assert!(matches!(
        build(nine, RATE, GraphConfig::default()).err(),
        Some(GraphError::Requirement {
            key: "dataBits",
            ..
        })
    ));
    let msb_first = UartConfig {
        bit_order: BitOrder::MsbFirst,
        ..line()
    };
    assert!(matches!(
        build(msb_first, RATE, GraphConfig::default()).err(),
        Some(GraphError::Requirement {
            key: "bitOrder",
            ..
        })
    ));
    // No parity with two stop bits, or none with one, as many devices use, is fine.
    let no_parity = UartConfig {
        parity: Parity::None,
        ..line()
    };
    assert!(build(no_parity, RATE, GraphConfig::default()).is_ok());
}

/// Frames with every kind of timing: whole, broken by a pause, with a bad CRC, separated by
/// silences just long enough or longer. Returns where each frame starts and its last byte ends.
fn busy_line(waveform: &mut UartWaveform) -> Vec<(Tick, Tick)> {
    let mut frames = Vec::new();
    for n in 0..12u8 {
        let start = waveform.position();
        let pdu: Vec<u8> = std::iter::once(0x03 + n % 4)
            .chain((0..n * 3).map(|i| i.wrapping_mul(29) ^ n))
            .collect();
        let bytes = frame(n % 5, &pdu);
        match n % 4 {
            1 => {
                let half = bytes.len() / 2;
                waveform.bytes(bytes[..half].iter().copied());
                waveform.idle(T15 + u64::from(n));
                waveform.bytes(bytes[half..].iter().copied());
            }
            2 => waveform.bytes(bytes[..bytes.len() - 1].iter().copied().chain([0x55])),
            _ => waveform.bytes(bytes),
        }
        frames.push((start, waveform.position()));
        waveform.idle(match n % 3 {
            0 => T35 + 1,
            1 => T35 + 200,
            _ => 3 * T35,
        });
    }
    frames
}

#[test]
fn every_cut_and_pause_yields_the_same_frames() {
    let mut waveform = waveform(400);
    let frames = busy_line(&mut waveform);
    let block = waveform.finish();
    // In the silence after frame 2 before it reached t3.5, in the silence after frame 4 once it
    // had, inside frame 9.
    let gaps = [
        TickRange::new(frames[2].1.offset(50), frames[2].1.offset(100)),
        TickRange::new(frames[4].1.offset(T35 + 20), frames[4].1.offset(T35 + 90)),
        TickRange::new(frames[9].0.offset(CHAR / 2), frames[9].0.offset(CHAR)),
    ];
    let reference = {
        let mut harness = harness(line(), RATE);
        harness.feed_with_gaps(&block, &gaps, &[u64::MAX]);
        harness.end();
        harness.without_progress()
    };
    let reasons: Vec<EndReason> = reference
        .iter()
        .filter_map(Item::event)
        .filter_map(|event| match event.payload {
            ModbusEvent::End { reason, .. } => Some(reason),
            _ => None,
        })
        .collect();
    for reason in [
        EndReason::Complete,
        EndReason::BadCrc,
        EndReason::Broken,
        EndReason::Aborted,
    ] {
        assert!(reasons.contains(&reason), "{reason:?} in {reasons:?}");
    }
    assert_block_invariant(
        |graph_config| build(line(), RATE, graph_config).expect("builds"),
        &block,
        &gaps,
    );
    // The UART stream below is just as stable.
    assert_block_invariant(
        |graph_config| {
            Harness::build(
                CHANNELS,
                TimeBase::new(RATE),
                graph_config,
                |graph, source| {
                    let uart = graph.add(Uart::new(line()).expect("a valid configuration"));
                    graph.connect(source.samples, Uart::input(uart))?;
                    Ok(Uart::events(uart))
                },
            )
            .expect("builds")
        },
        &block,
        &gaps,
    );
}

#[test]
fn progress_never_passes_a_byte_not_yet_reported() {
    let mut waveform = waveform(400);
    busy_line(&mut waveform);
    let block = waveform.finish();
    for sizes in [&[1u64][..], &[13], &[64, 7]] {
        let mut harness = harness(line(), RATE);
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
    }
}

/// One frame of a random scenario.
#[derive(Clone, Debug)]
struct Scenario {
    address: u8,
    function: u8,
    data: Vec<u8>,
    /// A pause inside the frame: after which byte, how long.
    pause: Option<(usize, u64)>,
    corrupt: bool,
    /// The idle time after the frame.
    idle: u64,
}

fn scenario() -> impl Strategy<Value = Scenario> {
    (
        any::<u8>(),
        any::<u8>(),
        prop::collection::vec(any::<u8>(), 0..40),
        prop::option::weighted(0.2, (0usize..8, 0u64..2 * T35)),
        prop::bool::weighted(0.2),
        prop_oneof![3 => T35..3 * T35, 1 => 0..T35],
    )
        .prop_map(|(address, function, data, pause, corrupt, idle)| Scenario {
            address,
            function,
            data,
            pause,
            corrupt,
            idle,
        })
}

fn busy_waveform(scenarios: &[Scenario]) -> LogicBlock {
    let mut waveform = waveform(T35 + 7);
    for scenario in scenarios {
        let mut pdu = vec![scenario.function];
        pdu.extend_from_slice(&scenario.data);
        let mut bytes = frame(scenario.address, &pdu);
        if scenario.corrupt
            && let Some(last) = bytes.last_mut()
        {
            *last ^= 0x5a;
        }
        match scenario.pause {
            Some((after, silence)) if after < bytes.len() => {
                waveform.bytes(bytes[..=after].iter().copied());
                waveform.idle(silence);
                waveform.bytes(bytes[after + 1..].iter().copied());
            }
            _ => waveform.bytes(bytes),
        }
        waveform.idle(scenario.idle);
    }
    waveform.finish()
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 48, ..ProptestConfig::default() })]

    /// Blocks of any size, a reader that pauses and tiny queues and budgets all yield the frames
    /// of one uninterrupted run; every frame that begins ends once, and only a matching CRC
    /// makes it complete.
    #[test]
    fn frames_are_the_same_however_the_samples_come(
        scenarios in prop::collection::vec(scenario(), 1..8),
        sizes in prop::collection::vec(1u64..500, 1..6),
        pauses in prop::collection::vec(0usize..12, 1..8),
        queue_capacity in 1usize..6,
        budget in 1u32..40,
        cut in prop::option::of(0.05f64..1.0),
        gap in prop::option::of((0.05f64..0.95, 1u64..400)),
    ) {
        let whole = busy_waveform(&scenarios);
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
        let reference = {
            let mut harness = harness(line(), RATE);
            match gaps.first() {
                Some(_) => harness.feed_with_gaps(&block, &gaps, &[u64::MAX]),
                None => harness.push(SignalItem::Data(Arc::new(block.clone()))),
            }
            harness.end();
            harness.without_progress()
        };

        let mut harness = build(line(), RATE, GraphConfig { queue_capacity, budget })
            .expect("builds");
        harness.drain_on_full = 1;
        harness.feed_cut(&block, &gaps, &sizes, &pauses);
        harness.end();
        prop_assert_eq!(without_progress(&harness.items), reference.clone());
        let stats = harness.pipeline().stats();
        prop_assert!(stats.within_capacity(), "{stats:?}");

        let mut open = BTreeMap::new();
        for event in reference.iter().filter_map(Item::event) {
            match &event.payload {
                ModbusEvent::Begin { frame, .. } => {
                    prop_assert!(open.insert(*frame, None).is_none(), "{frame:?} began twice");
                }
                ModbusEvent::Crc { frame, ok, .. } => {
                    prop_assert!(open.contains_key(frame));
                    open.insert(*frame, Some(*ok));
                }
                ModbusEvent::Data { frame, bytes, .. } => {
                    prop_assert!(open.contains_key(frame));
                    prop_assert!(!bytes.is_empty() && bytes.len() <= DATA_BATCH);
                }
                ModbusEvent::End { frame, reason } => {
                    let crc = open.remove(frame).expect("an open frame");
                    match reason {
                        EndReason::Complete => prop_assert_eq!(crc, Some(true)),
                        EndReason::BadCrc => prop_assert_eq!(crc, Some(false)),
                        EndReason::Short | EndReason::Aborted => prop_assert_eq!(crc, None),
                        EndReason::Broken => {}
                    }
                }
            }
        }
        prop_assert!(open.is_empty(), "frames left open: {open:?}");
    }
}
