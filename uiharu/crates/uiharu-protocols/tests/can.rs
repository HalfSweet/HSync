// SPDX-License-Identifier: Apache-2.0

//! The CAN decoder on synthesized waveforms.

use std::sync::Arc;

use proptest::prelude::*;
use uiharu_core::testing::{self, Harness};
use uiharu_core::{
    EventSchema, GraphConfig, GraphError, InputSpec, Io, Item, LogicBlock, Node, NodeFuture,
    NodeId, NodeSpec, Outbox, OutputRef, OutputSpec, Props, Requires, SchemaId, SignalItem, Tick,
    TickRange, TimeBase,
};
use uiharu_protocols::can::synth::{BitKind, CanFrame, CanWaveform, EdgeShift, Faults, SentBit};
use uiharu_protocols::can::{
    Can, CanBus, CanConfig, CanEvent, Control, DATA_BATCH, EndReason, Ending, FrameId, Identifier,
    Next,
};

const CHANNELS: u8 = 2;
const RX: u8 = 1;
const RATE: u64 = 20_000_000;
const BITRATE: u32 = 500_000;
const FD_BITRATE: u32 = 2_000_000;
/// Samples per nominal bit at `RATE`.
const NOMINAL: u64 = 40;
/// Samples per data bit at `RATE`.
const DATA: u64 = 10;
/// Where a nominal bit is sampled: 87 % of 40 samples, rounded down.
const SAMPLE: u64 = 34;

fn config() -> CanConfig {
    CanConfig {
        rx: RX,
        bitrate: BITRATE,
        fd_bitrate: Some(FD_BITRATE),
        ..CanConfig::default()
    }
}

fn harness_on(config: CanConfig, rate: u64, graph: GraphConfig) -> Harness<CanEvent> {
    Harness::build(CHANNELS, TimeBase::new(rate), graph, |graph, source| {
        let can = graph.add(Can::new(config).expect("a valid configuration"));
        graph.connect(source.samples, Can::input(can))?;
        Ok(Can::events(can))
    })
    .expect("builds")
}

fn harness(config: CanConfig) -> Harness<CanEvent> {
    harness_on(config, RATE, GraphConfig::default())
}

/// A waveform that begins with the bus idle long enough to integrate.
fn bus_waveform(config: &CanConfig) -> CanWaveform {
    let mut waveform = CanWaveform::new(config, CHANNELS, NOMINAL, DATA, Tick::ZERO);
    waveform.idle(12);
    waveform
}

/// Runs `block` through a decoder for `config` in one piece.
fn decode(config: &CanConfig, block: &LogicBlock) -> Vec<Item<CanEvent>> {
    let mut harness = harness(config.clone());
    harness.run_all(block);
    harness.items
}

fn payload(len: usize, seed: u8) -> Vec<u8> {
    (0..len)
        .map(|i| {
            seed.wrapping_mul(29)
                .wrapping_add((i as u8).wrapping_mul(37))
                ^ 0x5a
        })
        .collect()
}

