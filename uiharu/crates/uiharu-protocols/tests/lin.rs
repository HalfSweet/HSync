// SPDX-License-Identifier: Apache-2.0

//! The LIN decoder on synthesized UART waveforms.

use std::collections::BTreeMap;
use std::sync::Arc;

use proptest::prelude::*;
use uiharu_core::testing::{Harness, assert_block_invariant, without_progress};
use uiharu_core::{
    GraphConfig, GraphError, Item, LogicBlock, SignalItem, Tick, TickRange, TimeBase,
};
use uiharu_protocols::lin::synth;
use uiharu_protocols::lin::{
    DATA_BYTES, EndReason, ErrorReason, FrameId, Lin, LinConfig, LinEvent,
};
use uiharu_protocols::uart::synth::{Faults, UartWaveform};
use uiharu_protocols::uart::{BitOrder, Parity, StopBits, Uart, UartConfig};

const CHANNELS: u8 = 1;
/// Ten samples per bit: 19200 baud sampled at 192 kHz.
const BIT: u64 = 10;
const BAUD: u32 = 19_200;
const RATE: u64 = 192_000;
/// A byte field: a start bit, eight data bits and a stop bit.
const CHAR: u64 = 10 * BIT;
/// A break field of 13 bit times, and its delimiter of one.
const BREAK: u64 = 13 * BIT;
const DELIMITER: u64 = BIT;
/// From the start of a break field to the end of the protected identifier.
const HEADER: u64 = BREAK + DELIMITER + 2 * CHAR;
/// Where in a byte field its stop bit is read: nine and a half bits after its start.
const STOP: u64 = 95;

/// Frame identifiers and their protected identifiers, worked out by hand from the parity
/// equations P0 = ID0 ⊕ ID1 ⊕ ID2 ⊕ ID4 and P1 = ¬(ID1 ⊕ ID3 ⊕ ID4 ⊕ ID5).
const PIDS: [(u8, u8); 8] = [
    // No identifier bit set: P0 = 0, P1 = ¬0 = 1.
    (0x00, 0x80),
    // ID0: P0 = 1, P1 = ¬0 = 1.
    (0x01, 0xc1),
    // ID4: P0 = 1, P1 = ¬1 = 0.
    (0x10, 0x50),
    // ID1, ID2, ID4: P0 = 1 ⊕ 1 ⊕ 1 = 1, P1 = ¬(1 ⊕ 1) = 1.
    (0x16, 0xd6),
    // ID1, ID3, ID5: P0 = 1, P1 = ¬(1 ⊕ 1 ⊕ 1) = 0.
    (0x2a, 0x6a),
    // ID0, ID1, ID3, ID4, ID5: P0 = 1 ⊕ 1 ⊕ 1 = 1, P1 = ¬(1 ⊕ 1 ⊕ 1 ⊕ 1) = 1.
    (0x3b, 0xfb),
    // The master request frame, ID2 to ID5: P0 = 1 ⊕ 1 = 0, P1 = ¬(1 ⊕ 1 ⊕ 1) = 0.
    (0x3c, 0x3c),
    // The slave response frame, ID0 and ID2 to ID5: P0 = 1 ⊕ 1 ⊕ 1 = 1, P1 = 0.
    (0x3d, 0x7d),
];
const PID_10: u8 = 0x50;
const PID_2A: u8 = 0x6a;
const PID_3C: u8 = 0x3c;

/// 8N1 at 19200 baud: LIN's byte field.
fn line() -> UartConfig {
    UartConfig {
        baud: BAUD,
        ..UartConfig::default()
    }
}

/// Source → UART → LIN, the LIN frames being the output.
fn build(
    uart: UartConfig,
    lin: LinConfig,
    graph_config: GraphConfig,
) -> Result<Harness<LinEvent>, GraphError> {
    Harness::build(
        CHANNELS,
        TimeBase::new(RATE),
        graph_config,
        |graph, source| {
            let uart = graph.add(Uart::new(uart).expect("a valid configuration"));
            graph.connect(source.samples, Uart::input(uart))?;
            let lin = graph.add(Lin::new(lin).expect("a valid configuration"));
            graph.connect(Uart::events(uart), Lin::input(lin))?;
            Ok(Lin::events(lin))
        },
    )
}

fn harness(lin: LinConfig) -> Harness<LinEvent> {
    build(line(), lin, GraphConfig::default()).expect("builds")
}

/// The LIN checksum worked out apart from the decoder: the plain sum of the bytes, its carries
/// folded back into eight bits, inverted.
fn reference_checksum(bytes: &[u8]) -> u8 {
    let mut sum: u32 = bytes.iter().map(|byte| u32::from(*byte)).sum();
    while sum > 0xff {
        sum = (sum & 0xff) + (sum >> 8);
    }
    !u8::try_from(sum).expect("folded into eight bits")
}

/// The classic checksum: over the data bytes.
fn classic(data: &[u8]) -> u8 {
    reference_checksum(data)
}

/// The enhanced checksum: over the protected identifier and the data bytes.
fn enhanced(pid: u8, data: &[u8]) -> u8 {
    let mut bytes = vec![pid];
    bytes.extend_from_slice(data);
    reference_checksum(&bytes)
}