/// The number of data bytes a DLC stands for, from the table of ISO 11898-1.
fn length_of(dlc: u8, fd: bool) -> u8 {
    const FD: [u8; 16] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 12, 16, 20, 24, 32, 48, 64];
    if fd {
        FD[usize::from(dlc & 15)]
    } else {
        dlc.min(8)
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// An event as a line; with `values`, CRC values and stuff counts too.
fn describe(event: &CanEvent, values: bool) -> String {
    match event {
        CanEvent::Begin { frame, partial } => {
            format!(
                "begin f{}{}",
                frame.0,
                if *partial { " partial" } else { "" }
            )
        }
        CanEvent::Id {
            frame,
            id,
            extended,
        } => format!(
            "id f{} {id:x}{}",
            frame.0,
            if *extended { " ext" } else { "" }
        ),
        CanEvent::Control {
            frame,
            remote,
            fd,
            brs,
            esi,
            dlc,
            length,
        } => {
            let mut line = format!("control f{} dlc {dlc} len {length}", frame.0);
            for (set, name) in [
                (*remote, " remote"),
                (*fd, " fd"),
                (*brs, " brs"),
                (*esi, " esi"),
            ] {
                if set {
                    line.push_str(name);
                }
            }
            line
        }
        CanEvent::Data {
            frame,
            offset,
            bytes,
        } => format!("data f{} +{offset} {}", frame.0, hex(bytes)),
        CanEvent::Crc {
            frame,
            value,
            ok,
            stuff_count,
        } => {
            let verdict = if *ok { "ok" } else { "bad" };
            if values {
                let count = stuff_count.map_or(String::new(), |count| format!(" sc {count}"));
                format!("crc f{} {value:x} {verdict}{count}", frame.0)
            } else {
                format!("crc f{} {verdict}", frame.0)
            }
        }
        CanEvent::Ack { frame, acked } => {
            format!("ack f{} {}", frame.0, if *acked { "yes" } else { "no" })
        }
        CanEvent::End { frame, reason } => format!("end f{} {reason:?}", frame.0),
    }
}

fn describe_item(item: &Item<CanEvent>, values: bool, at: bool) -> String {
    match item {
        Item::Event(event) => {
            let what = describe(&event.payload, values);
            if at {
                format!(
                    "{what} [{},{})@{}",
                    event.span.start, event.span.end, event.settled
                )
            } else {
                what
            }
        }
        Item::Gap(range) => format!("gap {range}"),
        Item::Progress(tick) => format!("progress {tick}"),
        Item::End(tick) => format!("eof {tick}"),
    }
}

/// The items as lines without CRC values or ticks, progress left out.
fn brief(items: &[Item<CanEvent>]) -> Vec<String> {
    testing::without_progress(items)
        .iter()
        .map(|item| describe_item(item, false, false))
        .collect()
}

/// The items as lines with CRC values and the ticks each covers and settled at.
fn exact(items: &[Item<CanEvent>]) -> Vec<String> {
    testing::without_progress(items)
        .iter()
        .map(|item| describe_item(item, true, true))
        .collect()
}

/// What a frame sent without faults decodes to, numbered `n`.
fn expected(frame: &CanFrame, n: u64) -> Vec<String> {
    let mut lines = vec![
        format!("begin f{n}"),
        format!(
            "id f{n} {:x}{}",
            frame.id,
            if frame.extended { " ext" } else { "" }
        ),
    ];
    let remote = frame.remote && !frame.fd;
    let mut control = format!(
        "control f{n} dlc {} len {}",
        frame.dlc,
        length_of(frame.dlc, frame.fd)
    );
    for (set, name) in [
        (remote, " remote"),
        (frame.fd, " fd"),
        (frame.fd && frame.brs, " brs"),
        (frame.fd && frame.esi, " esi"),
    ] {
        if set {
            control.push_str(name);
        }
    }
    lines.push(control);
    if !remote {
        for (index, chunk) in frame.data.chunks(DATA_BATCH).enumerate() {
            lines.push(format!("data f{n} +{} {}", index * DATA_BATCH, hex(chunk)));
        }
    }
    lines.push(format!("crc f{n} ok"));
    lines.push(format!("ack f{n} yes"));
    lines.push(format!("end f{n} Eof"));
    lines
}

/// Sends `frames` back to back, three idle bits apart, and checks that each decodes to what
/// was sent.
fn assert_frames_decode(config: &CanConfig, frames: &[CanFrame]) -> Vec<Vec<SentBit>> {
    let mut waveform = bus_waveform(config);
    let mut sent = Vec::new();
    for frame in frames {
        sent.push(waveform.frame(frame));
        waveform.idle(3);
    }
    waveform.idle(8);
    let end = waveform.position();
    let items = decode(config, &waveform.finish());
    let mut lines = Vec::new();
    for (n, frame) in frames.iter().enumerate() {
        lines.extend(expected(frame, n as u64 + 1));
    }
    lines.push(format!("eof {end}"));
    assert_eq!(brief(&items), lines);
    sent
}

/// The index of the first sent bit of `kind`.
fn first(sent: &[SentBit], kind: BitKind) -> usize {
    sent.iter()
        .position(|bit| bit.kind == kind)
        .expect("the frame has such a bit")
}

/// The index of the last sent bit of `kind`.
fn last(sent: &[SentBit], kind: BitKind) -> usize {
    sent.iter()
        .rposition(|bit| bit.kind == kind)
        .expect("the frame has such a bit")
}

/// The events among `items`, in order.
fn events(items: &[Item<CanEvent>]) -> Vec<&uiharu_core::Event<CanEvent>> {
    items.iter().filter_map(Item::event).collect()
}

#[test]
fn classic_base_frames_with_every_dlc() {
    let mut frames: Vec<CanFrame> = (0..=8u8)
        .map(|dlc| {
            CanFrame::base(
                0x100 + u32::from(dlc) * 0x51,
                &payload(usize::from(dlc), dlc),
            )
        })
        .collect();
    // Codes 9 to 15 stand for eight bytes in classic frames.
    frames.push(CanFrame::base(0x7ff, &payload(8, 9)).with_dlc(12));
    frames.push(CanFrame::base(0x000, &[0x00; 8]).with_dlc(15));
    assert_frames_decode(&config(), &frames);
}

#[test]
fn classic_extended_frames_with_every_dlc() {
    let frames: Vec<CanFrame> = (0..=8u8)
        .map(|dlc| {
            CanFrame::extended(
                (0x123_4567 + u32::from(dlc) * 0x10_1010) & 0x1fff_ffff,
                &payload(usize::from(dlc), dlc.wrapping_add(40)),
            )
        })
        .chain([
            CanFrame::extended(0x1fff_ffff, &[0xff; 8]),
            CanFrame::extended(0, &[0; 8]),
        ])
        .collect();
    assert_frames_decode(&config(), &frames);
}

#[test]
fn remote_frames_have_no_data_field() {
    let frames = [
        CanFrame::remote(0x321, false, 4),
        CanFrame::remote(0x1fe_dcba, true, 8),
        CanFrame::remote(0x000, false, 0),
        CanFrame::remote(0x7ff, false, 15),
    ];
    assert_frames_decode(&config(), &frames);
}

#[test]
fn a_crc_15_checked_by_hand() {
    // ID 0x123 in the base format, DLC 2, data AB CD: SOF, the identifier, RTR, IDE, r0, the
    // DLC and the data, without stuff bits, as they go into the CRC.
    let bits = "0 00100100011 0 0 0 0010 10101011 11001101";
    let mut crc = 0u16;
    for bit in bits.chars().filter(|c| !c.is_whitespace()) {
        let feedback = (bit == '1') != (crc >> 14 & 1 == 1);
        crc = (crc << 1) & 0x7fff;
        if feedback {
            crc ^= 0x4599;
        }
    }
    // Dividing the stream, with fifteen zeros appended, by x^15 + x^14 + x^10 + x^8 + x^7 +
    // x^4 + x^3 + 1 leaves the same remainder.
    assert_eq!(crc, 0x7f3c);

    let config = config();
    let mut waveform = bus_waveform(&config);
    let sent = waveform.frame(&CanFrame::base(0x123, &[0xab, 0xcd]));
    waveform.idle(8);
    let sent_crc = sent
        .iter()
        .filter(|bit| bit.kind == BitKind::Crc)
        .fold(0u16, |value, bit| value << 1 | u16::from(bit.recessive));
    assert_eq!(sent_crc, crc, "the synthesizer sends the CRC");
    let items = decode(&config, &waveform.finish());
    let crc_line = exact(&items)
        .into_iter()
        .find(|line| line.starts_with("crc "))
        .expect("a CRC");
    assert!(crc_line.starts_with("crc f1 7f3c ok "), "{crc_line}");
}

#[test]
fn a_classic_frame_in_detail() {
    let config = config();
    let mut waveform = bus_waveform(&config);
    waveform.frame(&CanFrame::base(0x123, &[0xab, 0xcd]));
    waveform.idle(14);
    let end = waveform.position();
    let items = decode(&config, &waveform.finish());
    // Bit `n` of the frame, stuff bits counted, starts at 480 + 40 n and is sampled 34 ticks
    // in. The frame is SOF (0); the identifier (1–11); RTR, IDE, r0 (12–14); DLC 3 and 2
    // (15, 16), a stuff bit after five dominant bits (17), DLC 1 and 0 (18, 19); the data
    // (20–35); the CRC, 0x7f3c, with a stuff bit after its fourth bit (36–51); the CRC
    // delimiter (52); the ACK slot (53); the ACK delimiter (54); end of frame (55–61).
    let bit = |n: u64| 480 + n * NOMINAL;
    let sample = |n: u64| bit(n) + SAMPLE;
    assert_eq!(
        exact(&items),
        vec![
            format!("begin f1 [{},{})@{}", bit(0), bit(1), sample(0)),
            // A base identifier is settled by IDE.
            format!("id f1 123 [{},{})@{}", bit(1), bit(12), sample(13)),
            format!(
                "control f1 dlc 2 len 2 [{},{})@{}",
                bit(12),
                bit(20),
                sample(19)
            ),
            format!("data f1 +0 abcd [{},{})@{}", bit(20), bit(36), sample(35)),
            format!("crc f1 7f3c ok [{},{})@{}", bit(36), bit(52), sample(51)),
            format!("ack f1 yes [{},{})@{}", bit(53), bit(54), sample(53)),
            format!("end f1 Eof [{},{})@{}", bit(55), bit(62), sample(61)),
            format!("eof {end}"),
        ]
    );
}

#[test]
fn fd_frames_switch_bit_rates_and_pick_their_crc() {
    let mut frames = vec![
        // CRC-17, at the data bit rate, from an error-passive transmitter.
        CanFrame::fd(0x456, &payload(12, 1)).with_brs().with_esi(),
        // CRC-21, extended format.
        CanFrame::fd(0x1ab_cdef, &payload(32, 2))
            .in_extended_format()
            .with_brs(),
        // Without bit rate switching, on both sides of the CRC boundary.
        CanFrame::fd(0x055, &payload(16, 3)),
        CanFrame::fd(0x2aa, &payload(20, 4)),
        // Two batches.
        CanFrame::fd(0x07f, &payload(64, 5)).with_brs(),
        CanFrame::fd(0x1fff_ffff, &payload(48, 6))
            .in_extended_format()
            .with_brs()
            .with_esi(),
    ];
    frames.extend((0..=8).map(|len| CanFrame::fd(0x300 + len as u32, &payload(len, 7)).with_brs()));
    let config = config();
    let sent = assert_frames_decode(&config, &frames);

    // The CRC is as wide as the data field calls for, and the stuff count is the number of
    // dynamic stuff bits the synthesizer sent, modulo 8.
    let mut waveform = bus_waveform(&config);
    for frame in &frames {
        waveform.frame(frame);
        waveform.idle(3);
    }
    let items = decode(&config, &waveform.finish());
    let crcs: Vec<(u32, Option<u8>)> = events(&items)
        .iter()
        .filter_map(|event| match event.payload {
            CanEvent::Crc {
                value, stuff_count, ..
            } => Some((value, stuff_count)),
            _ => None,
        })
        .collect();
    assert_eq!(crcs.len(), frames.len());
    for ((frame, sent), (value, stuff_count)) in frames.iter().zip(&sent).zip(crcs) {
        let width = if frame.data.len() > 16 { 21 } else { 17 };
        assert!(
            value < 1 << width,
            "{value:x} for {} bytes",
            frame.data.len()
        );
        let stuff_bits = sent.iter().filter(|bit| bit.kind == BitKind::Stuff).count();
        assert_eq!(stuff_count, Some((stuff_bits % 8) as u8));
    }
}

#[test]
fn stuff_bits_cross_field_boundaries() {
    let config = config();
    let mut frames = Vec::new();
    let mut probe = CanWaveform::new(&config, CHANNELS, NOMINAL, DATA, Tick::ZERO);

    // Five recessive identifier bits end the identifier: the stuff bit sits between it and
    // RTR, and five dominant bits from RTR through DLC 2 put one in the DLC.
    let frame = CanFrame::base(0x7e0, &[0x0f]);
    let sent = probe.frame(&frame);
    let id_end = last(&sent, BitKind::Id);
    assert_eq!(sent[id_end + 1].kind, BitKind::Stuff);
    assert_eq!(sent[id_end + 2].kind, BitKind::Rtr);
    frames.push(frame);

    // FD data ending in five equal bits: the fixed stuff bit that opens the CRC field stands
    // in for the dynamic stuff bit that would follow them.
    let frame = CanFrame::fd(0x123, &[0, 0, 0, 0, 0, 0, 0, 0x1f]).with_brs();
    let sent = probe.frame(&frame);
    let data_end = last(&sent, BitKind::Data);
    assert!(
        sent[data_end - 4..=data_end]
            .iter()
            .all(|bit| bit.recessive)
    );
    assert_eq!(sent[data_end + 1].kind, BitKind::FixedStuff);
    frames.push(frame);

    // A CRC sequence ending in five equal bits is followed by a stuff bit before the CRC
    // delimiter in classic frames.
    let frame = (0..=255u8)
        .map(|byte| CanFrame::base(0x123, &[byte]))
        .find(|frame| {
            let sent = probe.frame(frame);
            sent[first(&sent, BitKind::CrcDelimiter) - 1].kind == BitKind::Stuff
        })
        .expect("some byte ends the CRC in five equal bits");
    frames.push(frame);

    // Stuff bits around the switch to the data bit rate.
    frames.push(CanFrame::fd(0x000, &[0x00, 0xff]).with_brs());
    frames.push(
        CanFrame::fd(0x1fff_ffff, &[0xff; 3])
            .in_extended_format()
            .with_brs()
            .with_esi(),
    );
    assert_frames_decode(&config, &frames);
}

#[test]
fn a_crc_mismatch_is_reported() {
    let config = config();
    let mut waveform = bus_waveform(&config);
    let classic = CanFrame::base(0x123, &[0xab, 0xcd]);
    waveform.frame_with(
        &classic,
        &Faults {
            crc_xor: 1,
            ..Faults::default()
        },
    );
    waveform.idle(3);
    let fd = CanFrame::fd(0x123, &payload(24, 3)).with_brs();
    let sent = waveform.frame_with(
        &fd,
        &Faults {
            crc_xor: 0x10,
            ..Faults::default()
        },
    );
    let stuff_bits = sent.iter().filter(|bit| bit.kind == BitKind::Stuff).count() as u8;
    waveform.idle(3);
    waveform.frame_with(
        &fd,
        &Faults {
            stuff_count_xor: 1,
            ..Faults::default()
        },
    );
    waveform.idle(3);
    // As on a real bus: the receivers that found the CRC wrong send an error flag after the
    // ACK delimiter.
    let sent = CanWaveform::new(&config, CHANNELS, NOMINAL, DATA, Tick::ZERO).frame(&classic);
    waveform.frame_with(
        &classic,
        &Faults {
            crc_xor: 1,
            error_flag_at: Some(first(&sent, BitKind::Eof)),
            ..Faults::default()
        },
    );
    waveform.idle(3);
    let items = decode(&config, &waveform.finish());
    let lines: Vec<String> = testing::without_progress(&items)
        .iter()
        .map(|item| describe_item(item, true, false))
        .filter(|line| line.starts_with("crc ") || line.starts_with("end "))
        .collect();
    assert_eq!(lines[0], "crc f1 7f3d bad");
    assert_eq!(lines[1], "end f1 Eof");
    let count = stuff_bits % 8;
    assert!(
        lines[2].starts_with("crc f2 ") && lines[2].ends_with(&format!(" bad sc {count}")),
        "{lines:?}"
    );
    assert_eq!(lines[3], "end f2 Eof");
    // A stuff count one off: the count and the CRC over it are sent as they are, and the
    // check fails all the same.
    assert!(
        lines[4].starts_with("crc f3 ") && lines[4].ends_with(&format!(" bad sc {}", count ^ 1)),
        "{lines:?}"
    );
    assert_eq!(lines[5], "end f3 Eof");
    assert_eq!(lines[6], "crc f4 7f3d bad");
    assert_eq!(lines[7], "end f4 ErrorFrame");
    assert_eq!(lines.len(), 8, "{lines:?}");
}

#[test]
fn a_missing_ack_is_reported() {
    let config = config();
    let classic = CanFrame::base(0x123, &[0xab, 0xcd]);
    let fd = CanFrame::fd(0x456, &payload(12, 1)).with_brs();
    let probe = |frame: &CanFrame| {
        CanWaveform::new(&config, CHANNELS, NOMINAL, DATA, Tick::ZERO).frame(frame)
    };
    let (classic_ack, fd_ack) = (
        first(&probe(&classic), BitKind::AckDelimiter),
        first(&probe(&fd), BitKind::AckDelimiter),
    );
    let mut waveform = bus_waveform(&config);
    let no_ack = Faults {
        no_ack: true,
        ..Faults::default()
    };
    waveform.frame_with(&classic, &no_ack);
    waveform.idle(3);
    waveform.frame_with(&fd, &no_ack);
    waveform.idle(3);
    // FD receivers accept an acknowledgement one bit late, after a CRC delimiter two bits
    // long, and one two bits long; classic ones do neither.
    waveform.frame_with(
        &fd,
        &Faults {
            no_ack: true,
            flip_bit: Some(fd_ack),
            ..Faults::default()
        },
    );
    waveform.idle(4);
    waveform.frame_with(
        &fd,
        &Faults {
            flip_bit: Some(fd_ack),
            ..Faults::default()
        },
    );
    waveform.idle(4);
    let sent = waveform.frame_with(
        &classic,
        &Faults {
            flip_bit: Some(classic_ack),
            ..Faults::default()
        },
    );
    waveform.idle(10);
    let items = decode(&config, &waveform.finish());
    let lines: Vec<String> = brief(&items)
        .into_iter()
        .filter(|line| line.starts_with("ack ") || line.starts_with("end "))
        .collect();
    assert_eq!(
        lines,
        [
            "ack f1 no",
            "end f1 Eof",
            "ack f2 no",
            "end f2 Eof",
            "ack f3 yes",
            "end f3 Eof",
            "ack f4 yes",
            "end f4 Eof",
            "ack f5 yes",
            "end f5 FormError",
        ]
    );
    // The form error is the ACK delimiter, decided where the line was recessive again.
    let delimiter = sent[classic_ack];
    let eof = sent[classic_ack + 1];
    let end = exact(&items)
        .into_iter()
        .find(|line| line.starts_with("end f5 "))
        .expect("an end");
    assert_eq!(
        end,
        format!(
            "end f5 FormError [{},{})@{}",
            delimiter.start,
            delimiter.end,
            eof.start.0 + SAMPLE
        )
    );
}

#[test]
fn an_error_flag_ends_the_frame() {
    let config = config();
    let classic = CanFrame::base(0x123, &[0xab, 0xcd]);
    let fd = CanFrame::fd(0x456, &payload(12, 1)).with_brs();
    let probe = |frame: &CanFrame| {
        CanWaveform::new(&config, CHANNELS, NOMINAL, DATA, Tick::ZERO).frame(frame)
    };
    let in_data = first(&probe(&classic), BitKind::Data) + 3;
    let in_fd_data = first(&probe(&fd), BitKind::Data) + 20;
    let mut waveform = bus_waveform(&config);
    let sent = waveform.frame_with(
        &classic,
        &Faults {
            error_flag_at: Some(in_data),
            ..Faults::default()
        },
    );
    waveform.idle(3);
    waveform.frame_with(
        &fd,
        &Faults {
            error_flag_at: Some(in_fd_data),
            ..Faults::default()
        },
    );
    waveform.idle(3);
    waveform.frame(&CanFrame::base(0x42, &[1]));
    waveform.idle(5);
    let items = decode(&config, &waveform.finish());
    let lines = brief(&items);
    // The data byte the flag cut into was read with the flag's bits in it; the end says the
    // frame did not hold.
    assert_eq!(
        lines[..5],
        [
            "begin f1",
            "id f1 123",
            "control f1 dlc 2 len 2",
            "data f1 +0 a0",
            "end f1 ErrorFrame",
        ]
    );
    // The error frame covers the six dominant bits, and is decided at the sixth.
    let flag = sent[in_data];
    let end = exact(&items)
        .into_iter()
        .find(|line| line.starts_with("end f1 "))
        .expect("an end");
    assert_eq!(
        end,
        format!(
            "end f1 ErrorFrame [{},{})@{}",
            flag.start,
            flag.start.0 + 6 * NOMINAL,
            flag.start.0 + 5 * NOMINAL + SAMPLE
        )
    );
    // In the data phase the flag is found at the data bit rate; the decoder returns to the
    // nominal one to wait for the error delimiter.
    assert!(lines.contains(&"end f2 ErrorFrame".to_owned()), "{lines:?}");
    assert!(
        !lines.iter().any(|line| line.starts_with("crc f2")),
        "{lines:?}"
    );
    let tail: Vec<String> = lines
        .iter()
        .skip_while(|line| !line.starts_with("begin f3"))
        .cloned()
        .collect();
    let mut want = expected(&CanFrame::base(0x42, &[1]), 3);
    want.push(lines.last().cloned().expect("the end"));
    assert_eq!(tail, want);
}

#[test]
fn a_dominant_delimiter_or_end_of_frame_is_a_form_error() {
    let config = config();
    let frame = CanFrame::base(0x123, &[0xab, 0xcd]);
    let sent = CanWaveform::new(&config, CHANNELS, NOMINAL, DATA, Tick::ZERO).frame(&frame);
    let delimiter = first(&sent, BitKind::CrcDelimiter);
    let eof3 = first(&sent, BitKind::Eof) + 2;
    for (flip, want) in [
        // The CRC ends dominant and the ACK slot is dominant: four dominant bits, then the
        // recessive ACK delimiter decides it.
        (delimiter, "end f1 FormError [2560,2600)@2674"),
        (eof3, "end f1 FormError [2760,2800)@2834"),
    ] {
        let mut waveform = bus_waveform(&config);
        waveform.frame_with(
            &frame,
            &Faults {
                flip_bit: Some(flip),
                ..Faults::default()
            },
        );
        waveform.idle(12);
        let lines = exact(&decode(&config, &waveform.finish()));
        let end = lines
            .iter()
            .find(|line| line.starts_with("end "))
            .expect("an end");
        assert_eq!(end, want, "{lines:?}");
        assert!(lines.iter().any(|line| line.starts_with("crc f1 7f3c ok")));
        assert_eq!(
            lines.iter().any(|line| line.starts_with("ack ")),
            flip == eof3,
            "{lines:?}"
        );
    }
}

#[test]
fn a_form_error_still_undecided_when_the_samples_end_keeps_its_reason() {
    let config = config();
    let frame = CanFrame::base(0x123, &[0xab, 0xcd]);
    let sent = CanWaveform::new(&config, CHANNELS, NOMINAL, DATA, Tick::ZERO).frame(&frame);
    let delimiter = first(&sent, BitKind::CrcDelimiter);
    let mut waveform = bus_waveform(&config);
    let sent = waveform.frame_with(
        &frame,
        &Faults {
            flip_bit: Some(delimiter),
            ..Faults::default()
        },
    );
    let block = waveform.finish();
    // The samples end in the dominant ACK slot, before the line turns recessive again.
    let cut = sent[delimiter + 1].start.offset(20);
    let head = block
        .slice(TickRange::new(Tick::ZERO, cut))
        .expect("a slice");
    let lines = exact(&decode(&config, &head));
    let n = lines.len();
    assert_eq!(
        lines[n - 2],
        format!(
            "end f1 FormError [{},{})@{cut}",
            sent[delimiter].start, sent[delimiter].end
        )
    );
    assert_eq!(lines[n - 1], format!("eof {cut}"));
}

#[test]
fn stuffing_violations_end_the_frame() {
    let config = config();
    // Six recessive bits: the sixth identifier bit of 0x7ff, where a stuff bit was due.
    let mut waveform = bus_waveform(&config);
    waveform.frame_with(
        &CanFrame::base(0x7ff, &[0xff]),
        &Faults {
            drop_stuff_bit: Some(0),
            ..Faults::default()
        },
    );
    waveform.idle(3);
    // Six dominant bits: SOF and five identifier bits of 0x000 make an error flag.
    waveform.frame_with(
        &CanFrame::base(0x000, &[0x00]),
        &Faults {
            drop_stuff_bit: Some(0),
            ..Faults::default()
        },
    );
    waveform.idle(3);
    // A fixed stuff bit of an FD CRC field equal to the bit before it.
    let fd = CanFrame::fd(0x456, &payload(12, 1)).with_brs();
    let sent = CanWaveform::new(&config, CHANNELS, NOMINAL, DATA, Tick::ZERO).frame(&fd);
    let second_fixed = sent
        .iter()
        .enumerate()
        .filter(|(_, bit)| bit.kind == BitKind::FixedStuff)
        .nth(1)
        .map(|(index, _)| index)
        .expect("fixed stuff bits");
    waveform.frame_with(
        &fd,
        &Faults {
            flip_bit: Some(second_fixed),
            ..Faults::default()
        },
    );
    waveform.idle(12);
    let first_frame = 480;
    let items = decode(&config, &waveform.finish());
    let lines = exact(&items);
    let ends: Vec<&String> = lines
        .iter()
        .filter(|line| line.starts_with("end "))
        .collect();
    let bit = |n: u64| first_frame + n * NOMINAL;
    assert_eq!(
        *ends[0],
        format!(
            "end f1 StuffError [{},{})@{}",
            bit(6),
            bit(7),
            bit(6) + SAMPLE
        )
    );
    assert!(ends[1].starts_with("end f2 ErrorFrame ["), "{ends:?}");
    assert!(ends[2].starts_with("end f3 StuffError ["), "{ends:?}");
    assert_eq!(ends.len(), 3, "{lines:?}");
    assert!(
        !lines.iter().any(|line| line.starts_with("crc f3")),
        "{lines:?}"
    );
    // The second frame's six dominant bits are SOF and the five after it.
    let second = events(&items)
        .into_iter()
        .find(|event| {
            matches!(
                event.payload,
                CanEvent::Begin {
                    frame: FrameId(2),
                    ..
                }
            )
        })
        .expect("a second frame")
        .span
        .start;
    assert_eq!(
        *ends[1],
        format!(
            "end f2 ErrorFrame [{second},{})@{}",
            second.0 + 6 * NOMINAL,
            second.0 + 5 * NOMINAL + SAMPLE
        )
    );
}

#[test]
fn the_end_of_the_samples_aborts_a_frame() {
    let config = config();
    let frame = CanFrame::fd(0x7f, &payload(64, 5)).with_brs();
    let mut waveform = bus_waveform(&config);
    let sent = waveform.frame(&frame);
    let block = waveform.finish();
    // Cut in the fourth bit of byte 40.
    let cut = sent
        .iter()
        .filter(|bit| bit.kind == BitKind::Data)
        .nth(40 * 8 + 3)
        .expect("data bits")
        .start;
    let head = block
        .slice(TickRange::new(Tick::ZERO, cut))
        .expect("a slice");
    let items = decode(&config, &head);
    let lines = exact(&items);
    let n = lines.len();
    assert!(
        lines[n - 4].starts_with(&format!("data f1 +0 {} ", hex(&frame.data[..32]))),
        "{lines:?}"
    );
    // The bytes after the full batch go out with the end that cut the frame short; the byte
    // under way is dropped.
    assert!(
        lines[n - 3].starts_with(&format!("data f1 +32 {} [", hex(&frame.data[32..40]))),
        "{lines:?}"
    );
    assert!(lines[n - 3].ends_with(&format!(")@{cut}")), "{lines:?}");
    assert_eq!(lines[n - 2], format!("end f1 Aborted [{cut},{cut})@{cut}"));
    assert_eq!(lines[n - 1], format!("eof {cut}"));
}

#[test]
fn a_gap_aborts_the_frame_and_the_decoder_waits_for_bus_idle() {
    let config = config();
    let frames = [
        CanFrame::base(0x111, &payload(8, 1)),
        CanFrame::base(0x222, &payload(8, 2)),
        CanFrame::base(0x333, &payload(2, 3)),
    ];
    let mut waveform = bus_waveform(&config);
    waveform.frame(&frames[0]);
    waveform.idle(3);
    let sent = waveform.frame(&frames[1]);
    waveform.idle(12);
    waveform.frame(&frames[2]);
    waveform.idle(5);
    let block = waveform.finish();
    // The gap swallows the third data byte and part of the fourth.
    let data: Vec<&SentBit> = sent
        .iter()
        .filter(|bit| bit.kind == BitKind::Data)
        .collect();
    let gap = TickRange::new(Tick(data[16].start.0 + 5), Tick(data[16].start.0 + 405));
    let mut harness = harness(config);
    harness.feed_with_gaps(&block, &[gap], &[97]);
    harness.end();
    let lines = brief(&harness.items);
    let mut want = expected(&frames[0], 1);
    want.extend([
        "begin f2".to_owned(),
        "id f2 222".to_owned(),
        "control f2 dlc 8 len 8".to_owned(),
        format!("data f2 +0 {}", hex(&frames[1].data[..2])),
        "end f2 Aborted".to_owned(),
        format!("gap {gap}"),
        // The rest of the frame, seen before the bus was idle, is not decoded.
        "begin f3 partial".to_owned(),
        "end f3 Aborted".to_owned(),
    ]);
    want.extend(expected(&frames[2], 4));
    want.push(format!("eof {}", block.end()));
    assert_eq!(lines, want);
    let lines = exact(&harness.items);
    assert!(
        lines[10].ends_with(&format!(")@{}", gap.start)),
        "{lines:?}"
    );
    assert_eq!(
        lines[11],
        format!("end f2 Aborted [{},{})@{}", gap.start, gap.start, gap.start)
    );
    assert!(
        lines[13].starts_with(&format!(
            "begin f3 partial [{},{})@",
            gap.end,
            gap.end.next()
        )),
        "{lines:?}"
    );
}

/// Frames that exercise both phases and both formats.
fn mixed_frames() -> Vec<CanFrame> {
    vec![
        CanFrame::base(0x123, &payload(8, 1)),
        CanFrame::extended(0x1abc_def0 & 0x1fff_ffff, &payload(3, 2)),
        CanFrame::fd(0x456, &payload(12, 3)).with_brs().with_esi(),
        CanFrame::fd(0x0f0f_0f0f & 0x1fff_ffff, &payload(32, 4))
            .in_extended_format()
            .with_brs(),
        CanFrame::remote(0x555, false, 3),
    ]
}

#[test]
fn edges_a_tenth_of_a_bit_early_or_late_still_decode() {
    let config = config();
    // Frames at the nominal bit rate throughout: edges moved by a tenth of a nominal bit.
    let nominal = vec![
        CanFrame::base(0x123, &payload(8, 1)),
        CanFrame::extended(0x1abc_def0 & 0x1fff_ffff, &payload(3, 2)),
        CanFrame::fd(0x456, &payload(12, 3)).with_esi(),
        CanFrame::remote(0x555, false, 3),
    ];
    // FD frames that switch to the data bit rate: edges moved by a tenth of a data bit, the
    // arbitration phase's too. An edge moved before the switch carries the shift into the
    // data phase, where a tenth of a nominal bit is four tenths of a data bit: a receiver
    // synchronised to it samples the next bit.
    let switching = vec![
        CanFrame::fd(0x456, &payload(12, 3)).with_brs().with_esi(),
        CanFrame::fd(0x0f0f_0f0f & 0x1fff_ffff, &payload(32, 4))
            .in_extended_format()
            .with_brs(),
    ];
    for (frames, tenth) in [(&nominal, NOMINAL / 10), (&switching, DATA / 10)] {
        // Every combination but late dominant edges with early recessive ones, which together
        // take a fifth off every dominant bit.
        for (to_dominant, to_recessive) in
            [(1, 0), (-1, 0), (0, 1), (0, -1), (1, 1), (-1, -1), (-1, 1)]
        {
            let tenth = tenth as i64;
            let shift = EdgeShift {
                to_dominant: to_dominant * tenth,
                to_recessive: to_recessive * tenth,
            };
            let mut waveform = bus_waveform(&config);
            for frame in frames {
                waveform.frame_with(
                    frame,
                    &Faults {
                        shift,
                        ..Faults::default()
                    },
                );
                waveform.idle(3);
            }
            let end = waveform.position();
            let mut want = Vec::new();
            for (n, frame) in frames.iter().enumerate() {
                want.extend(expected(frame, n as u64 + 1));
            }
            want.push(format!("eof {end}"));
            assert_eq!(
                brief(&decode(&config, &waveform.finish())),
                want,
                "edges moved by {shift:?}"
            );
        }
    }
}

#[test]
fn a_transmitter_one_percent_off_still_decodes() {
    let config = config();
    let frames = mixed_frames();
    // The waveform keeps 40 and 10 samples per bit; at these rates the decoder expects 40.4
    // and 10.1, or 39.6 and 9.9.
    for rate in [20_200_000, 19_800_000] {
        let mut waveform = bus_waveform(&config);
        for frame in &frames {
            waveform.frame(frame);
            waveform.idle(3);
        }
        let end = waveform.position();
        let mut harness = harness_on(config.clone(), rate, GraphConfig::default());
        harness.run_all(&waveform.finish());
        let mut want = Vec::new();
        for (n, frame) in frames.iter().enumerate() {
            want.extend(expected(frame, n as u64 + 1));
        }
        want.push(format!("eof {end}"));
        assert_eq!(brief(&harness.items), want, "{rate} Hz");
    }
}

#[test]
fn a_frame_under_way_at_the_first_sample_is_partial() {
    let config = config();
    let frames = [
        CanFrame::base(0x123, &payload(8, 1)),
        CanFrame::fd(0x456, &payload(12, 2)).with_brs(),
    ];
    let mut waveform = bus_waveform(&config);
    let sent = waveform.frame(&frames[0]);
    waveform.idle(5);
    waveform.frame(&frames[1]);
    waveform.idle(5);
    let block = waveform.finish();
    // Start on a dominant data bit of the first frame.
    let from = sent
        .iter()
        .find(|bit| bit.kind == BitKind::Data && !bit.recessive)
        .expect("a dominant data bit")
        .start;
    let tail = block
        .slice(TickRange::new(from, block.end()))
        .expect("a slice");
    let items = decode(&config, &tail);
    // The bus is idle eleven bits after the dominant ACK slot, the last edge before.
    let ack = sent[first(&sent, BitKind::AckSlot)];
    let idle = ack.start.0 + 11 * NOMINAL + SAMPLE;
    let lines = exact(&items);
    assert_eq!(
        lines[0],
        format!("begin f1 partial [{from},{})@{from}", from.next())
    );
    assert_eq!(lines[1], format!("end f1 Aborted [{idle},{idle})@{idle}"));
    let mut want = vec!["begin f1 partial".to_owned(), "end f1 Aborted".to_owned()];
    want.extend(expected(&frames[1], 2));
    want.push(format!("eof {}", block.end()));
    assert_eq!(brief(&items), want);
}

#[test]
fn with_fd_off_the_fdf_bit_is_r0() {
    let config = CanConfig {
        fd: false,
        ..config()
    };
    let mut waveform = bus_waveform(&config);
    // An FD frame without bit rate switching, read as a classic frame: res, BRS, ESI and DLC 3
    // make its DLC, and from there on nothing fits.
    waveform.frame(&CanFrame::fd(0x456, &[0x11; 12]));
    waveform.idle(10);
    let classic = CanFrame::base(0x42, &[1]);
    waveform.frame(&classic);
    waveform.idle(5);
    let end = waveform.position();
    let lines = brief(&decode(&config, &waveform.finish()));
    let mut want: Vec<String> = [
        "begin f1",
        "id f1 456",
        "control f1 dlc 1 len 1",
        "data f1 +0 22",
        "crc f1 bad",
        "end f1 FormError",
    ]
    .map(str::to_owned)
    .to_vec();
    want.extend(expected(&classic, 2));
    want.push(format!("eof {end}"));
    assert_eq!(lines, want);
}

#[test]
fn frames_the_decoder_cannot_follow_are_unsupported() {
    let config = config();
    let fd = CanFrame::fd(0x456, &payload(12, 1)).with_brs();
    let classic = CanFrame::base(0x42, &[1]);
    // Without a data bit rate, an FD frame that switches is not decoded past BRS.
    let no_data_rate = CanConfig {
        fd_bitrate: None,
        ..config.clone()
    };
    let mut waveform = bus_waveform(&no_data_rate);
    let sent = waveform.frame(&fd);
    waveform.idle(10);
    waveform.frame(&classic);
    waveform.idle(5);
    let end = waveform.position();
    let items = decode(&no_data_rate, &waveform.finish());
    let mut want: Vec<String> = ["begin f1", "id f1 456", "end f1 Unsupported"]
        .map(str::to_owned)
        .to_vec();
    want.extend(expected(&classic, 2));
    want.push(format!("eof {end}"));
    assert_eq!(brief(&items), want);
    let brs = sent[first(&sent, BitKind::Brs)];
    assert_eq!(
        exact(&items)[2],
        format!(
            "end f1 Unsupported [{},{})@{}",
            brs.start,
            brs.start.0 + NOMINAL,
            brs.start.0 + SAMPLE
        )
    );

    // A recessive res bit is a protocol exception.
    let res = first(&sent, BitKind::Res);
    let mut waveform = bus_waveform(&config);
    waveform.frame_with(
        &fd,
        &Faults {
            flip_bit: Some(res),
            ..Faults::default()
        },
    );
    waveform.idle(10);
    waveform.frame(&classic);
    waveform.idle(5);
    let end = waveform.position();
    let mut want: Vec<String> = ["begin f1", "id f1 456", "end f1 Unsupported"]
        .map(str::to_owned)
        .to_vec();
    want.extend(expected(&classic, 2));
    want.push(format!("eof {end}"));
    assert_eq!(brief(&decode(&config, &waveform.finish())), want);
}

#[test]
fn overload_frames_are_frames_of_their_own() {
    let config = config();
    let frame = CanFrame::base(0x123, &[0xab, 0xcd]);
    let sent = CanWaveform::new(&config, CHANNELS, NOMINAL, DATA, Tick::ZERO).frame(&frame);
    let mut waveform = bus_waveform(&config);
    // A dominant last bit of end of frame, which receivers answer with overload flags.
    let first_sent = waveform.frame_with(
        &frame,
        &Faults {
            flip_bit: Some(last(&sent, BitKind::Eof)),
            ..Faults::default()
        },
    );
    waveform.dominant(6);
    waveform.idle(8 + 3);
    // An overload flag in the second bit of the intermission.
    waveform.frame(&frame);
    waveform.idle(1);
    let flag = waveform.position();
    waveform.dominant(6);
    waveform.idle(8 + 3);
    waveform.frame(&frame);
    waveform.idle(5);
    let end = waveform.position();
    let items = decode(&config, &waveform.finish());
    let mut want = expected(&frame, 1);
    want.extend(["begin f2".to_owned(), "end f2 Overload".to_owned()]);
    want.extend(expected(&frame, 3));
    want.extend(["begin f4".to_owned(), "end f4 Overload".to_owned()]);
    want.extend(expected(&frame, 5));
    want.push(format!("eof {end}"));
    assert_eq!(brief(&items), want);
    let lines = exact(&items);
    let eof7 = first_sent[last(&first_sent, BitKind::Eof)];
    let at = format!("[{},{})@{}", eof7.start, eof7.end, eof7.start.0 + SAMPLE);
    assert_eq!(lines[7], format!("begin f2 {at}"));
    assert_eq!(lines[8], format!("end f2 Overload {at}"));
    let at = format!("[{flag},{})@{}", flag.0 + NOMINAL, flag.0 + SAMPLE);
    assert_eq!(lines[16], format!("begin f4 {at}"));
    assert_eq!(lines[17], format!("end f4 Overload {at}"));
}

#[test]
fn a_short_pulse_on_the_idle_bus_is_no_frame() {
    let config = config();
    let mut waveform = bus_waveform(&config);
    // Dominant for less than the 34 ticks to the sample point.
    waveform.dominant_samples(20);
    waveform.idle(4);
    let frame = CanFrame::base(0x0aa, &[0x55]);
    waveform.frame(&frame);
    waveform.idle(5);
    let end = waveform.position();
    let mut want = expected(&frame, 1);
    want.push(format!("eof {end}"));
    assert_eq!(brief(&decode(&config, &waveform.finish())), want);
}

#[test]
fn a_line_dominant_when_high_decodes_alike() {
    let config = CanConfig {
        dominant_low: false,
        ..config()
    };
    assert_frames_decode(&config, &mixed_frames());
}

#[test]
fn data_shows_up_before_the_frame_ends() {
    let config = config();
    let frame = CanFrame::fd(0x7f, &payload(64, 9)).with_brs();
    let mut waveform = bus_waveform(&config);
    let sent = waveform.frame(&frame);
    waveform.idle(5);
    let block = waveform.finish();
    // Up to the middle of byte 50.
    let cut = sent
        .iter()
        .filter(|bit| bit.kind == BitKind::Data)
        .nth(50 * 8 + 4)
        .expect("data bits")
        .start;
    let head = block
        .slice(TickRange::new(Tick::ZERO, cut))
        .expect("a slice");
    let mut harness = harness(config);
    harness.feed(&head, &[333]);
    harness.collect();
    assert_eq!(
        brief(&harness.items),
        [
            "begin f1".to_owned(),
            "id f1 7f".to_owned(),
            "control f1 dlc 15 len 64 fd brs".to_owned(),
            format!("data f1 +0 {}", hex(&frame.data[..32])),
        ]
    );
    let tail = block
        .slice(TickRange::new(cut, block.end()))
        .expect("a slice");
    harness.feed(&tail, &[333]);
    harness.end();
    let mut want = expected(&frame, 1);
    want.push(format!("eof {}", block.end()));
    assert_eq!(brief(&harness.items), want);
}

#[test]
fn the_source_must_be_fast_and_wide_enough() {
    let build = |channels: u8, rate: u64, config: CanConfig| {
        Harness::build(
            channels,
            TimeBase::new(rate),
            GraphConfig::default(),
            |graph, source| {
                let can = graph.add(Can::new(config).expect("a valid configuration"));
                graph.connect(source.samples, Can::input(can))?;
                Ok(Can::events(can))
            },
        )
        .err()
    };
    // Four samples per bit at the data bit rate are the least.
    match build(2, 7_999_999, config()) {
        Some(GraphError::Requirement {
            key,
            requirement,
            found,
            ..
        }) => {
            assert_eq!(key, "rateHz");
            assert_eq!(requirement, ">= 8000000");
            assert_eq!(found, "7999999");
        }
        other => panic!("expected a requirement error, got {other:?}"),
    }
    assert!(build(2, 8_000_000, config()).is_none());
    // Without bit rate switching the nominal bit rate counts.
    let classic = CanConfig {
        fd_bitrate: None,
        ..config()
    };
    assert!(build(2, 2_000_000, classic.clone()).is_none());
    assert!(matches!(
        build(1, RATE, classic),
        Some(GraphError::Requirement {
            key: "channels",
            ..
        })
    ));
}

/// A decoder built on CAN for the test: what it read of each frame through the bus cursor.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Summary {
    frame: FrameId,
    partial: bool,
    identifier: Option<Identifier>,
    control: Option<Control>,
    bytes: Vec<u8>,
    ending: Ending,
}

impl EventSchema for Summary {
    const SCHEMA: SchemaId = SchemaId::new("test.can-summary", 1);
}

/// Reads the identifier, the control field and up to `bytes` data bytes of each frame, and
/// leaves the rest to the end.
struct Summarize {
    bytes: usize,
}

impl Summarize {
    fn input(node: NodeId) -> uiharu_core::InputRef<CanEvent> {
        uiharu_core::InputRef::of_node(node, 0)
    }

    fn output(node: NodeId) -> OutputRef<Summary> {
        OutputRef::of_node(node, 0)
    }
}

impl Node for Summarize {
    fn spec(&self) -> NodeSpec {
        NodeSpec {
            name: "summarize".into(),
            inputs: vec![InputSpec::new::<CanEvent>("can", Requires::new())],
            outputs: vec![OutputSpec::new::<Summary>("summaries", Props::new())],
        }
    }

    fn start(self: Box<Self>, mut io: Io) -> NodeFuture {
        let bus = CanBus::new(io.input(0));
        Box::pin(summarize(bus, io.output(0), self.bytes))
    }
}

async fn summarize(mut bus: CanBus, mut out: Outbox<Summary>, wanted: usize) {
    loop {
        match bus.next().await {
            Next::Frame(mut frame) => {
                let id = frame.id();
                let partial = frame.is_partial();
                let identifier = frame.identifier().await.ok();
                let control = frame.control().await.ok();
                let mut bytes = Vec::new();
                while bytes.len() < wanted {
                    match frame.byte().await {
                        Ok(byte) => bytes.push(byte),
                        Err(_) => break,
                    }
                }
                let ending = frame.end().await.unwrap_or(Ending {
                    reason: EndReason::Aborted,
                    crc: None,
                    acked: None,
                });
                let at = frame.consumed();
                let summary = Summary {
                    frame: id,
                    partial,
                    identifier,
                    control,
                    bytes,
                    ending,
                };
                out.emit(at, summary).await;
            }
            Next::Gap(range) => out.gap(range).await,
            Next::Progress(tick) => out.progress(tick).await,
            Next::End(tick) => {
                out.end(tick).await;
                return;
            }
        }
        // Whatever a frame left unread, `end` drained it.
        let _ = bus.finish().await;
    }
}