/// `data` followed by `checksum`.
fn with_checksum(data: &[u8], checksum: u8) -> Vec<u8> {
    let mut bytes = data.to_vec();
    bytes.push(checksum);
    bytes
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// One line per item, readable in assertions.
fn describe(items: &[Item<LinEvent>]) -> Vec<String> {
    items
        .iter()
        .map(|item| match item {
            Item::Event(event) => {
                let what = match &event.payload {
                    LinEvent::Header {
                        frame,
                        pid,
                        id,
                        pid_ok,
                    } => format!(
                        "header f{} {pid:02x} id {id:02x}{}",
                        frame.0,
                        if *pid_ok { "" } else { " parity-error" }
                    ),
                    LinEvent::Data {
                        frame,
                        offset,
                        bytes,
                    } => format!("data f{} +{offset} {}", frame.0, hex(bytes)),
                    LinEvent::Checksum {
                        frame,
                        value,
                        classic_ok,
                        enhanced_ok,
                    } => format!(
                        "checksum f{} {value:02x} {}",
                        frame.0,
                        match (classic_ok, enhanced_ok) {
                            (true, true) => "both",
                            (true, false) => "classic",
                            (false, true) => "enhanced",
                            (false, false) => "neither",
                        }
                    ),
                    LinEvent::Error { frame, reason } => format!("error f{} {reason:?}", frame.0),
                    LinEvent::End { frame, reason } => format!("end f{} {reason:?}", frame.0),
                };
                format!(
                    "{what} [{},{})@{}",
                    event.span.start, event.span.end, event.settled
                )
            }
            Item::Gap(range) => format!("gap {range}"),
            Item::Progress(tick) => format!("progress {tick}"),
            Item::End(tick) => format!("eof {tick}"),
        })
        .collect()
}

/// Decodes `block` in one piece with `config`: the lines, progress left out.
fn decode_with(config: LinConfig, block: &LogicBlock) -> Vec<String> {
    let mut harness = harness(config);
    harness.run_all(block);
    describe(&harness.without_progress())
}

fn decode(block: &LogicBlock) -> Vec<String> {
    decode_with(LinConfig::default(), block)
}

/// A waveform on the line, idle for `idle` samples.
fn waveform(idle: u64) -> UartWaveform {
    let mut waveform = UartWaveform::new(line(), CHANNELS, BIT, Tick::ZERO);
    waveform.idle(idle);
    waveform
}

/// Sends a break field of `break_samples` samples, its delimiter, `sync` and `pid`; returns
/// where the break field began.
fn send_header_after(waveform: &mut UartWaveform, break_samples: u64, sync: u8, pid: u8) -> u64 {
    let start = waveform.position().0;
    synth::break_field(waveform, break_samples, DELIMITER);
    waveform.bytes([sync, pid]);
    start
}

/// Sends a header with the protected identifier `pid`; returns where its break field began.
fn send_header(waveform: &mut UartWaveform, pid: u8) -> u64 {
    send_header_after(waveform, BREAK, 0x55, pid)
}

/// The line of a header sent from `start` with the protected identifier `pid`.
fn header_line(frame: u64, start: u64, pid: u8) -> String {
    let parity = if pid == synth::header(pid & 0x3f)[1] {
        ""
    } else {
        " parity-error"
    };
    format!(
        "header f{frame} {pid:02x} id {:02x}{parity} [{start},{})@{}",
        pid & 0x3f,
        start + HEADER,
        start + HEADER - CHAR + STOP
    )
}

/// The ticks of response bytes `from..to` of a frame whose break field began at `start`, sent
/// right after its header.
fn bytes_at(start: u64, from: u64, to: u64) -> String {
    format!(
        "[{},{})",
        start + HEADER + from * CHAR,
        start + HEADER + to * CHAR
    )
}

/// Where response byte `n` of a frame whose break field began at `start` settles.
fn settled(start: u64, n: u64) -> u64 {
    start + HEADER + n * CHAR + STOP
}

/// Where the break field that began at `start` is recognized: at its end.
fn recognized(start: u64) -> u64 {
    start + BREAK
}

/// The end of a frame by the break field that began at `next`.
fn ended_by_break(frame: u64, reason: &str, next: u64) -> String {
    format!("end f{frame} {reason} [{next},{next})@{}", recognized(next))
}

#[test]
fn the_reference_checksum_matches_the_specification_example() {
    // LIN 2.2A §2.8.3: four bytes summing to 0x19, the checksum 0xE6.
    assert_eq!(classic(&[0x4a, 0x55, 0x93, 0xe5]), 0xe6);
    assert_eq!(enhanced(0x4a, &[0x55, 0x93, 0xe5]), 0xe6);
}

#[test]
fn a_frame_with_a_classic_checksum() {
    let data = [0x4a, 0x55, 0x93, 0xe5];
    let mut waveform = waveform(200);
    let first = send_header(&mut waveform, PID_10);
    waveform.bytes(with_checksum(&data, classic(&data)));
    waveform.idle(500);
    let second = send_header(&mut waveform, PID_10);
    waveform.idle(200);
    let block = waveform.finish();
    let end = recognized(second);
    let eof = block.end().0;
    assert_eq!(
        decode(&block),
        vec![
            header_line(1, first, PID_10),
            format!("data f1 +0 4a5593e5 {}@{end}", bytes_at(first, 0, 4)),
            format!("checksum f1 e6 classic {}@{end}", bytes_at(first, 4, 5)),
            ended_by_break(1, "Complete", second),
            header_line(2, second, PID_10),
            // No break follows the second header: the samples end before the frame does.
            format!("end f2 Aborted [{eof},{eof})@{eof}"),
            format!("eof {eof}"),
        ]
    );
}

#[test]
fn a_frame_with_an_enhanced_checksum() {
    // Bytes whose sums carry.
    let data = [0x01, 0x80, 0xff];
    let checksum = enhanced(PID_2A, &data);
    assert_ne!(checksum, classic(&data));
    let mut waveform = waveform(200);
    let first = send_header(&mut waveform, PID_2A);
    waveform.bytes(with_checksum(&data, checksum));
    waveform.idle(300);
    let second = send_header(&mut waveform, PID_2A);
    let end = recognized(second);
    assert_eq!(
        decode(&waveform.finish())[..4],
        [
            header_line(1, first, PID_2A),
            format!("data f1 +0 0180ff {}@{end}", bytes_at(first, 0, 3)),
            format!(
                "checksum f1 {checksum:02x} enhanced {}@{end}",
                bytes_at(first, 3, 4)
            ),
            ended_by_break(1, "Complete", second),
        ]
    );
}

#[test]
fn a_checksum_that_fits_neither_model_is_an_error() {
    let data = [0x10, 0x20];
    let wrong = classic(&data) ^ 0x01;
    assert_ne!(wrong, enhanced(PID_10, &data));
    let mut waveform = waveform(200);
    let first = send_header(&mut waveform, PID_10);
    waveform.bytes(with_checksum(&data, wrong));
    waveform.idle(300);
    let second = send_header(&mut waveform, PID_10);
    let end = recognized(second);
    assert_eq!(
        decode(&waveform.finish())[..5],
        [
            header_line(1, first, PID_10),
            format!("data f1 +0 1020 {}@{end}", bytes_at(first, 0, 2)),
            format!(
                "checksum f1 {wrong:02x} neither {}@{end}",
                bytes_at(first, 2, 3)
            ),
            format!("error f1 Checksum {}@{end}", bytes_at(first, 2, 3)),
            ended_by_break(1, "Error", second),
        ]
    );
}

#[test]
fn protected_identifiers_are_checked() {
    let mut waveform = waveform(200);
    let mut sent = Vec::new();
    for (id, pid) in PIDS {
        sent.push((id, pid, true));
        send_header(&mut waveform, pid);
        waveform.idle(200);
        // Either parity bit wrong.
        for wrong in [pid ^ 0x40, pid ^ 0x80] {
            sent.push((id, wrong, false));
            send_header(&mut waveform, wrong);
            waveform.idle(200);
        }
    }
    send_header(&mut waveform, PID_10);
    let mut harness = harness(LinConfig::default());
    harness.run_all(&waveform.finish());
    let events = harness.events();
    let headers: Vec<(u8, u8, bool)> = events
        .iter()
        .filter_map(|event| match event.payload {
            LinEvent::Header {
                pid, id, pid_ok, ..
            } => Some((id, pid, pid_ok)),
            _ => None,
        })
        .collect();
    assert_eq!(headers[..sent.len()], sent[..]);
    // A wrong parity is an error of the frame: it ends in error, without a response too.
    for (n, (_, _, ok)) in (1..).zip(&sent) {
        let frame = FrameId(n);
        let of_frame: Vec<&LinEvent> = events
            .iter()
            .map(|event| &event.payload)
            .filter(|event| event.frame() == frame)
            .collect();
        let expected = if *ok {
            vec![EndReason::NoResponse]
        } else {
            vec![EndReason::Error]
        };
        let ends: Vec<EndReason> = of_frame
            .iter()
            .filter_map(|event| match event {
                LinEvent::End { reason, .. } => Some(*reason),
                _ => None,
            })
            .collect();
        assert_eq!(ends, expected, "frame {n}");
        let parity_errors = of_frame
            .iter()
            .filter(|event| {
                matches!(
                    event,
                    LinEvent::Error {
                        reason: ErrorReason::Parity,
                        ..
                    }
                )
            })
            .count();
        assert_eq!(parity_errors, usize::from(!ok), "frame {n}");
    }
}

#[test]
fn a_wrong_parity_is_reported_with_the_header() {
    // Identifier 0x10 without its parity bits.
    let pid = 0x10;
    let data = [0x11];
    let mut waveform = waveform(200);
    let first = send_header(&mut waveform, pid);
    waveform.bytes(with_checksum(&data, classic(&data)));
    waveform.idle(300);
    let second = send_header(&mut waveform, PID_10);
    let end = recognized(second);
    let pid_at = first + HEADER - CHAR;
    assert_eq!(
        decode(&waveform.finish())[..5],
        [
            header_line(1, first, pid),
            format!(
                "error f1 Parity [{pid_at},{})@{}",
                pid_at + CHAR,
                pid_at + STOP
            ),
            format!("data f1 +0 11 {}@{end}", bytes_at(first, 0, 1)),
            format!(
                "checksum f1 {:02x} classic {}@{end}",
                classic(&data),
                bytes_at(first, 1, 2)
            ),
            ended_by_break(1, "Error", second),
        ]
    );
}

#[test]
fn a_header_without_a_response_ends_at_the_next_break() {
    let mut waveform = waveform(200);
    let first = send_header(&mut waveform, PID_2A);
    waveform.idle(700);
    let second = send_header(&mut waveform, PID_10);
    waveform.idle(100);
    let block = waveform.finish();
    let eof = block.end().0;
    assert_eq!(
        decode(&block),
        vec![
            header_line(1, first, PID_2A),
            ended_by_break(1, "NoResponse", second),
            header_line(2, second, PID_10),
            format!("end f2 Aborted [{eof},{eof})@{eof}"),
            format!("eof {eof}"),
        ]
    );
}

#[test]
fn a_master_request_frame_carries_eight_bytes_and_the_classic_checksum() {
    // A diagnostic request to every node: NAD 0x7F, PCI, SID, then its data.
    let data = [0x7f, 0x06, 0xb2, 0x00, 0xff, 0x7f, 0xff, 0xff];
    let checksum = classic(&data);
    let mut waveform = waveform(200);
    let first = send_header(&mut waveform, PID_3C);
    waveform.bytes(with_checksum(&data, checksum));
    waveform.idle(400);
    let second = send_header(&mut waveform, PID_3C);
    // The same request with the enhanced checksum, which diagnostic frames never carry.
    let wrong = enhanced(PID_3C, &data);
    assert_ne!(wrong, checksum);
    waveform.bytes(with_checksum(&data, wrong));
    waveform.idle(400);
    let third = send_header(&mut waveform, PID_10);
    let block = waveform.finish();
    let lines = decode(&block);
    let end = recognized(second);
    assert_eq!(
        lines[..4],
        [
            header_line(1, first, PID_3C),
            // Eight data bytes are all a frame carries: they go out once the ninth byte comes.
            format!(
                "data f1 +0 7f06b200ff7fffff {}@{}",
                bytes_at(first, 0, 8),
                settled(first, 8)
            ),
            format!(
                "checksum f1 {checksum:02x} classic {}@{end}",
                bytes_at(first, 8, 9)
            ),
            ended_by_break(1, "Complete", second),
        ]
    );
    let end = recognized(third);
    assert_eq!(
        lines[6..9],
        [
            format!(
                "checksum f2 {wrong:02x} enhanced {}@{end}",
                bytes_at(second, 8, 9)
            ),
            format!("error f2 Checksum {}@{end}", bytes_at(second, 8, 9)),
            ended_by_break(2, "Error", third),
        ]
    );

    // The data shows up while the frame is still under way, before the next break.
    let cut = Tick(first + HEADER + 9 * CHAR + 50);
    let head = block
        .slice(TickRange::new(Tick::ZERO, cut))
        .expect("a slice");
    let mut harness = harness(LinConfig::default());
    harness.feed(&head, &[97]);
    harness.collect();
    assert_eq!(describe(&harness.without_progress()), lines[..2]);
}

#[test]
fn a_tenth_response_byte_is_too_long() {
    let bytes: Vec<u8> = (1..=12).collect();
    let mut waveform = waveform(200);
    let first = send_header(&mut waveform, PID_10);
    waveform.bytes(bytes.iter().copied());
    waveform.idle(300);
    let second = send_header(&mut waveform, PID_2A);
    let data = [0x21];
    waveform.bytes(with_checksum(&data, enhanced(PID_2A, &data)));
    waveform.idle(300);
    let third = send_header(&mut waveform, PID_10);
    let lines = decode(&waveform.finish());
    let too_long_end = first + HEADER + 10 * CHAR;
    assert_eq!(
        lines[..4],
        [
            header_line(1, first, PID_10),
            format!(
                "data f1 +0 0102030405060708 {}@{}",
                bytes_at(first, 0, 8),
                settled(first, 8)
            ),
            // The ninth and tenth bytes, neither data nor a checksum.
            format!(
                "error f1 TooLong {}@{}",
                bytes_at(first, 8, 10),
                settled(first, 9)
            ),
            format!("end f1 Error [{too_long_end},{too_long_end})@{too_long_end}"),
        ]
    );
    // The bytes after the tenth are skipped; the next frame is whole.
    assert_eq!(lines[4], header_line(2, second, PID_2A));
    assert_eq!(lines[7], ended_by_break(2, "Complete", third));
}

#[test]
fn a_single_response_byte_is_too_short() {
    let mut waveform = waveform(200);
    let first = send_header(&mut waveform, PID_10);
    waveform.byte(0x42);
    waveform.idle(300);
    let second = send_header(&mut waveform, PID_10);
    let end = recognized(second);
    assert_eq!(
        decode(&waveform.finish())[..4],
        [
            header_line(1, first, PID_10),
            format!("data f1 +0 42 {}@{end}", bytes_at(first, 0, 1)),
            format!("error f1 TooShort {}@{end}", bytes_at(first, 0, 1)),
            ended_by_break(1, "Error", second),
        ]
    );
}

#[test]
fn a_bad_sync_byte_ends_the_frame() {
    let data = [0x01, 0x02];
    let response = with_checksum(&data, enhanced(PID_10, &data));
    let mut waveform = waveform(200);
    // A wrong value.
    let first = send_header_after(&mut waveform, BREAK, 0x54, PID_10);
    waveform.bytes(response.iter().copied());
    waveform.idle(300);
    // 0x55 with its stop bit at dominant.
    let second = waveform.position().0;
    synth::break_field(&mut waveform, BREAK, DELIMITER);
    waveform.frame(0x55, Faults::FRAMING);
    waveform.idle(BIT);
    waveform.byte(u16::from(PID_10));
    waveform.bytes(response.iter().copied());
    waveform.idle(300);
    let third = send_header(&mut waveform, PID_10);
    waveform.bytes(response.iter().copied());
    waveform.idle(300);
    let fourth = send_header(&mut waveform, PID_10);
    let sync_at = |start: u64| start + BREAK + DELIMITER;
    let lines = decode(&waveform.finish());
    assert_eq!(
        lines[..4],
        [
            format!(
                "error f1 Sync [{},{})@{}",
                sync_at(first),
                sync_at(first) + CHAR,
                sync_at(first) + STOP
            ),
            format!("end f1 Error [{e},{e})@{e}", e = sync_at(first) + CHAR),
            format!(
                "error f2 Sync [{},{})@{}",
                sync_at(second),
                sync_at(second) + CHAR,
                sync_at(second) + STOP
            ),
            format!("end f2 Error [{e},{e})@{e}", e = sync_at(second) + CHAR),
        ]
    );
    assert_eq!(lines[4], header_line(3, third, PID_10));
    assert_eq!(lines[7], ended_by_break(3, "Complete", fourth));
}

#[test]
fn a_framing_error_marks_the_frame() {
    let data = [0x12, 0x34];
    let checksum = classic(&data);
    let mut waveform = waveform(200);
    let first = send_header(&mut waveform, PID_10);
    // The first data byte's stop bit at dominant, then a bit time of idle.
    waveform.frame(0x12, Faults::FRAMING);
    waveform.idle(BIT);
    waveform.bytes([0x34, checksum]);
    waveform.idle(300);
    let second = send_header(&mut waveform, PID_10);
    let end = recognized(second);
    let response = first + HEADER;
    assert_eq!(
        decode(&waveform.finish())[..6],
        [
            header_line(1, first, PID_10),
            format!(
                "error f1 Framing [{response},{})@{}",
                response + CHAR,
                response + STOP
            ),
            format!(
                "data f1 +0 1234 [{response},{})@{end}",
                response + 2 * CHAR + BIT
            ),
            format!(
                "checksum f1 {checksum:02x} classic [{},{})@{end}",
                response + 2 * CHAR + BIT,
                response + 3 * CHAR + BIT
            ),
            ended_by_break(1, "Error", second),
            header_line(2, second, PID_10),
        ]
    );
}

#[test]
fn a_short_break_begins_no_frame() {
    let data = [0x55, 0xaa];
    let response = with_checksum(&data, enhanced(PID_10, &data));
    let mut waveform = waveform(200);
    // Twelve bit times at dominant: not a break field.
    send_header_after(&mut waveform, 12 * BIT, 0x55, PID_10);
    waveform.bytes(response.iter().copied());
    waveform.idle(300);
    // A frame that a break one tick short of 13 bit times ends; what follows it is skipped.
    let first = send_header(&mut waveform, PID_10);
    waveform.bytes(response.iter().copied());
    waveform.idle(300);
    let short = send_header_after(&mut waveform, BREAK - 1, 0x55, PID_10);
    waveform.bytes(response.iter().copied());
    waveform.idle(300);
    // Exactly 13 bit times.
    let second = send_header(&mut waveform, PID_10);
    waveform.bytes(response.iter().copied());
    waveform.idle(300);
    let third = send_header(&mut waveform, PID_10);
    let block = waveform.finish();
    let lines = decode(&block);
    let end = short + BREAK - 1;
    assert_eq!(
        lines[..8],
        [
            header_line(1, first, PID_10),
            format!("data f1 +0 55aa {}@{end}", bytes_at(first, 0, 2)),
            format!(
                "checksum f1 {:02x} enhanced {}@{end}",
                response[2],
                bytes_at(first, 2, 3)
            ),
            format!("end f1 Complete [{short},{short})@{end}"),
            header_line(2, second, PID_10),
            format!(
                "data f2 +0 55aa {}@{}",
                bytes_at(second, 0, 2),
                recognized(third)
            ),
            format!(
                "checksum f2 {:02x} enhanced {}@{}",
                response[2],
                bytes_at(second, 2, 3),
                recognized(third)
            ),
            ended_by_break(2, "Complete", third),
        ]
    );
    // A slave's threshold of 11 bit times takes both short breaks.
    let slave = LinConfig {
        min_break_bits: 11,
        ..LinConfig::default()
    };
    let headers = decode_with(slave, &block)
        .iter()
        .filter(|line| line.starts_with("header"))
        .count();
    assert_eq!(headers, 5);
}

#[test]
fn the_end_of_the_samples_aborts_the_frame() {
    let data = [0x01, 0x02, 0x03];
    let mut waveform = waveform(200);
    let first = send_header(&mut waveform, PID_10);
    waveform.bytes(with_checksum(&data, classic(&data)));
    waveform.idle(300);
    let block = waveform.finish();
    let cut = |end: u64| {
        let block = block
            .slice(TickRange::new(Tick::ZERO, Tick(end)))
            .expect("a slice");
        decode(&block)
    };
    // In the middle of the checksum: the bytes so far are data, and no checksum is given.
    let end = first + HEADER + 3 * CHAR + 50;
    assert_eq!(
        cut(end),
        vec![
            header_line(1, first, PID_10),
            format!("data f1 +0 010203 {}@{end}", bytes_at(first, 0, 3)),
            format!("end f1 Aborted [{end},{end})@{end}"),
            format!("eof {end}"),
        ]
    );
    // After the checksum, no break has said the response is over.
    let end = first + HEADER + 4 * CHAR + 100;
    assert_eq!(
        cut(end)[1],
        format!(
            "data f1 +0 010203{:02x} {}@{end}",
            classic(&data),
            bytes_at(first, 0, 4)
        )
    );
    // Before the protected identifier: the frame reports nothing.
    let end = first + HEADER - 50;
    assert_eq!(cut(end), vec![format!("eof {end}")]);
}

#[test]
fn a_gap_aborts_the_frame_and_the_decoder_waits_for_a_break() {
    let data = [0x10, 0x20, 0x30, 0x40];
    let response = with_checksum(&data, enhanced(PID_10, &data));
    let mut waveform = waveform(200);
    let first = send_header(&mut waveform, PID_10);
    waveform.bytes(response.iter().copied());
    waveform.idle(300);
    let second = send_header(&mut waveform, PID_10);
    waveform.bytes(response.iter().copied());
    waveform.idle(300);
    let third = send_header(&mut waveform, PID_10);
    let block = waveform.finish();
    // Inside the third byte of the first response, and between the second frame's break field
    // and its sync byte.
    let inside = first + HEADER + 2 * CHAR;
    let gaps = [
        TickRange::new(Tick(inside + 30), Tick(inside + 60)),
        TickRange::new(Tick(second + BREAK + 2), Tick(second + BREAK + 8)),
    ];
    let mut harness = harness(LinConfig::default());
    harness.feed_with_gaps(&block, &gaps, &[u64::MAX]);
    harness.end();
    let gap = inside + 30;
    assert_eq!(
        describe(&harness.without_progress())[..6],
        [
            header_line(1, first, PID_10),
            format!("data f1 +0 1020 {}@{gap}", bytes_at(first, 0, 2)),
            format!("end f1 Aborted [{gap},{gap})@{gap}"),
            format!("gap {}", gaps[0]),
            // The second frame lost the samples after its break field: it reports nothing.
            format!("gap {}", gaps[1]),
            header_line(2, third, PID_10),
        ]
    );
}

#[test]
fn length_by_id_ends_frames_with_their_checksum() {
    let config = LinConfig {
        length_by_id: true,
        ..LinConfig::default()
    };
    let mut waveform = waveform(200);
    // Identifier 0x10: two data bytes. Two more bytes before the next break are skipped.
    let data = [0xa1, 0xa2];
    let checksum = enhanced(PID_10, &data);
    let first = send_header(&mut waveform, PID_10);
    waveform.bytes(with_checksum(&data, checksum));
    waveform.bytes([0xee, 0xef]);
    waveform.idle(300);
    // Identifier 0x30: eight data bytes, but only three come.
    let second = send_header(&mut waveform, 0xf0);
    waveform.bytes([0x01, 0x02, 0x03]);
    waveform.idle(300);
    // Identifier 0x20: four data bytes and the checksum, then no more.
    let data_4 = [0x0a, 0x0b, 0x0c, 0x0d];
    let third = send_header(&mut waveform, 0x20);
    waveform.bytes(with_checksum(&data_4, classic(&data_4)));
    waveform.idle(300);
    let fourth = send_header(&mut waveform, PID_10);
    let lines = decode_with(config, &waveform.finish());
    let end = first + HEADER + 3 * CHAR;
    let short = recognized(third);
    assert_eq!(
        lines[..10],
        [
            header_line(1, first, PID_10),
            // The identifier says both are data: they go out with the second.
            format!(
                "data f1 +0 a1a2 {}@{}",
                bytes_at(first, 0, 2),
                settled(first, 1)
            ),
            format!(
                "checksum f1 {checksum:02x} enhanced {}@{}",
                bytes_at(first, 2, 3),
                settled(first, 2)
            ),
            format!("end f1 Complete [{end},{end})@{end}"),
            header_line(2, second, 0xf0),
            format!("data f2 +0 010203 {}@{short}", bytes_at(second, 0, 3)),
            format!("error f2 TooShort {}@{short}", bytes_at(second, 0, 3)),
            ended_by_break(2, "Error", third),
            header_line(3, third, 0x20),
            format!(
                "data f3 +0 0a0b0c0d {}@{}",
                bytes_at(third, 0, 4),
                settled(third, 3)
            ),
        ]
    );
    let end = third + HEADER + 5 * CHAR;
    assert_eq!(
        lines[10..12],
        [
            format!(
                "checksum f3 {:02x} classic {}@{}",
                classic(&data_4),
                bytes_at(third, 4, 5),
                settled(third, 4)
            ),
            format!("end f3 Complete [{end},{end})@{end}"),
        ]
    );
    assert_eq!(lines[12], header_line(4, fourth, PID_10));
}

#[test]
fn the_uart_stream_must_carry_lin_byte_fields() {
    let refused = |uart: UartConfig| {
        build(uart, LinConfig::default(), GraphConfig::default()).expect_err("refused")
    };
    assert_eq!(
        refused(UartConfig {
            data_bits: 7,
            ..line()
        }),
        GraphError::Requirement {
            from: "uart#0.events".into(),
            to: "lin#1.uart".into(),
            key: "dataBits",
            requirement: "= 8".into(),
            found: "7".into(),
        }
    );
    assert_eq!(
        refused(UartConfig {
            parity: Parity::Even,
            ..line()
        }),
        GraphError::Requirement {
            from: "uart#0.events".into(),
            to: "lin#1.uart".into(),
            key: "parity",
            requirement: r#"= "none""#.into(),
            found: r#""even""#.into(),
        }
    );
    assert!(matches!(
        refused(UartConfig {
            stop_bits: StopBits::Two,
            ..line()
        }),
        GraphError::Requirement {
            key: "stopBits",
            ..
        }
    ));
    assert!(matches!(
        refused(UartConfig {
            bit_order: BitOrder::MsbFirst,
            ..line()
        }),
        GraphError::Requirement {
            key: "bitOrder",
            ..
        }
    ));
}

/// Frames of every kind, one after the other; returns where each break field began.
fn busy_bus(waveform: &mut UartWaveform) -> Vec<u64> {
    let mut starts = Vec::new();
    let data: Vec<u8> = (0..8u8).map(|i| i.wrapping_mul(53) ^ 0x5a).collect();
    // A frame with an enhanced checksum, then one with a classic checksum.
    for (n, pid) in [(3usize, PID_10), (8, PID_3C)] {
        starts.push(send_header(waveform, pid));
        let checksum = if pid == PID_3C {
            classic(&data[..n])
        } else {
            enhanced(pid, &data[..n])
        };
        waveform.bytes(with_checksum(&data[..n], checksum));
        waveform.idle(150);
    }
    // No response.
    starts.push(send_header(waveform, PID_2A));
    waveform.idle(400);
    // A wrong sync byte.
    starts.push(send_header_after(waveform, BREAK, 0x15, PID_10));
    waveform.bytes([0x01, 0x02]);
    waveform.idle(200);
    // A wrong parity.
    starts.push(send_header(waveform, 0x10));
    waveform.bytes([0x01, classic(&[0x01])]);
    waveform.idle(200);
    // Too long.
    starts.push(send_header(waveform, PID_10));
    waveform.bytes(0..11);
    waveform.idle(200);
    // Too short.
    starts.push(send_header(waveform, PID_2A));
    waveform.byte(0x99);
    waveform.idle(200);
    // A framing error.
    starts.push(send_header(waveform, PID_2A));
    waveform.frame(0x0f, Faults::FRAMING);
    waveform.idle(BIT);
    waveform.bytes([0x0f, 0x00]);
    waveform.idle(200);
    // A short break, and what follows it.
    starts.push(send_header_after(waveform, 11 * BIT, 0x55, PID_10));
    waveform.bytes([0x01, 0x02, 0x03]);
    waveform.idle(200);
    // A long break, with a long delimiter, and a frame whose bytes come with spaces.
    starts.push(waveform.position().0);
    synth::break_field(waveform, 20 * BIT, 3 * BIT);
    waveform.bytes([0x55, PID_2A]);
    for byte in with_checksum(&data[..4], enhanced(PID_2A, &data[..4])) {
        waveform.idle(BIT / 2 + 7);
        waveform.byte(u16::from(byte));
    }
    waveform.idle(200);
    starts.push(send_header(waveform, PID_10));
    waveform.bytes(with_checksum(&data[..2], enhanced(PID_10, &data[..2])));
    waveform.idle(100);
    starts
}

/// The ends and errors of every frame of a run.
fn outcomes(items: &[Item<LinEvent>]) -> (Vec<EndReason>, Vec<ErrorReason>) {
    let mut ends = Vec::new();
    let mut errors = Vec::new();
    for event in items.iter().filter_map(Item::event) {
        match event.payload {
            LinEvent::End { reason, .. } => ends.push(reason),
            LinEvent::Error { reason, .. } => errors.push(reason),
            _ => {}
        }
    }
    (ends, errors)
}

#[test]
fn every_cut_and_pause_yields_the_same_frames() {
    let mut waveform = waveform(300);
    let starts = busy_bus(&mut waveform);
    let block = waveform.finish();
    // Between the first frame's break field and its sync byte, inside the second frame's
    // response, and on the idle line before the fifth frame.
    let gaps = [
        TickRange::new(Tick(starts[0] + BREAK + 1), Tick(starts[0] + BREAK + 5)),
        TickRange::new(
            Tick(starts[1] + HEADER + 3 * CHAR + 20),
            Tick(starts[1] + HEADER + 3 * CHAR + 45),
        ),
        TickRange::new(Tick(starts[4] - 120), Tick(starts[4] - 60)),
    ];
    for config in [
        LinConfig::default(),
        LinConfig {
            length_by_id: true,
            min_break_bits: 11,
        },
    ] {
        let reference = {
            let mut harness = harness(config);
            harness.feed_with_gaps(&block, &gaps, &[u64::MAX]);
            harness.end();
            harness.without_progress()
        };
        let (ends, errors) = outcomes(&reference);
        for reason in [
            EndReason::Complete,
            EndReason::NoResponse,
            EndReason::Error,
            EndReason::Aborted,
        ] {
            assert!(ends.contains(&reason), "{reason:?} in {ends:?}");
        }
        let expected_errors: &[ErrorReason] = if config.length_by_id {
            &[
                ErrorReason::Sync,
                ErrorReason::Parity,
                ErrorReason::Framing,
                ErrorReason::TooShort,
            ]
        } else {
            &[
                ErrorReason::Sync,
                ErrorReason::Parity,
                ErrorReason::Framing,
                ErrorReason::TooShort,
                ErrorReason::TooLong,
            ]
        };
        for reason in expected_errors {
            assert!(errors.contains(reason), "{reason:?} in {errors:?}");
        }
        assert_block_invariant(
            |graph_config| build(line(), config, graph_config).expect("builds"),
            &block,
            &gaps,
        );
    }
}

#[test]
fn progress_never_passes_a_byte_not_yet_reported() {
    let mut waveform = waveform(300);
    busy_bus(&mut waveform);
    let block = waveform.finish();
    for config in [
        LinConfig::default(),
        LinConfig {
            length_by_id: true,
            ..LinConfig::default()
        },
    ] {
        for sizes in [&[1u64][..], &[13], &[64, 7]] {
            let mut harness = harness(config);
            harness.feed_cut(&block, &[], sizes, &[1]);
            harness.end();
            let mut seen = Tick::ZERO;
            let mut settled = Tick::ZERO;
            for item in &harness.items {
                match item {
                    Item::Progress(tick) => seen = seen.max(*tick),
                    Item::Event(event) => {
                        assert!(event.span.start >= seen, "{event:?} after {seen}");
                        assert!(event.settled >= settled, "{event:?} after {settled}");
                        settled = event.settled;
                    }
                    Item::Gap(_) | Item::End(_) => {}
                }
            }
        }
    }
}

/// One frame of a random scenario.
#[derive(Clone, Debug)]
struct Scenario {
    break_bits: u64,
    sync: u8,
    id: u8,
    flip_parity: bool,
    data: Vec<u8>,
    checksum: Option<bool>,
    corrupt: bool,
    framing_at: Option<usize>,
    idle: u64,
}

fn scenario() -> impl Strategy<Value = Scenario> {
    (
        prop_oneof![6 => Just(13u64), 1 => 10u64..13, 1 => 14u64..20],
        prop_oneof![9 => Just(0x55u8), 1 => any::<u8>()],
        0u8..64,
        prop::bool::weighted(0.1),
        prop::collection::vec(any::<u8>(), 0..11),
        prop::option::weighted(0.85, any::<bool>()),
        prop::bool::weighted(0.15),
        prop::option::weighted(0.1, 0usize..10),
        0u64..400,
    )
        .prop_map(
            |(break_bits, sync, id, flip_parity, data, checksum, corrupt, framing_at, idle)| {
                Scenario {
                    break_bits,
                    sync,
                    id,
                    flip_parity,
                    data,
                    checksum,
                    corrupt,
                    framing_at,
                    idle,
                }
            },
        )
}

fn busy_waveform(scenarios: &[Scenario]) -> LogicBlock {
    let mut waveform = waveform(57);
    for scenario in scenarios {
        let pid = synth::header(scenario.id)[1] ^ if scenario.flip_parity { 0x80 } else { 0 };
        send_header_after(&mut waveform, scenario.break_bits * BIT, scenario.sync, pid);
        let mut bytes = scenario.data.clone();
        if let Some(enhanced_model) = scenario.checksum {
            let checksum = if enhanced_model {
                enhanced(pid, &scenario.data)
            } else {
                classic(&scenario.data)
            };
            bytes.push(checksum ^ u8::from(scenario.corrupt));
        }
        for (n, byte) in bytes.into_iter().enumerate() {
            if scenario.framing_at == Some(n) {
                waveform.frame(u16::from(byte), Faults::FRAMING);
                waveform.idle(BIT);
            } else {
                waveform.byte(u16::from(byte));
            }
        }
        waveform.idle(scenario.idle + 1);
    }
    waveform.finish()
}

/// Checks the framing of a run: every frame begins with its header or a sync error, ends once,
/// ends in error exactly when an error was reported, and keeps its data within bounds.
fn check_frames(items: &[Item<LinEvent>]) -> Result<(), TestCaseError> {
    let mut open: BTreeMap<FrameId, (bool, bool, usize)> = BTreeMap::new();
    let mut closed = Vec::new();
    let mut settled = Tick::ZERO;
    for event in items.iter().filter_map(Item::event) {
        prop_assert!(
            event.settled >= settled,
            "{event:?} settled before {settled}"
        );
        settled = event.settled;
        let frame = event.payload.frame();
        prop_assert!(!closed.contains(&frame), "{frame:?} after its end");
        match &event.payload {
            LinEvent::Header { .. } => {
                prop_assert!(open.insert(frame, (false, false, 0)).is_none());
            }
            LinEvent::Error {
                reason: ErrorReason::Sync,
                ..
            } => {
                prop_assert!(open.insert(frame, (true, false, 0)).is_none());
            }
            LinEvent::Error { .. } => {
                let state = open.get_mut(&frame).expect("an open frame");
                state.0 = true;
            }
            LinEvent::Data { bytes, offset, .. } => {
                prop_assert!(!bytes.is_empty() && bytes.len() <= DATA_BYTES);
                let state = open.get_mut(&frame).expect("an open frame");
                prop_assert_eq!(usize::from(*offset), state.2);
                state.2 += bytes.len();
            }
            LinEvent::Checksum { .. } => {
                let state = open.get_mut(&frame).expect("an open frame");
                prop_assert!(!state.1, "two checksums");
                state.1 = true;
            }
            LinEvent::End { reason, .. } => {
                let (error, checksum, data) = open.remove(&frame).expect("an open frame");
                closed.push(frame);
                match reason {
                    EndReason::Complete => {
                        prop_assert!(!error && checksum && (1..=DATA_BYTES).contains(&data));
                    }
                    EndReason::NoResponse => prop_assert!(!error && !checksum && data == 0),
                    EndReason::Error => prop_assert!(error),
                    EndReason::Aborted => prop_assert!(!checksum),
                }
            }
        }
    }
    prop_assert!(open.is_empty(), "frames left open: {open:?}");
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 48, ..ProptestConfig::default() })]

    /// Blocks of any size, a reader that pauses and tiny queues and budgets all yield the
    /// frames of one uninterrupted run, and every frame is framed as the contract says.
    #[test]
    fn frames_are_the_same_however_the_samples_come(
        scenarios in prop::collection::vec(scenario(), 1..8),
        length_by_id in any::<bool>(),
        sizes in prop::collection::vec(1u64..500, 1..6),
        pauses in prop::collection::vec(0usize..12, 1..8),
        queue_capacity in 1usize..6,
        budget in 1u32..40,
        cut in prop::option::of(0.05f64..1.0),
        gap in prop::option::of((0.05f64..0.95, 1u64..400)),
    ) {
        let config = LinConfig { length_by_id, ..LinConfig::default() };
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
            let mut harness = harness(config);
            match gaps.first() {
                Some(_) => harness.feed_with_gaps(&block, &gaps, &[u64::MAX]),
                None => harness.push(SignalItem::Data(Arc::new(block.clone()))),
            }
            harness.end();
            harness.without_progress()
        };

        let mut harness = build(line(), config, GraphConfig { queue_capacity, budget })
            .expect("builds");
        harness.drain_on_full = 1;
        harness.feed_cut(&block, &gaps, &sizes, &pauses);
        harness.end();
        prop_assert_eq!(without_progress(&harness.items), reference.clone());
        let stats = harness.pipeline().stats();
        prop_assert!(stats.within_capacity(), "{stats:?}");
        check_frames(&reference)?;
    }
}