fn summaries(config: &CanConfig, block: &LogicBlock, wanted: usize) -> Vec<Item<Summary>> {
    let mut harness = Harness::build(
        CHANNELS,
        TimeBase::new(RATE),
        GraphConfig {
            queue_capacity: 2,
            budget: 3,
        },
        |graph, source| {
            let can = graph.add(Can::new(config.clone()).expect("a valid configuration"));
            graph.connect(source.samples, Can::input(can))?;
            let summarize = graph.add(Summarize { bytes: wanted });
            graph.connect(Can::events(can), Summarize::input(summarize))?;
            Ok(Summarize::output(summarize))
        },
    )
    .expect("builds");
    harness.drain_on_full = 1;
    harness.feed(block, &[57]);
    harness.end();
    testing::without_progress(&harness.items)
}

#[test]
fn decoders_built_on_can_read_frames_through_the_bus_cursor() {
    let config = config();
    let classic = CanFrame::base(0x123, &[0xab, 0xcd, 0xef]);
    let long = CanFrame::fd(0x456, &payload(64, 4)).with_brs();
    let remote = CanFrame::remote(0x321, false, 4);
    let mut waveform = bus_waveform(&config);
    let first_sent = waveform.frame(&classic);
    waveform.idle(3);
    waveform.frame(&long);
    waveform.idle(3);
    waveform.frame(&remote);
    waveform.idle(3);
    let in_data = first(&first_sent, BitKind::Data) + 3;
    waveform.frame_with(
        &classic,
        &Faults {
            error_flag_at: Some(in_data),
            ..Faults::default()
        },
    );
    waveform.idle(5);
    let block = waveform.finish();

    // Reading every byte, across the batches of the long frame.
    let all = summaries(&config, &block, 64);
    let read: Vec<&Summary> = all
        .iter()
        .filter_map(Item::event)
        .map(|e| &e.payload)
        .collect();
    assert_eq!(read.len(), 4);
    assert_eq!(
        read[0].identifier,
        Some(Identifier {
            id: 0x123,
            extended: false
        })
    );
    assert_eq!(read[0].bytes, classic.data);
    assert!(read[0].ending.is_valid());
    assert_eq!(read[0].ending.acked, Some(true));
    assert_eq!(read[1].bytes, long.data);
    assert_eq!(read[1].control.map(|control| control.length), Some(64));
    assert!(
        read[1]
            .ending
            .crc
            .is_some_and(|crc| crc.stuff_count.is_some())
    );
    // A remote frame has no data to read.
    assert!(read[2].control.is_some_and(|control| control.remote));
    assert!(read[2].bytes.is_empty());
    assert!(read[2].ending.is_valid());
    // The frame the error flag cut short: its byte, and an end that is not valid.
    assert_eq!(read[3].bytes, [0xa0]);
    assert_eq!(read[3].ending.reason, EndReason::ErrorFrame);
    assert_eq!(read[3].ending.crc, None);
    assert!(!read[3].ending.is_valid());

    // Reading two bytes and leaving the rest: the end drains it, and each summary covers its
    // frame from the begin to the end.
    let some = summaries(&config, &block, 2);
    let decoded = decode(&config, &block);
    let frames: Vec<(FrameId, TickRange, Tick)> = {
        let events = events(&decoded);
        let mut spans = Vec::new();
        for event in &events {
            if let CanEvent::End { frame, .. } = event.payload {
                let begin = events
                    .iter()
                    .find(|begin| {
                        matches!(begin.payload, CanEvent::Begin { frame: f, .. } if f == frame)
                    })
                    .expect("a begin");
                spans.push((
                    frame,
                    TickRange::new(begin.span.start, event.span.end),
                    event.settled,
                ));
            }
        }
        spans
    };
    let read: Vec<&uiharu_core::Event<Summary>> = some.iter().filter_map(Item::event).collect();
    assert_eq!(read.len(), frames.len());
    for (summary, (frame, span, settled)) in read.iter().zip(&frames) {
        assert_eq!(summary.payload.frame, *frame);
        assert!(summary.payload.bytes.len() <= 2);
        assert_eq!(summary.span, *span);
        assert_eq!(summary.settled, *settled);
    }
    assert_eq!(read[1].payload.bytes, long.data[..2]);
    assert!(read[1].payload.ending.is_valid());
    assert!(matches!(some.last(), Some(Item::End(_))));
}

#[test]
fn a_partial_frame_reads_as_one() {
    let config = config();
    let frame = CanFrame::base(0x123, &payload(8, 1));
    let mut waveform = bus_waveform(&config);
    let sent = waveform.frame(&frame);
    waveform.idle(5);
    waveform.frame(&frame);
    waveform.idle(5);
    let block = waveform.finish();
    let from = sent[first(&sent, BitKind::Data) + 2].start;
    let tail = block
        .slice(TickRange::new(from, block.end()))
        .expect("a slice");
    let read = summaries(&config, &tail, 8);
    let read: Vec<&Summary> = read
        .iter()
        .filter_map(Item::event)
        .map(|e| &e.payload)
        .collect();
    assert_eq!(read.len(), 2);
    assert!(read[0].partial);
    assert_eq!(read[0].identifier, None);
    assert_eq!(read[0].ending.reason, EndReason::Aborted);
    assert!(!read[1].partial);
    assert_eq!(read[1].bytes, frame.data);
}

/// A waveform with a bit of everything, at 8 and 4 samples per bit.
fn rich_waveform(config: &CanConfig) -> (LogicBlock, Vec<TickRange>) {
    let mut waveform = CanWaveform::new(config, CHANNELS, 8, 4, Tick::ZERO);
    waveform.idle(12);
    waveform.frame(&CanFrame::base(0x123, &payload(8, 1)));
    waveform.idle(3);
    waveform.frame(
        &CanFrame::fd(0x1abc_def0 & 0x1fff_ffff, &payload(20, 2))
            .in_extended_format()
            .with_brs()
            .with_esi(),
    );
    // An overload flag in the second bit of the intermission.
    waveform.idle(1);
    waveform.dominant(6);
    waveform.idle(8 + 3);
    waveform.frame(&CanFrame::remote(0x555, false, 3));
    waveform.idle(5);
    let classic = CanFrame::base(0x0f0, &payload(2, 3));
    let sent = CanWaveform::new(config, CHANNELS, 8, 4, Tick::ZERO).frame(&classic);
    waveform.frame_with(
        &classic,
        &Faults {
            crc_xor: 3,
            error_flag_at: Some(first(&sent, BitKind::Eof)),
            ..Faults::default()
        },
    );
    waveform.idle(3);
    let long = waveform.frame(&CanFrame::fd(0x07f, &payload(64, 4)).with_brs());
    waveform.idle(12);
    waveform.frame_with(
        &CanFrame::base(0x7ff, &[0xff, 0x00]),
        &Faults {
            drop_stuff_bit: Some(1),
            ..Faults::default()
        },
    );
    waveform.idle(12);
    waveform.frame_with(
        &CanFrame::extended(0x1555_5555, &payload(5, 5)),
        &Faults {
            shift: EdgeShift {
                to_dominant: -1,
                to_recessive: 1,
            },
            ..Faults::default()
        },
    );
    waveform.idle(4);
    let cut = waveform.frame(&CanFrame::fd(0x222, &payload(16, 6)).with_brs());
    waveform.idle(12);
    waveform.frame(&CanFrame::base(0x333, &payload(4, 7)));
    waveform.idle(5);
    let block = waveform.finish();
    // A gap in the data phase of the 64-byte frame, another across the end of one frame and
    // the start of the next.
    let long_data = long[first(&long, BitKind::Data) + 100].start;
    let across = cut[first(&cut, BitKind::Data) + 40].start;
    let gaps = vec![
        TickRange::new(long_data, long_data.offset(37)),
        TickRange::new(across, across.offset(400)),
    ];
    (block, gaps)
}

fn fast_config() -> CanConfig {
    CanConfig {
        rx: RX,
        bitrate: 1_000_000,
        fd_bitrate: Some(2_000_000),
        ..CanConfig::default()
    }
}

const FAST_RATE: u64 = 8_000_000;

#[test]
fn every_cut_and_pause_yields_the_same_events() {
    let config = fast_config();
    let (block, gaps) = rich_waveform(&config);
    let build = |graph: GraphConfig| harness_on(config.clone(), FAST_RATE, graph);
    // The waveform decodes to every kind of event and end.
    let mut whole = build(GraphConfig::default());
    whole.feed_with_gaps(&block, &gaps, &[u64::MAX]);
    whole.end();
    let lines = brief(&whole.items);
    for needle in [
        " Eof",
        " ErrorFrame",
        " Overload",
        " StuffError",
        " Aborted",
        "remote",
        "partial",
        "esi",
        "gap ",
        " bad",
    ] {
        assert!(
            lines.iter().any(|line| line.contains(needle)),
            "{needle}: {lines:?}"
        );
    }
    testing::assert_block_invariant(build, &block, &gaps);
}

/// Events settle in order, and none settles before its span begins.
fn assert_settled_in_order(items: &[Item<CanEvent>]) {
    let mut last = Tick::ZERO;
    for event in items.iter().filter_map(Item::event) {
        assert!(event.settled >= last, "{event:?} settles before {last}");
        assert!(event.settled >= event.span.start, "{event:?}");
        last = event.settled;
    }
}

#[derive(Clone, Debug)]
struct Sent {
    frame: CanFrame,
    faults: Faults,
    idle: u64,
}

fn arb_frame() -> impl Strategy<Value = CanFrame> {
    (
        0u32..0x2000_0000,
        any::<bool>(),
        0u8..16,
        0u8..4,
        any::<bool>(),
        any::<bool>(),
        any::<u8>(),
    )
        .prop_map(|(id, extended, dlc, kind, brs, esi, seed)| {
            let id = if extended { id } else { id & 0x7ff };
            let frame = match kind {
                0 => CanFrame::base(id, &payload(usize::from(dlc.min(8)), seed)).with_dlc(dlc),
                1 => CanFrame::remote(id, false, dlc),
                _ => CanFrame {
                    brs,
                    esi,
                    ..CanFrame::fd(id, &payload(usize::from(length_of(dlc, true)), seed))
                },
            };
            if extended {
                frame.in_extended_format()
            } else {
                frame
            }
        })
}

fn arb_faults() -> impl Strategy<Value = Faults> {
    prop_oneof![
        8 => Just(Faults::default()),
        1 => (1u32..0x7fff).prop_map(|crc_xor| Faults { crc_xor, ..Faults::default() }),
        1 => Just(Faults { no_ack: true, ..Faults::default() }),
        1 => (0usize..160).prop_map(|at| Faults { error_flag_at: Some(at), ..Faults::default() }),
        1 => (0usize..160).prop_map(|at| Faults { flip_bit: Some(at), ..Faults::default() }),
        1 => (0usize..3).prop_map(|n| Faults { drop_stuff_bit: Some(n), ..Faults::default() }),
        1 => (-2i64..=2, -2i64..=2).prop_map(|(to_dominant, to_recessive)| Faults {
            shift: EdgeShift { to_dominant, to_recessive },
            ..Faults::default()
        }),
    ]
}

fn arb_traffic() -> impl Strategy<Value = Vec<Sent>> {
    prop::collection::vec(
        (arb_frame(), arb_faults(), 3u64..16).prop_map(|(frame, faults, idle)| Sent {
            frame,
            faults,
            idle,
        }),
        1..6,
    )
}

fn traffic_waveform(config: &CanConfig, traffic: &[Sent]) -> LogicBlock {
    let mut waveform = CanWaveform::new(config, CHANNELS, 8, 4, Tick::ZERO);
    waveform.idle(12);
    for sent in traffic {
        waveform.frame_with(&sent.frame, &sent.faults);
        waveform.idle(sent.idle);
    }
    waveform.finish()
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 48, ..ProptestConfig::default() })]

    /// Blocks of any size, a reader that pauses at random and tiny queues and budgets all
    /// yield the events of one uninterrupted run, settled in order, within the queues' bounds.
    #[test]
    fn blocks_pauses_queues_and_budgets_do_not_change_the_events(
        traffic in arb_traffic(),
        sizes in prop::collection::vec(1u64..300, 1..5),
        pauses in prop::collection::vec(0usize..8, 1..6),
        queue_capacity in 1usize..5,
        budget in 1u32..30,
        cut in prop::option::of(0.05f64..1.0),
        gap in prop::option::of((0.05f64..0.95, 1u64..400)),
    ) {
        let config = fast_config();
        let whole = traffic_waveform(&config, &traffic);
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

        let mut reference = harness_on(config.clone(), FAST_RATE, GraphConfig::default());
        reference.feed_with_gaps(&block, &gaps, &[u64::MAX]);
        reference.end();
        let reference = reference.without_progress();
        assert_settled_in_order(&reference);

        let mut harness = harness_on(config, FAST_RATE, GraphConfig { queue_capacity, budget });
        harness.drain_on_full = 1;
        harness.feed_cut(&block, &gaps, &sizes, &pauses);
        harness.end();
        prop_assert_eq!(harness.without_progress(), reference);
        let stats = harness.pipeline().stats();
        prop_assert!(stats.within_capacity(), "{stats:?}");
        for event in harness.events() {
            if let CanEvent::Data { bytes, .. } = &event.payload {
                prop_assert!(bytes.len() <= DATA_BATCH);
            }
        }
    }

    /// Frames sent without faults decode to what was sent: identifiers, formats, codes and
    /// data, a matching CRC, the acknowledgement and the end of frame.
    #[test]
    fn frames_decode_to_what_was_sent(frames in prop::collection::vec(arb_frame(), 1..6)) {
        let config = fast_config();
        let traffic: Vec<Sent> = frames
            .iter()
            .map(|frame| Sent { frame: frame.clone(), faults: Faults::default(), idle: 3 })
            .collect();
        let block = traffic_waveform(&config, &traffic);
        let mut harness = harness_on(config, FAST_RATE, GraphConfig::default());
        harness.push(SignalItem::Data(Arc::new(block.clone())));
        harness.end();
        let mut want = Vec::new();
        for (n, frame) in frames.iter().enumerate() {
            want.extend(expected(frame, n as u64 + 1));
        }
        want.push(format!("eof {}", block.end()));
        prop_assert_eq!(brief(&harness.items), want);
        assert_settled_in_order(&harness.items);
    }
}
