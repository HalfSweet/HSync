// SPDX-License-Identifier: Apache-2.0

//! The USB packet and transaction decoders on synthesized waveforms.

use std::collections::BTreeSet;

use proptest::prelude::*;
use uiharu_core::testing::{Harness, Tap, assert_block_invariant};
use uiharu_core::{GraphConfig, GraphError, InputRef, Item, LogicBlock, Tick, TickRange, TimeBase};
use uiharu_protocols::spi::{SPI_EVENTS, Spi, SpiConfig, SpiEvent};
use uiharu_protocols::usb::synth::{UsbWaveform, crc5, crc16, token_fields};
use uiharu_protocols::usb::{
    DATA_BATCH, Pid, Speed, TransactionEvent, USB_PACKETS, UsbConfig, UsbEvent, UsbPackets,
    UsbTransactions, XactId,
};

const CHANNELS: u8 = 2;
/// Full speed sampled at 48 MHz: four ticks per bit.
const FULL: u64 = 48_000_000;
/// Low speed sampled at 12 MHz: eight ticks per bit.
const LOW: u64 = 12_000_000;

fn config(speed: Speed) -> UsbConfig {
    UsbConfig {
        speed,
        ..UsbConfig::default()
    }
}

/// Source → USB packets.
fn packets(speed: Speed, rate: u64, graph: GraphConfig) -> Harness<UsbEvent> {
    Harness::build(CHANNELS, TimeBase::new(rate), graph, |graph, source| {
        let usb = graph.add(UsbPackets::new(config(speed)).expect("a valid configuration"));
        graph.connect(source.samples, UsbPackets::input(usb))?;
        Ok(UsbPackets::packets(usb))
    })
    .expect("builds")
}

fn decode(speed: Speed, rate: u64, block: &LogicBlock) -> Vec<String> {
    let mut harness = packets(speed, rate, GraphConfig::default());
    harness.run_all(block);
    describe(&harness.without_progress())
}

fn waveform(speed: Speed, rate: u64) -> UsbWaveform {
    let mut waveform = UsbWaveform::new(config(speed), CHANNELS, rate, Tick::ZERO);
    waveform.idle_bits(16);
    waveform
}

fn at(span: TickRange, settled: Tick) -> String {
    if span.is_empty() {
        format!("@{settled}")
    } else {
        format!("[{},{})@{settled}", span.start, span.end)
    }
}

fn bad(ok: bool) -> &'static str {
    if ok { "" } else { " bad" }
}

fn hex(bytes: &[u8]) -> String {
    if bytes.is_empty() {
        return "-".to_owned();
    }
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn what(event: &UsbEvent) -> String {
    match event {
        UsbEvent::Begin { pkt } => format!("begin p{}", pkt.0),
        UsbEvent::Pid { pkt, pid, ok } => format!("pid p{} {pid:?}{}", pkt.0, bad(*ok)),
        UsbEvent::Token {
            pkt,
            address,
            endpoint,
            crc5,
            ok,
        } => format!(
            "token p{} {address}.{endpoint} crc {crc5:02x}{}",
            pkt.0,
            bad(*ok)
        ),
        UsbEvent::Sof {
            pkt,
            frame,
            crc5,
            ok,
        } => format!("sof p{} {frame} crc {crc5:02x}{}", pkt.0, bad(*ok)),
        UsbEvent::Data { pkt, offset, bytes } => {
            format!("data p{} +{offset} {}", pkt.0, hex(bytes))
        }
        UsbEvent::DataEnd {
            pkt,
            crc16,
            ok,
            length,
        } => format!(
            "data-end p{} crc {} len {length}{}",
            pkt.0,
            crc16.map_or("--".to_owned(), |crc| format!("{crc:04x}")),
            bad(*ok)
        ),
        UsbEvent::Split { pkt, raw } => format!("split p{} {raw:06x}", pkt.0),
        UsbEvent::End { pkt, reason } => format!("end p{} {reason:?}", pkt.0),
        UsbEvent::Reset { ticks } => format!("reset {ticks}"),
        UsbEvent::Suspend => "suspend".to_owned(),
        UsbEvent::Resume => "resume".to_owned(),
        UsbEvent::KeepAlive => "keep-alive".to_owned(),
    }
}

fn lines<T>(items: &[Item<T>], what: impl Fn(&T) -> String) -> Vec<String> {
    items
        .iter()
        .map(|item| match item {
            Item::Event(event) => {
                format!("{} {}", what(&event.payload), at(event.span, event.settled))
            }
            Item::Gap(range) => format!("gap {range}"),
            Item::Progress(tick) => format!("progress {tick}"),
            Item::End(tick) => format!("eof {tick}"),
        })
        .collect()
}

/// One line per packet stream item, with where it comes from.
fn describe(items: &[Item<UsbEvent>]) -> Vec<String> {
    lines(items, what)
}

/// The lines without where they come from.
fn bare(lines: &[String]) -> Vec<String> {
    lines
        .iter()
        .map(|line| match line.rfind(" [").or_else(|| line.rfind(" @")) {
            Some(cut) if !line.starts_with("gap") && !line.starts_with("eof") => {
                line[..cut].to_owned()
            }
            _ => line.clone(),
        })
        .collect()
}

/// Checks the envelope: events settle in order and every packet that begins ends once.
fn assert_well_formed(items: &[Item<UsbEvent>]) {
    let mut settled = Tick::ZERO;
    let mut open = BTreeSet::new();
    for item in items {
        let Item::Event(event) = item else { continue };
        assert!(event.settled >= settled, "settled out of order: {event:?}");
        settled = event.settled;
        match &event.payload {
            UsbEvent::Begin { pkt } => assert!(open.insert(*pkt), "{pkt:?} began twice"),
            UsbEvent::End { pkt, .. } => assert!(open.remove(pkt), "{pkt:?} ended unopened"),
            UsbEvent::Data { bytes, .. } => assert!(bytes.len() <= DATA_BATCH),
            other => {
                if let Some(pkt) = other.pkt() {
                    assert!(open.contains(&pkt), "{other:?} outside its packet");
                }
            }
        }
    }
    assert!(open.is_empty(), "packets left open: {open:?}");
}

/// The cell of each bit of `bytes`, sent after a SYNC, counted from the SYNC's first cell and
/// stuffed bits skipped; and the cell that follows the last bit, where the EOP begins.
fn cells(bytes: &[u8]) -> (Vec<u64>, u64) {
    // The SYNC fills eight cells and its last bit is the first 1 of a run.
    let mut cell = 8;
    let mut ones = 1;
    let mut found = Vec::new();
    for byte in bytes {
        for i in 0..8 {
            found.push(cell);
            cell += 1;
            if (byte >> i) & 1 == 0 {
                ones = 0;
                continue;
            }
            ones += 1;
            if ones == 6 {
                cell += 1;
                ones = 0;
            }
        }
    }
    (found, cell)
}

#[test]
fn a_handshake_at_full_speed() {
    let mut waveform = waveform(Speed::Full, FULL);
    waveform.handshake(Pid::Ack);
    waveform.idle_bits(8);
    let block = waveform.finish();
    // 16 idle bits of 4 ticks; the SYNC in cells 0–7 from tick 64, each read at its centre;
    // the PID in cells 8–15; SE0 in cells 16 and 17, J from cell 18.
    assert_eq!(
        decode(Speed::Full, FULL, &block),
        vec![
            "begin p1 [64,95)@94",
            "pid p1 Ack [96,127)@126",
            "end p1 Eop [128,137)@136",
            "eof 172",
        ]
    );
}

#[test]
fn a_handshake_at_low_speed() {
    let mut waveform = waveform(Speed::Low, LOW);
    waveform.handshake(Pid::Nak);
    waveform.idle_bits(8);
    let block = waveform.finish();
    // Eight ticks per bit, D− high when idle.
    assert_eq!(
        decode(Speed::Low, LOW, &block),
        vec![
            "begin p1 [128,189)@188",
            "pid p1 Nak [192,253)@252",
            "end p1 Eop [256,273)@272",
            "eof 344",
        ]
    );
}

#[test]
fn every_pid_decodes() {
    let mut waveform = waveform(Speed::Full, FULL);
    waveform.token(Pid::Out, 0x15, 0xe);
    waveform.idle_bits(3);
    waveform.token(Pid::In, 0x7f, 0xf);
    waveform.idle_bits(3);
    waveform.token(Pid::Setup, 0, 0);
    waveform.idle_bits(3);
    waveform.token(Pid::Ping, 0x3a, 0xa);
    waveform.idle_bits(3);
    waveform.sof(0x7ff);
    waveform.idle_bits(3);
    for (pid, payload) in [
        (Pid::Data0, &[][..]),
        (Pid::Data1, &[0x01][..]),
        (Pid::Data2, &[0xff, 0xff][..]),
        (Pid::Mdata, &[0x00, 0x7e, 0xfe][..]),
    ] {
        waveform.data(pid, payload);
        waveform.idle_bits(3);
    }
    for pid in [Pid::Ack, Pid::Nak, Pid::Stall, Pid::Nyet] {
        waveform.handshake(pid);
        waveform.idle_bits(3);
    }
    waveform.split(0xfedcba);
    waveform.idle_bits(3);
    // A reserved PID, its check nibble in order.
    waveform.sync();
    waveform.byte(0xf0);
    waveform.eop();
    waveform.idle_bits(3);
    // A preamble has no EOP; the bus is idle for the hub's setup time.
    waveform.preamble();
    waveform.idle_bits(12);
    waveform.handshake(Pid::Ack);
    waveform.idle_bits(8);
    let block = waveform.finish();
    let lines = decode(Speed::Full, FULL, &block);
    let crc = |payload: &[u8]| format!("{:04x}", crc16(payload));
    assert_eq!(
        bare(&lines),
        vec![
            "begin p1".to_owned(),
            "pid p1 Out".to_owned(),
            format!(
                "token p1 21.14 crc {:02x}",
                crc5(token_fields(0x15, 0xe), 11)
            ),
            "end p1 Eop".to_owned(),
            "begin p2".to_owned(),
            "pid p2 In".to_owned(),
            format!(
                "token p2 127.15 crc {:02x}",
                crc5(token_fields(0x7f, 0xf), 11)
            ),
            "end p2 Eop".to_owned(),
            "begin p3".to_owned(),
            "pid p3 Setup".to_owned(),
            "token p3 0.0 crc 02".to_owned(),
            "end p3 Eop".to_owned(),
            "begin p4".to_owned(),
            "pid p4 Ping".to_owned(),
            format!(
                "token p4 58.10 crc {:02x}",
                crc5(token_fields(0x3a, 0xa), 11)
            ),
            "end p4 Eop".to_owned(),
            "begin p5".to_owned(),
            "pid p5 Sof".to_owned(),
            format!("sof p5 2047 crc {:02x}", crc5(0x7ff, 11)),
            "end p5 Eop".to_owned(),
            "begin p6".to_owned(),
            "pid p6 Data0".to_owned(),
            "data-end p6 crc 0000 len 0".to_owned(),
            "end p6 Eop".to_owned(),
            "begin p7".to_owned(),
            "pid p7 Data1".to_owned(),
            "data p7 +0 01".to_owned(),
            format!("data-end p7 crc {} len 1", crc(&[0x01])),
            "end p7 Eop".to_owned(),
            "begin p8".to_owned(),
            "pid p8 Data2".to_owned(),
            "data p8 +0 ffff".to_owned(),
            format!("data-end p8 crc {} len 2", crc(&[0xff, 0xff])),
            "end p8 Eop".to_owned(),
            "begin p9".to_owned(),
            "pid p9 Mdata".to_owned(),
            "data p9 +0 007efe".to_owned(),
            format!("data-end p9 crc {} len 3", crc(&[0x00, 0x7e, 0xfe])),
            "end p9 Eop".to_owned(),
            "begin p10".to_owned(),
            "pid p10 Ack".to_owned(),
            "end p10 Eop".to_owned(),
            "begin p11".to_owned(),
            "pid p11 Nak".to_owned(),
            "end p11 Eop".to_owned(),
            "begin p12".to_owned(),
            "pid p12 Stall".to_owned(),
            "end p12 Eop".to_owned(),
            "begin p13".to_owned(),
            "pid p13 Nyet".to_owned(),
            "end p13 Eop".to_owned(),
            "begin p14".to_owned(),
            "pid p14 Split".to_owned(),
            "split p14 fedcba".to_owned(),
            "end p14 Eop".to_owned(),
            "begin p15".to_owned(),
            "pid p15 Reserved".to_owned(),
            "end p15 Eop".to_owned(),
            "begin p16".to_owned(),
            "pid p16 Pre".to_owned(),
            "end p16 Preamble".to_owned(),
            "begin p17".to_owned(),
            "pid p17 Ack".to_owned(),
            "end p17 Eop".to_owned(),
            format!("eof {}", block.end()),
        ]
    );
}

#[test]
fn a_token_and_its_fields_are_placed_on_their_bits() {
    let mut waveform = waveform(Speed::Full, FULL);
    // SETUP to address 0, endpoint 0: the bytes 2D 00 10, with no stuffed bits.
    waveform.token(Pid::Setup, 0, 0);
    waveform.idle_bits(8);
    let block = waveform.finish();
    assert_eq!(
        decode(Speed::Full, FULL, &block),
        vec![
            "begin p1 [64,95)@94",
            "pid p1 Setup [96,127)@126",
            "token p1 0.0 crc 02 [128,191)@190",
            "end p1 Eop [192,201)@200",
            "eof 236",
        ]
    );
}

#[test]
fn crcs_are_checked() {
    let mut waveform = waveform(Speed::Full, FULL);
    let good = crc5(token_fields(5, 1), 11);
    waveform.token_with_crc(Pid::In, 5, 1, good ^ 0b100);
    waveform.idle_bits(3);
    waveform.sof_with_crc(42, crc5(42, 11) ^ 1);
    waveform.idle_bits(3);
    waveform.data_with_crc(Pid::Data0, &[1, 2, 3], crc16(&[1, 2, 3]) ^ 0x0100);
    waveform.idle_bits(3);
    waveform.data(Pid::Data1, &[1, 2, 3]);
    waveform.idle_bits(8);
    let block = waveform.finish();
    let lines = bare(&decode(Speed::Full, FULL, &block));
    assert_eq!(
        lines[2],
        format!("token p1 5.1 crc {:02x} bad", good ^ 0b100)
    );
    assert_eq!(
        lines[6],
        format!("sof p2 42 crc {:02x} bad", crc5(42, 11) ^ 1)
    );
    assert_eq!(lines[10], "data p3 +0 010203");
    assert_eq!(
        lines[11],
        format!(
            "data-end p3 crc {:04x} len 3 bad",
            crc16(&[1, 2, 3]) ^ 0x0100
        )
    );
    assert_eq!(
        lines[16],
        format!("data-end p4 crc {:04x} len 3", crc16(&[1, 2, 3]))
    );
}

#[test]
fn a_failed_pid_check_ends_the_packet() {
    let mut waveform = waveform(Speed::Full, FULL);
    waveform.sync();
    waveform.byte(Pid::In.byte() ^ 0x40);
    waveform.bits(0x1234, 16);
    waveform.eop();
    waveform.idle_bits(3);
    waveform.handshake(Pid::Ack);
    waveform.idle_bits(8);
    let block = waveform.finish();
    let lines = decode(Speed::Full, FULL, &block);
    assert_eq!(
        bare(&lines),
        vec![
            "begin p1".to_owned(),
            "pid p1 In bad".to_owned(),
            "end p1 PidError".to_owned(),
            "begin p2".to_owned(),
            "pid p2 Ack".to_owned(),
            "end p2 Eop".to_owned(),
            format!("eof {}", block.end()),
        ]
    );
    // The error is settled with the PID it was found in.
    assert_eq!(lines[1], "pid p1 In bad [96,127)@126");
    assert_eq!(lines[2], "end p1 PidError [96,127)@126");
}

#[test]
fn a_seventh_one_is_a_bit_stuff_error() {
    let mut waveform = waveform(Speed::Full, FULL);
    waveform.sync();
    waveform.pid(Pid::Data0);
    waveform.byte(0x00);
    // Data0 ends with two 1s, 0x00 with none: six 1s, then a seventh where a 0 belongs.
    let first_one = waveform.position();
    waveform.unstuffed_ones(9);
    waveform.eop();
    waveform.idle_bits(3);
    waveform.handshake(Pid::Ack);
    waveform.idle_bits(8);
    let block = waveform.finish();
    let lines = decode(Speed::Full, FULL, &block);
    let seventh = first_one.offset(6 * 4);
    assert_eq!(
        lines[2],
        format!(
            "end p1 BitStuffError [{seventh},{})@{}",
            seventh.offset(3),
            seventh.offset(2)
        )
    );
    assert_eq!(
        bare(&lines),
        vec![
            "begin p1".to_owned(),
            "pid p1 Data0".to_owned(),
            "end p1 BitStuffError".to_owned(),
            "begin p2".to_owned(),
            "pid p2 Ack".to_owned(),
            "end p2 Eop".to_owned(),
            format!("eof {}", block.end()),
        ]
    );
}

#[test]
fn stuffed_bits_are_dropped() {
    let mut waveform = waveform(Speed::Full, FULL);
    // Runs of 1s across bytes and fields, stuffed by the synthesizer.
    waveform.data(Pid::Data1, &[0xff, 0xff, 0x3f, 0x80, 0xfe, 0x7f]);
    waveform.idle_bits(3);
    waveform.token(Pid::Out, 0x7f, 0xf);
    waveform.idle_bits(8);
    let block = waveform.finish();
    let lines = bare(&decode(Speed::Full, FULL, &block));
    assert_eq!(lines[2], "data p1 +0 ffff3f80fe7f");
    assert!(lines[3].ends_with("len 6"), "{lines:?}");
    assert!(lines[7].starts_with("token p2 127.15 "), "{lines:?}");
    assert!(!lines[7].ends_with("bad"), "{lines:?}");
}

#[test]
fn a_long_data_packet_streams_in_batches() {
    let payload: Vec<u8> = (0..64u32).map(|i| (i * 37 % 256) as u8).collect();
    let mut waveform = waveform(Speed::Full, FULL);
    let sync = waveform.position();
    waveform.data(Pid::Data1, &payload);
    waveform.idle_bits(8);
    let block = waveform.finish();
    let lines = decode(Speed::Full, FULL, &block);
    // Where each bit lies: the PID, the payload, the CRC16, counted in cells of 4 ticks.
    let mut sent = vec![Pid::Data1.byte()];
    sent.extend(&payload);
    sent.extend(crc16(&payload).to_le_bytes());
    let (bits, eop) = cells(&sent);
    let first_bit = |byte: usize| sync.offset(4 * bits[8 * (byte + 1)]);
    let last_centre = |byte: usize| sync.offset(4 * bits[8 * (byte + 1) + 7] + 2);
    let eop = sync.offset(4 * eop + 2);
    // A byte is known to be payload once two more have followed it: the first batch when
    // byte 33 is whole, the second when the CRC's second byte is.
    assert_eq!(
        lines[2],
        format!(
            "data p1 +0 {} [{},{})@{}",
            hex(&payload[..32]),
            first_bit(0),
            last_centre(31).next(),
            last_centre(33)
        )
    );
    assert_eq!(
        lines[3],
        format!(
            "data p1 +32 {} [{},{})@{}",
            hex(&payload[32..]),
            first_bit(32),
            last_centre(63).next(),
            last_centre(65)
        )
    );
    // The CRC is told apart when the EOP begins.
    assert_eq!(
        lines[4],
        format!(
            "data-end p1 crc {:04x} len 64 [{},{})@{eop}",
            crc16(&payload),
            first_bit(64),
            last_centre(65).next()
        )
    );
    assert_eq!(bare(&lines[5..6]), vec!["end p1 Eop"]);
}

#[test]
fn a_short_data_packet_flushes_at_its_end() {
    let payload = [0x11, 0x22, 0x33];
    let mut waveform = waveform(Speed::Full, FULL);
    let sync = waveform.position();
    waveform.data(Pid::Data0, &payload);
    waveform.idle_bits(8);
    let block = waveform.finish();
    let lines = decode(Speed::Full, FULL, &block);
    let mut sent = vec![Pid::Data0.byte()];
    sent.extend(payload);
    sent.extend(crc16(&payload).to_le_bytes());
    let (bits, eop) = cells(&sent);
    let eop = sync.offset(4 * eop + 2);
    assert_eq!(
        lines[2],
        format!(
            "data p1 +0 112233 [{},{})@{eop}",
            sync.offset(4 * bits[8]),
            sync.offset(4 * bits[8 * 3 + 7] + 3)
        )
    );
}

#[test]
fn bits_after_a_packets_fields_are_dropped() {
    let mut waveform = waveform(Speed::Full, FULL);
    waveform.sync();
    waveform.pid(Pid::In);
    waveform.bits(u64::from(token_fields(9, 2)), 11);
    waveform.bits(u64::from(crc5(token_fields(9, 2), 11)), 5);
    // Dribble: three more bits before the EOP.
    waveform.bits(0b101, 3);
    waveform.eop();
    waveform.idle_bits(3);
    waveform.sync();
    waveform.pid(Pid::Data0);
    for byte in [0x12, 0x34] {
        waveform.byte(byte);
    }
    for byte in crc16(&[0x12, 0x34]).to_le_bytes() {
        waveform.byte(byte);
    }
    waveform.bits(0b1, 1);
    waveform.eop();
    waveform.idle_bits(8);
    let block = waveform.finish();
    let lines = bare(&decode(Speed::Full, FULL, &block));
    assert_eq!(
        lines,
        vec![
            "begin p1".to_owned(),
            "pid p1 In".to_owned(),
            format!("token p1 9.2 crc {:02x}", crc5(token_fields(9, 2), 11)),
            "end p1 Eop".to_owned(),
            "begin p2".to_owned(),
            "pid p2 Data0".to_owned(),
            "data p2 +0 1234".to_owned(),
            format!("data-end p2 crc {:04x} len 2", crc16(&[0x12, 0x34])),
            "end p2 Eop".to_owned(),
            format!("eof {}", block.end()),
        ]
    );
}

#[test]
fn packets_follow_each_other_closely() {
    // Two bit times between packets, the least the specification allows, at a sample rate
    // that is no multiple of the bit rate.
    for rate in [FULL, 50_000_000, 60_000_000] {
        let mut waveform = waveform(Speed::Full, rate);
        waveform.token(Pid::Out, 3, 2);
        waveform.idle_bits(2);
        waveform.data(Pid::Data0, &[0xde, 0xad, 0xbe, 0xef]);
        waveform.idle_bits(2);
        waveform.handshake(Pid::Ack);
        waveform.idle_bits(2);
        waveform.sof(0x123);
        waveform.idle_bits(8);
        let block = waveform.finish();
        let mut harness = packets(Speed::Full, rate, GraphConfig::default());
        harness.run_all(&block);
        assert_well_formed(&harness.items);
        let lines = bare(&describe(&harness.without_progress()));
        assert_eq!(
            lines,
            vec![
                "begin p1".to_owned(),
                "pid p1 Out".to_owned(),
                format!("token p1 3.2 crc {:02x}", crc5(token_fields(3, 2), 11)),
                "end p1 Eop".to_owned(),
                "begin p2".to_owned(),
                "pid p2 Data0".to_owned(),
                "data p2 +0 deadbeef".to_owned(),
                format!(
                    "data-end p2 crc {:04x} len 4",
                    crc16(&[0xde, 0xad, 0xbe, 0xef])
                ),
                "end p2 Eop".to_owned(),
                "begin p3".to_owned(),
                "pid p3 Ack".to_owned(),
                "end p3 Eop".to_owned(),
                "begin p4".to_owned(),
                "pid p4 Sof".to_owned(),
                format!("sof p4 291 crc {:02x}", crc5(0x123, 11)),
                "end p4 Eop".to_owned(),
                format!("eof {}", block.end()),
            ],
            "{rate} Hz"
        );
    }
}

#[test]
fn bus_signalling_at_low_speed() {
    let mut waveform = waveform(Speed::Low, LOW);
    let first = waveform.position();
    waveform.keep_alive();
    // A millisecond later, the next keep-alive: no suspend.
    waveform.idle(12_000);
    let second = waveform.position();
    waveform.keep_alive();
    let idle_from = second.offset(16);
    // Four milliseconds idle: suspended after three.
    waveform.idle(48_000);
    let k = waveform.position();
    waveform.resume(252_000);
    let resumed = k.offset(252_000 + 16);
    waveform.idle_bits(16);
    let reset = waveform.position();
    waveform.reset(600);
    waveform.idle_bits(16);
    waveform.handshake(Pid::Ack);
    waveform.idle_bits(8);
    let block = waveform.finish();
    let lines = decode(Speed::Low, LOW, &block);
    // An EOP is two bit times of SE0, 16 ticks; 3 ms is 36 000 ticks.
    assert_eq!(
        lines[..5].to_vec(),
        vec![
            format!(
                "keep-alive [{first},{})@{}",
                first.offset(17),
                first.offset(16)
            ),
            format!(
                "keep-alive [{second},{})@{}",
                second.offset(17),
                second.offset(16)
            ),
            format!(
                "suspend [{idle_from},{})@{}",
                idle_from.offset(36_000),
                idle_from.offset(36_000)
            ),
            format!("resume [{k},{})@{resumed}", resumed.next()),
            format!(
                "reset 600 [{reset},{})@{}",
                reset.offset(600),
                reset.offset(600)
            ),
        ]
    );
    assert_eq!(
        bare(&lines[5..]),
        vec![
            "begin p1".to_owned(),
            "pid p1 Ack".to_owned(),
            "end p1 Eop".to_owned(),
            format!("eof {}", block.end()),
        ]
    );
}

#[test]
fn a_reset_at_full_speed_and_no_keep_alive() {
    let mut waveform = waveform(Speed::Full, FULL);
    // 2.5 µs is 120 ticks: 119 is no reset; nor is an EOP alone at full speed a keep-alive.
    waveform.reset(119);
    waveform.idle_bits(8);
    waveform.keep_alive();
    waveform.idle_bits(8);
    let reset = waveform.position();
    waveform.reset(120);
    waveform.idle_bits(8);
    let block = waveform.finish();
    assert_eq!(
        decode(Speed::Full, FULL, &block),
        vec![
            format!(
                "reset 120 [{reset},{})@{}",
                reset.offset(120),
                reset.offset(120)
            ),
            format!("eof {}", block.end()),
        ]
    );
}

#[test]
fn a_suspend_is_reported_once_the_samples_reach_it() {
    let mut waveform = waveform(Speed::Full, FULL);
    waveform.handshake(Pid::Ack);
    // The EOP's J began a bit time before the end of the handshake.
    let since = waveform.position().0 - 4;
    waveform.idle(150_000);
    let block = waveform.finish();
    // 3 ms at 48 MHz.
    let suspended = since + 144_000;
    let expected = format!("suspend [{since},{suspended})@{suspended}");
    // Whether the samples reaching the deadline end a block or not, the suspend is the same.
    for sizes in [
        &[u64::MAX][..],
        &[suspended],
        &[suspended + 1],
        &[1000],
        &[7, 64],
    ] {
        let mut harness = packets(Speed::Full, FULL, GraphConfig::default());
        harness.feed(&block, sizes);
        harness.collect();
        let lines = describe(&harness.without_progress());
        assert_eq!(lines.get(3), Some(&expected), "blocks of {sizes:?}");
    }
    // Fed up to the deadline and no further, the suspend is already known.
    let head = block
        .slice(TickRange::new(Tick::ZERO, Tick(suspended)))
        .expect("a slice");
    let mut harness = packets(Speed::Full, FULL, GraphConfig::default());
    harness.feed(&head, &[u64::MAX]);
    harness.collect();
    assert_eq!(
        describe(&harness.without_progress()).get(3),
        Some(&expected)
    );
}

#[test]
fn a_packet_cut_off_by_the_end_of_the_samples_is_aborted() {
    let payload: Vec<u8> = (0..40).collect();
    let mut waveform = waveform(Speed::Full, FULL);
    let sync = waveform.position();
    waveform.data(Pid::Data0, &payload);
    let block = waveform.finish();
    let mut sent = vec![Pid::Data0.byte()];
    sent.extend(&payload);
    let (bits, _) = cells(&sent);
    // Cut at the fourth bit of payload byte 20: bytes 0–19 are whole, and 0–17 are known to
    // be payload; 18 and 19 might have been the CRC.
    let cut = sync.offset(4 * bits[8 * 21 + 3]);
    let head = block
        .slice(TickRange::new(Tick::ZERO, cut))
        .expect("a slice");
    let lines = decode(Speed::Full, FULL, &head);
    assert_eq!(
        lines[2..],
        vec![
            format!(
                "data p1 +0 {} [{},{})@{cut}",
                hex(&payload[..18]),
                sync.offset(4 * bits[8]),
                sync.offset(4 * bits[8 * 18 + 7] + 3)
            ),
            format!("end p1 Aborted @{cut}"),
            format!("eof {cut}"),
        ]
    );
}

#[test]
fn a_gap_aborts_the_packet_and_the_decoder_waits_for_the_bus_to_be_idle() {
    let payload: Vec<u8> = (100..120).collect();
    let mut waveform = waveform(Speed::Full, FULL);
    let sync = waveform.position();
    waveform.data(Pid::Data1, &payload);
    waveform.idle_bits(3);
    waveform.handshake(Pid::Ack);
    waveform.idle_bits(8);
    let block = waveform.finish();
    let mut sent = vec![Pid::Data1.byte()];
    sent.extend(&payload);
    let (bits, _) = cells(&sent);
    // Lose 50 ticks from the fourth bit of payload byte 10 on.
    let start = sync.offset(4 * bits[8 * 11 + 3]);
    let gap = TickRange::new(start, start.offset(50));
    let mut harness = packets(Speed::Full, FULL, GraphConfig::default());
    harness.feed_with_gaps(&block, &[gap], &[7]);
    harness.end();
    let lines = describe(&harness.without_progress());
    assert_eq!(
        lines[2],
        format!(
            "data p1 +0 {} [{},{})@{}",
            hex(&payload[..8]),
            sync.offset(4 * bits[8]),
            sync.offset(4 * bits[8 * 8 + 7] + 3),
            gap.start
        )
    );
    assert_eq!(lines[3], format!("end p1 Aborted @{}", gap.start));
    assert_eq!(lines[4], format!("gap {gap}"));
    // The rest of the data packet cannot be aligned; its EOP leaves the bus idle.
    assert_eq!(
        bare(&lines[5..]),
        vec![
            "begin p2".to_owned(),
            "pid p2 Ack".to_owned(),
            "end p2 Eop".to_owned(),
            format!("eof {}", block.end()),
        ]
    );
}

#[test]
fn a_bit_read_on_the_last_sample_before_a_gap_counts_however_the_samples_come() {
    let payload: Vec<u8> = (0..12).map(|i| i * 17).collect();
    let mut waveform = waveform(Speed::Full, FULL);
    let sync = waveform.position();
    waveform.data(Pid::Data0, &payload);
    waveform.idle_bits(3);
    waveform.handshake(Pid::Ack);
    waveform.idle_bits(8);
    let block = waveform.finish();
    let mut sent = vec![Pid::Data0.byte()];
    sent.extend(&payload);
    let (bits, _) = cells(&sent);
    // The gap begins right after the centre of the last bit of payload byte 6, which is
    // whole: bytes 0–4 are known to be payload, whether the gap was already queued when the
    // block before it was used up or came later.
    let start = sync.offset(4 * bits[8 * 7 + 7] + 3);
    let gap = TickRange::new(start, start.offset(40));
    assert_block_invariant(|graph| packets(Speed::Full, FULL, graph), &block, &[gap]);
    let mut harness = packets(Speed::Full, FULL, GraphConfig::default());
    harness.feed_with_gaps(&block, &[gap], &[u64::MAX]);
    harness.end();
    let lines = bare(&describe(&harness.without_progress()));
    assert_eq!(lines[2], format!("data p1 +0 {}", hex(&payload[..5])));
    assert_eq!(lines[3], "end p1 Aborted");
}

#[test]
fn a_packet_under_way_at_the_first_sample_is_skipped() {
    let mut waveform = waveform(Speed::Full, FULL);
    let sync = waveform.position();
    waveform.data(
        Pid::Data0,
        &[0x00, 0xff, 0x0f, 0xf0, 0x3c, 0xc3, 0x55, 0xaa],
    );
    waveform.idle_bits(3);
    waveform.token(Pid::In, 1, 1);
    waveform.idle_bits(8);
    let block = waveform.finish();
    // Start in the middle of the data packet's fourth byte, and in the middle of its SYNC.
    for from in [sync.offset(4 * (16 + 3 * 8 + 4) + 1), sync.offset(9)] {
        let tail = block
            .slice(TickRange::new(from, block.end()))
            .expect("a slice");
        assert_eq!(
            bare(&decode(Speed::Full, FULL, &tail)),
            vec![
                "begin p1".to_owned(),
                "pid p1 In".to_owned(),
                format!("token p1 1.1 crc {:02x}", crc5(token_fields(1, 1), 11)),
                "end p1 Eop".to_owned(),
                format!("eof {}", block.end()),
            ],
            "from {from}"
        );
    }
}

#[test]
fn a_sync_needs_three_alternating_cells() {
    let mut waveform = waveform(Speed::Full, FULL);
    // KJKK: the shortest SYNC accepted, as a repeater may leave it.
    waveform.sync_with(3);
    waveform.pid(Pid::Ack);
    waveform.eop();
    waveform.idle_bits(8);
    // KK and KJJ: no SYNC; their bits are not read.
    waveform.sync_with(1);
    waveform.pid(Pid::Nak);
    waveform.eop();
    waveform.idle_bits(8);
    waveform.sync_with(2);
    waveform.pid(Pid::Nak);
    waveform.eop();
    waveform.idle_bits(8);
    let block = waveform.finish();
    assert_eq!(
        bare(&decode(Speed::Full, FULL, &block)),
        vec![
            "begin p1".to_owned(),
            "pid p1 Ack".to_owned(),
            "end p1 Eop".to_owned(),
            format!("eof {}", block.end()),
        ]
    );
}

#[test]
fn clocks_off_their_rate_and_skewed_lines_decode() {
    // (speed, sample rate, transmitter off by ‰, ticks between the lines' edges)
    let cases = [
        (Speed::Full, FULL, -50i64, 0u64),
        (Speed::Full, FULL, 30, 0),
        (Speed::Full, 50_000_000, -30, 0),
        (Speed::Full, 50_000_000, 30, 0),
        (Speed::Low, LOW, -50, 0),
        (Speed::Low, LOW, 50, 0),
        (Speed::Full, FULL, 0, 2),
        (Speed::Low, LOW, 0, 4),
        (Speed::Full, 50_000_000, 0, 1),
    ];
    // The worst cases for the clock: six 1s and a stuffed 0 (0xFF), a transition in every
    // bit (0x00).
    let mut payload = vec![0xff; 12];
    payload.extend([0x00; 12]);
    payload.extend((0..40u32).map(|i| (i * 89 % 256) as u8));
    for (speed, rate, permille, skew) in cases {
        let nominal = speed.bit_rate_hz();
        let bit_rate = nominal
            .checked_add_signed(nominal as i64 * permille / 1000)
            .expect("a bit rate");
        let mut waveform = UsbWaveform::new(config(speed), CHANNELS, rate, Tick::ZERO)
            .with_bit_rate(bit_rate)
            .with_skew(skew);
        waveform.idle_bits(16);
        waveform.token(Pid::In, 0x7f, 0xf);
        waveform.idle_bits(3);
        waveform.data(Pid::Data1, &payload);
        waveform.idle_bits(3);
        waveform.handshake(Pid::Ack);
        waveform.idle_bits(8);
        let block = waveform.finish();
        assert_eq!(
            bare(&decode(speed, rate, &block)),
            vec![
                "begin p1".to_owned(),
                "pid p1 In".to_owned(),
                format!(
                    "token p1 127.15 crc {:02x}",
                    crc5(token_fields(0x7f, 0xf), 11)
                ),
                "end p1 Eop".to_owned(),
                "begin p2".to_owned(),
                "pid p2 Data1".to_owned(),
                format!("data p2 +0 {}", hex(&payload[..32])),
                format!("data p2 +32 {}", hex(&payload[32..])),
                format!("data-end p2 crc {:04x} len 64", crc16(&payload)),
                "end p2 Eop".to_owned(),
                "begin p3".to_owned(),
                "pid p3 Ack".to_owned(),
                "end p3 Eop".to_owned(),
                format!("eof {}", block.end()),
            ],
            "{speed:?} at {rate} Hz, the transmitter {permille}‰ off, the lines {skew} apart"
        );
    }
}

#[test]
fn the_source_must_sample_four_times_per_bit() {
    let build = |speed: Speed, rate: u64, dm: u8| {
        Harness::build(
            CHANNELS,
            TimeBase::new(rate),
            GraphConfig::default(),
            |graph, source| {
                let config = UsbConfig {
                    dm,
                    ..config(speed)
                };
                let usb = graph.add(UsbPackets::new(config).expect("a valid configuration"));
                graph.connect(source.samples, UsbPackets::input(usb))?;
                Ok(UsbPackets::packets(usb))
            },
        )
        .err()
    };
    assert_eq!(
        build(Speed::Full, 40_000_000, 1),
        Some(GraphError::Requirement {
            from: "logic#0.samples".into(),
            to: "usb-packets#0.logic".into(),
            key: "rateHz",
            requirement: ">= 48000000".into(),
            found: "40000000".into(),
        })
    );
    assert!(build(Speed::Full, FULL, 1).is_none());
    assert!(build(Speed::Low, 6_000_000, 1).is_none());
    assert!(matches!(
        build(Speed::Low, 5_000_000, 1),
        Some(GraphError::Requirement { key: "rateHz", .. })
    ));
    assert!(matches!(
        build(Speed::Full, FULL, 2),
        Some(GraphError::Requirement {
            key: "channels",
            ..
        })
    ));
}

#[test]
fn the_reference_crcs_leave_the_published_residuals() {
    // The checker as the specification describes it: an all-ones register shifted left, the
    // polynomial added whenever its top bit differs from the bit coming in.
    fn register(bits: &[bool], width: u32, polynomial: u32) -> u32 {
        let mask = (1 << width) - 1;
        let mut register = mask;
        for bit in bits {
            let top = (register >> (width - 1)) & 1 != 0;
            register = (register << 1) & mask;
            if top != *bit {
                register ^= polynomial;
            }
        }
        register
    }
    let lsb_first = |value: u32, count: u32| (0..count).map(move |i| (value >> i) & 1 != 0);
    for (address, endpoint) in [(0, 0), (0x15, 0xe), (0x3a, 0xa), (0x7f, 0xf), (1, 2)] {
        let fields = token_fields(address, endpoint);
        let bits: Vec<bool> = lsb_first(fields, 11)
            .chain(lsb_first(u32::from(crc5(fields, 11)), 5))
            .collect();
        assert_eq!(
            register(&bits, 5, 0b0_0101),
            0b0_1100,
            "{address}.{endpoint}"
        );
    }
    let payloads: [&[u8]; 4] = [
        &[],
        &[0x80, 0x06, 0x00, 0x01, 0x00, 0x00, 0x40, 0x00],
        &[0xff; 70],
        &[0x00, 0x01, 0x02, 0x03],
    ];
    for payload in payloads {
        let crc = crc16(payload).to_le_bytes();
        let bits: Vec<bool> = payload
            .iter()
            .chain(&crc)
            .flat_map(|byte| lsb_first(u32::from(*byte), 8))
            .collect();
        assert_eq!(register(&bits, 16, 0x8005), 0x800d, "{payload:02x?}");
    }
    // By hand: the CRC5 of a token to address 0, endpoint 0 is that of eleven 0 bits.
    let mut remainder = 0b1_1111u8;
    for _ in 0..11 {
        let top = remainder & 0b1_0000 != 0;
        remainder = (remainder << 1) & 0b1_1111;
        if top {
            remainder ^= 0b0_0101;
        }
    }
    // Inverted, it goes out most significant bit first; read least significant first, as the
    // field is, its bits are reversed.
    let sent = !remainder & 0b1_1111;
    let field = (0..5).fold(0, |field, i| field | (((sent >> (4 - i)) & 1) << i));
    assert_eq!(sent, 0b0_1000);
    assert_eq!(field, 0b0_0010);
    assert_eq!(crc5(token_fields(0, 0), 11), field);
    // So SETUP to address 0, endpoint 0 is the bytes 2D 00 10, and the GET_DESCRIPTOR request
    // for the device descriptor ends in DD 94.
    assert_eq!(
        (token_fields(0, 0) | u32::from(field) << 11).to_le_bytes()[..2],
        [0x00, 0x10]
    );
    assert_eq!(
        crc16(&[0x80, 0x06, 0x00, 0x01, 0x00, 0x00, 0x40, 0x00]).to_le_bytes(),
        [0xdd, 0x94]
    );
}

/// A transaction: its token, its data packet if any, its handshake if any.
fn exchange(
    waveform: &mut UsbWaveform,
    token: Pid,
    address: u8,
    endpoint: u8,
    data: Option<(Pid, &[u8])>,
    handshake: Option<Pid>,
) {
    waveform.token(token, address, endpoint);
    waveform.idle_bits(2);
    if let Some((pid, payload)) = data {
        waveform.data(pid, payload);
        waveform.idle_bits(2);
    }
    if let Some(pid) = handshake {
        waveform.handshake(pid);
        waveform.idle_bits(2);
    }
    waveform.idle_bits(6);
}

/// Traffic of every kind at full speed: transactions, frames, damaged packets, bus states.
fn rich_full() -> LogicBlock {
    let request = [0x80, 0x06, 0x00, 0x01, 0x00, 0x00, 0x12, 0x00];
    let descriptor: Vec<u8> = (0..40u32).map(|i| (i * 53 % 256) as u8).collect();
    let mut waveform = waveform(Speed::Full, FULL);
    let w = &mut waveform;
    w.sof(0x101);
    w.idle_bits(4);
    exchange(
        w,
        Pid::Setup,
        3,
        0,
        Some((Pid::Data0, &request)),
        Some(Pid::Ack),
    );
    exchange(
        w,
        Pid::In,
        3,
        0,
        Some((Pid::Data1, &descriptor)),
        Some(Pid::Ack),
    );
    exchange(
        w,
        Pid::Out,
        3,
        1,
        Some((Pid::Data0, &[0xff; 9])),
        Some(Pid::Nak),
    );
    exchange(w, Pid::In, 3, 2, None, Some(Pid::Stall));
    exchange(w, Pid::Ping, 3, 1, None, Some(Pid::Ack));
    exchange(w, Pid::Out, 3, 1, Some((Pid::Data1, &[])), None);
    w.token_with_crc(Pid::In, 3, 1, 0);
    w.idle_bits(4);
    w.sync();
    w.byte(Pid::Nak.byte() ^ 0x20);
    w.eop();
    w.idle_bits(4);
    w.sync();
    w.pid(Pid::Data1);
    w.unstuffed_ones(8);
    w.eop();
    w.idle_bits(4);
    w.split(0x0f_0f0f);
    w.idle_bits(4);
    w.reset(200);
    w.idle_bits(8);
    w.preamble();
    w.idle_bits(12);
    w.sof(0x102);
    w.idle_bits(8);
    waveform.finish()
}

/// Traffic at low speed, with keep-alives.
fn rich_low() -> LogicBlock {
    let mut waveform = waveform(Speed::Low, 6_000_000);
    let w = &mut waveform;
    w.keep_alive();
    w.idle_bits(6);
    exchange(
        w,
        Pid::In,
        7,
        1,
        Some((Pid::Data0, &[1, 2, 3, 4, 5, 6, 7, 8])),
        Some(Pid::Ack),
    );
    w.keep_alive();
    w.idle_bits(6);
    exchange(
        w,
        Pid::Out,
        7,
        2,
        Some((Pid::Data1, &[0xfe; 3])),
        Some(Pid::Stall),
    );
    exchange(w, Pid::In, 7, 1, None, Some(Pid::Nak));
    w.reset(40);
    w.idle_bits(6);
    w.keep_alive();
    w.idle_bits(8);
    waveform.finish()
}

fn gaps_in(block: &LogicBlock) -> Vec<TickRange> {
    let len = block.len();
    vec![
        TickRange::new(Tick(len * 3 / 10), Tick(len * 3 / 10 + 37)),
        TickRange::new(Tick(len * 7 / 10), Tick(len * 7 / 10 + 211)),
    ]
}

#[test]
fn every_cut_and_pause_yields_the_same_packets() {
    let block = rich_full();
    let gaps = gaps_in(&block);
    assert_block_invariant(|graph| packets(Speed::Full, FULL, graph), &block, &gaps);
    let block = rich_low();
    let gaps = gaps_in(&block);
    assert_block_invariant(|graph| packets(Speed::Low, 6_000_000, graph), &block, &gaps);
}

#[test]
fn rich_traffic_is_well_formed() {
    let block = rich_full();
    let mut harness = packets(Speed::Full, FULL, GraphConfig::default());
    harness.feed_with_gaps(&block, &gaps_in(&block), &[u64::MAX]);
    harness.end();
    assert_well_formed(&harness.items);
    let lines = bare(&describe(&harness.without_progress()));
    for kind in [
        "sof p",
        "Setup",
        "data-end",
        "Stall",
        "Ping",
        "Split",
        "reset",
        "Preamble",
        "PidError",
        "BitStuffError",
        "gap",
    ] {
        assert!(
            lines.iter().any(|line| line.contains(kind)),
            "no {kind} in {lines:?}"
        );
    }
    let block = rich_low();
    let mut harness = packets(Speed::Low, 6_000_000, GraphConfig::default());
    harness.run_all(&block);
    assert_well_formed(&harness.items);
    let lines = bare(&describe(&harness.without_progress()));
    assert_eq!(
        lines.iter().filter(|line| *line == "keep-alive").count(),
        3,
        "{lines:?}"
    );
    assert!(lines.contains(&"reset 40".to_owned()), "{lines:?}");
}

/// Source → USB packets → USB transactions, the packets read along as a tap.
fn stack(
    speed: Speed,
    rate: u64,
    graph: GraphConfig,
) -> (Harness<TransactionEvent>, Tap<UsbEvent>) {
    let mut exposed = None;
    let mut harness = Harness::build(CHANNELS, TimeBase::new(rate), graph, |graph, source| {
        let usb = graph.add(UsbPackets::new(config(speed)).expect("a valid configuration"));
        graph.connect(source.samples, UsbPackets::input(usb))?;
        let xact = graph.add(UsbTransactions::new());
        graph.connect(UsbPackets::packets(usb), UsbTransactions::input(xact))?;
        exposed = Some(graph.expose("packets", UsbPackets::packets(usb))?);
        Ok(UsbTransactions::transactions(xact))
    })
    .expect("builds");
    let tap = harness.tap(exposed.expect("exposed"));
    (harness, tap)
}

fn transactions_only(speed: Speed, rate: u64, graph: GraphConfig) -> Harness<TransactionEvent> {
    Harness::build(CHANNELS, TimeBase::new(rate), graph, |graph, source| {
        let usb = graph.add(UsbPackets::new(config(speed)).expect("a valid configuration"));
        graph.connect(source.samples, UsbPackets::input(usb))?;
        let xact = graph.add(UsbTransactions::new());
        graph.connect(UsbPackets::packets(usb), UsbTransactions::input(xact))?;
        Ok(UsbTransactions::transactions(xact))
    })
    .expect("builds")
}

fn transact(speed: Speed, rate: u64, block: &LogicBlock) -> Vec<String> {
    let (mut harness, _) = stack(speed, rate, GraphConfig::default());
    harness.run_all(block);
    describe_xact(&harness.without_progress())
}

fn what_xact(event: &TransactionEvent) -> String {
    match event {
        TransactionEvent::Begin {
            xact,
            token,
            address,
            endpoint,
            frame,
        } => {
            let target = match (address, endpoint, frame) {
                (Some(address), Some(endpoint), _) => format!(" {address}.{endpoint}"),
                (_, _, Some(frame)) => format!(" frame {frame}"),
                _ => String::new(),
            };
            format!("begin x{} {token:?}{target}", xact.0)
        }
        TransactionEvent::Setup {
            xact,
            bm_request_type,
            b_request,
            w_value,
            w_index,
            w_length,
        } => format!(
            "setup x{} {bm_request_type:02x} {b_request:02x} {w_value:04x} {w_index:04x} {w_length}",
            xact.0
        ),
        TransactionEvent::Data {
            xact,
            pid,
            offset,
            bytes,
        } => format!("data x{} {pid:?} +{offset} {}", xact.0, hex(bytes)),
        TransactionEvent::Handshake { xact, pid } => format!("handshake x{} {pid:?}", xact.0),
        TransactionEvent::End { xact, reason } => format!("end x{} {reason:?}", xact.0),
    }
}

/// One line per transaction stream item, with where it comes from.
fn describe_xact(items: &[Item<TransactionEvent>]) -> Vec<String> {
    lines(items, what_xact)
}

/// Checks the envelope of a transaction stream: events settle in order, one transaction is
/// open at a time, and every event belongs to the open one.
fn assert_transactions_well_formed(items: &[Item<TransactionEvent>]) {
    let mut settled = Tick::ZERO;
    let mut open: Option<XactId> = None;
    for item in items {
        let Item::Event(event) = item else { continue };
        assert!(event.settled >= settled, "settled out of order: {event:?}");
        settled = event.settled;
        match &event.payload {
            TransactionEvent::Begin { xact, .. } => {
                assert!(open.is_none(), "{xact:?} began while {open:?} was open");
                open = Some(*xact);
            }
            TransactionEvent::End { xact, .. } => {
                assert_eq!(open, Some(*xact), "{event:?}");
                open = None;
            }
            TransactionEvent::Setup { xact, .. }
            | TransactionEvent::Data { xact, .. }
            | TransactionEvent::Handshake { xact, .. } => {
                assert_eq!(open, Some(*xact), "{event:?}");
            }
        }
    }
    assert!(open.is_none(), "{open:?} left open");
}

#[test]
fn transactions_end_as_their_handshakes_say() {
    let set_configuration = [0x00, 0x09, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00];
    let mut waveform = waveform(Speed::Full, FULL);
    let w = &mut waveform;
    exchange(
        w,
        Pid::Setup,
        2,
        0,
        Some((Pid::Data0, &set_configuration)),
        Some(Pid::Ack),
    );
    exchange(
        w,
        Pid::In,
        2,
        1,
        Some((Pid::Data1, &[0x01, 0x02])),
        Some(Pid::Ack),
    );
    exchange(w, Pid::In, 2, 1, None, Some(Pid::Nak));
    exchange(w, Pid::In, 2, 1, None, Some(Pid::Stall));
    exchange(
        w,
        Pid::Out,
        2,
        2,
        Some((Pid::Data0, &[0x03])),
        Some(Pid::Ack),
    );
    exchange(
        w,
        Pid::Out,
        2,
        2,
        Some((Pid::Data1, &[0x04])),
        Some(Pid::Nak),
    );
    exchange(w, Pid::Out, 2, 2, Some((Pid::Data0, &[])), Some(Pid::Stall));
    exchange(w, Pid::Ping, 2, 2, None, Some(Pid::Ack));
    exchange(w, Pid::Ping, 2, 2, None, Some(Pid::Nak));
    exchange(
        w,
        Pid::Out,
        2,
        2,
        Some((Pid::Data1, &[0x05])),
        Some(Pid::Nyet),
    );
    let block = waveform.finish();
    assert_eq!(
        bare(&transact(Speed::Full, FULL, &block)),
        vec![
            "begin x1 Setup 2.0".to_owned(),
            "data x1 Data0 +0 0009010000000000".to_owned(),
            "setup x1 00 09 0001 0000 0".to_owned(),
            "handshake x1 Ack".to_owned(),
            "end x1 Complete".to_owned(),
            "begin x2 In 2.1".to_owned(),
            "data x2 Data1 +0 0102".to_owned(),
            "handshake x2 Ack".to_owned(),
            "end x2 Complete".to_owned(),
            "begin x3 In 2.1".to_owned(),
            "handshake x3 Nak".to_owned(),
            "end x3 Nak".to_owned(),
            "begin x4 In 2.1".to_owned(),
            "handshake x4 Stall".to_owned(),
            "end x4 Stall".to_owned(),
            "begin x5 Out 2.2".to_owned(),
            "data x5 Data0 +0 03".to_owned(),
            "handshake x5 Ack".to_owned(),
            "end x5 Complete".to_owned(),
            "begin x6 Out 2.2".to_owned(),
            "data x6 Data1 +0 04".to_owned(),
            "handshake x6 Nak".to_owned(),
            "end x6 Nak".to_owned(),
            "begin x7 Out 2.2".to_owned(),
            "data x7 Data0 +0 -".to_owned(),
            "handshake x7 Stall".to_owned(),
            "end x7 Stall".to_owned(),
            "begin x8 Ping 2.2".to_owned(),
            "handshake x8 Ack".to_owned(),
            "end x8 Complete".to_owned(),
            "begin x9 Ping 2.2".to_owned(),
            "handshake x9 Nak".to_owned(),
            "end x9 Nak".to_owned(),
            "begin x10 Out 2.2".to_owned(),
            "data x10 Data1 +0 05".to_owned(),
            "handshake x10 Nyet".to_owned(),
            "end x10 Complete".to_owned(),
            format!("eof {}", block.end()),
        ]
    );
}

#[test]
fn a_control_transfer_is_three_transactions() {
    // GET_DESCRIPTOR for the 18-byte device descriptor: the setup, data and status stages.
    let request = [0x80, 0x06, 0x00, 0x01, 0x00, 0x00, 0x12, 0x00];
    let descriptor = [
        0x12, 0x01, 0x00, 0x02, 0x00, 0x00, 0x00, 0x40, 0x09, 0x12, 0x34, 0x56, 0x00, 0x01, 0x01,
        0x02, 0x03, 0x01,
    ];
    let mut waveform = waveform(Speed::Low, LOW);
    let w = &mut waveform;
    exchange(
        w,
        Pid::Setup,
        0,
        0,
        Some((Pid::Data0, &request)),
        Some(Pid::Ack),
    );
    exchange(
        w,
        Pid::In,
        0,
        0,
        Some((Pid::Data1, &descriptor[..8])),
        Some(Pid::Ack),
    );
    exchange(
        w,
        Pid::In,
        0,
        0,
        Some((Pid::Data0, &descriptor[8..16])),
        Some(Pid::Ack),
    );
    exchange(
        w,
        Pid::In,
        0,
        0,
        Some((Pid::Data1, &descriptor[16..])),
        Some(Pid::Ack),
    );
    exchange(w, Pid::Out, 0, 0, Some((Pid::Data1, &[])), Some(Pid::Ack));
    let block = waveform.finish();
    assert_eq!(
        bare(&transact(Speed::Low, LOW, &block)),
        vec![
            "begin x1 Setup 0.0".to_owned(),
            "data x1 Data0 +0 8006000100001200".to_owned(),
            "setup x1 80 06 0100 0000 18".to_owned(),
            "handshake x1 Ack".to_owned(),
            "end x1 Complete".to_owned(),
            "begin x2 In 0.0".to_owned(),
            format!("data x2 Data1 +0 {}", hex(&descriptor[..8])),
            "handshake x2 Ack".to_owned(),
            "end x2 Complete".to_owned(),
            "begin x3 In 0.0".to_owned(),
            format!("data x3 Data0 +0 {}", hex(&descriptor[8..16])),
            "handshake x3 Ack".to_owned(),
            "end x3 Complete".to_owned(),
            "begin x4 In 0.0".to_owned(),
            format!("data x4 Data1 +0 {}", hex(&descriptor[16..])),
            "handshake x4 Ack".to_owned(),
            "end x4 Complete".to_owned(),
            "begin x5 Out 0.0".to_owned(),
            "data x5 Data1 +0 -".to_owned(),
            "handshake x5 Ack".to_owned(),
            "end x5 Complete".to_owned(),
            format!("eof {}", block.end()),
        ]
    );
}

#[test]
fn transactions_take_their_ticks_from_their_packets() {
    let mut waveform = waveform(Speed::Full, FULL);
    // IN to address 0, endpoint 0 (69 00 10), then NAK, with no stuffed bits.
    waveform.token(Pid::In, 0, 0);
    waveform.idle_bits(2);
    waveform.handshake(Pid::Nak);
    waveform.idle_bits(8);
    let block = waveform.finish();
    // The token's packet: SYNC from 64, PID from 96, fields from 128, EOP at 192 to 204;
    // the NAK's SYNC from 212, its PID from 244, its EOP at 276.
    assert_eq!(
        transact(Speed::Full, FULL, &block),
        vec![
            "begin x1 In 0.0 [64,191)@190",
            "handshake x1 Nak [212,275)@274",
            "end x1 Nak [276,285)@284",
            "eof 320",
        ]
    );
}

#[test]
fn a_transaction_left_without_an_answer_ends_when_the_bus_moves_on() {
    let mut waveform = waveform(Speed::Full, FULL);
    let w = &mut waveform;
    // OUT and its data, but no handshake: the next SOF closes it.
    exchange(w, Pid::Out, 4, 1, Some((Pid::Data0, &[0xaa])), None);
    w.idle_bits(20);
    let sof = w.position();
    w.sof(7);
    w.idle_bits(8);
    // IN and nothing: the next IN closes it.
    exchange(w, Pid::In, 4, 2, None, None);
    exchange(w, Pid::In, 4, 2, None, Some(Pid::Nak));
    // An isochronous IN, data without handshake: a reset ends it.
    exchange(w, Pid::In, 4, 3, Some((Pid::Data0, &[1, 2])), None);
    w.reset(480);
    w.idle_bits(8);
    // An IN still waiting when the samples end.
    exchange(w, Pid::In, 4, 2, None, None);
    let block = waveform.finish();
    let lines = transact(Speed::Full, FULL, &block);
    assert_eq!(
        bare(&lines),
        vec![
            "begin x1 Out 4.1".to_owned(),
            "data x1 Data0 +0 aa".to_owned(),
            "end x1 NoResponse".to_owned(),
            "begin x2 Sof frame 7".to_owned(),
            "end x2 Complete".to_owned(),
            "begin x3 In 4.2".to_owned(),
            "end x3 NoResponse".to_owned(),
            "begin x4 In 4.2".to_owned(),
            "handshake x4 Nak".to_owned(),
            "end x4 Nak".to_owned(),
            "begin x5 In 4.3".to_owned(),
            "data x5 Data0 +0 0102".to_owned(),
            "end x5 NoResponse".to_owned(),
            "begin x6 In 4.2".to_owned(),
            "end x6 Aborted".to_owned(),
            format!("eof {}", block.end()),
        ]
    );
    // No answer is known once the SOF's PID has shown it to be no answer.
    assert_eq!(
        lines[2],
        format!("end x1 NoResponse @{}", sof.offset(4 * 15 + 2))
    );
    assert_eq!(lines[14], format!("end x6 Aborted @{}", block.end()));
}

#[test]
fn damaged_and_unexpected_packets_end_a_transaction_in_error() {
    let mut waveform = waveform(Speed::Full, FULL);
    let w = &mut waveform;
    // OUT whose data fails its CRC16.
    w.token(Pid::Out, 5, 1);
    w.idle_bits(2);
    w.data_with_crc(Pid::Data0, &[1, 2, 3], crc16(&[1, 2, 3]) ^ 0x8000);
    w.idle_bits(8);
    // IN answered by ACK, which only the host sends, after data.
    exchange(w, Pid::In, 5, 1, None, Some(Pid::Ack));
    // OUT whose data packet breaks the stuffing rule.
    w.token(Pid::Out, 5, 1);
    w.idle_bits(2);
    w.sync();
    w.pid(Pid::Data1);
    w.unstuffed_ones(8);
    w.eop();
    w.idle_bits(8);
    // A token whose CRC5 fails, which the device ignores: its data and ACK belong to nothing.
    w.token_with_crc(Pid::Out, 5, 1, crc5(token_fields(5, 1), 11) ^ 1);
    w.idle_bits(2);
    w.data(Pid::Data0, &[9]);
    w.idle_bits(2);
    w.handshake(Pid::Ack);
    w.idle_bits(8);
    // OUT with a second data packet where the handshake belongs.
    exchange(w, Pid::Out, 5, 1, Some((Pid::Data0, &[1])), None);
    w.data(Pid::Data1, &[2]);
    w.idle_bits(8);
    // A failed PID check where the handshake belongs.
    exchange(w, Pid::Out, 5, 1, Some((Pid::Data1, &[3])), None);
    w.sync();
    w.byte(Pid::Ack.byte() ^ 0x10);
    w.eop();
    w.idle_bits(8);
    let block = waveform.finish();
    assert_eq!(
        bare(&transact(Speed::Full, FULL, &block)),
        vec![
            "begin x1 Out 5.1".to_owned(),
            "data x1 Data0 +0 010203".to_owned(),
            "end x1 Error".to_owned(),
            "begin x2 In 5.1".to_owned(),
            "handshake x2 Ack".to_owned(),
            "end x2 Error".to_owned(),
            "begin x3 Out 5.1".to_owned(),
            "end x3 Error".to_owned(),
            "begin x4 Out 5.1".to_owned(),
            "end x4 Error".to_owned(),
            "begin x5 Out 5.1".to_owned(),
            "data x5 Data0 +0 01".to_owned(),
            "end x5 Error".to_owned(),
            "begin x6 Out 5.1".to_owned(),
            "data x6 Data1 +0 03".to_owned(),
            "end x6 Error".to_owned(),
            format!("eof {}", block.end()),
        ]
    );
}

#[test]
fn a_gap_aborts_the_open_transaction() {
    let payload: Vec<u8> = (0..20).collect();
    let mut waveform = waveform(Speed::Full, FULL);
    let w = &mut waveform;
    w.token(Pid::In, 1, 1);
    w.idle_bits(2);
    let sync = w.position();
    w.data(Pid::Data1, &payload);
    w.idle_bits(2);
    w.handshake(Pid::Ack);
    w.idle_bits(8);
    exchange(w, Pid::In, 1, 1, None, Some(Pid::Nak));
    let block = waveform.finish();
    let gap = TickRange::new(sync.offset(4 * 80), sync.offset(4 * 90));
    let (mut harness, _) = stack(Speed::Full, FULL, GraphConfig::default());
    harness.feed_with_gaps(&block, &[gap], &[100]);
    harness.end();
    let lines = describe_xact(&harness.without_progress());
    assert_eq!(
        bare(&lines),
        vec![
            "begin x1 In 1.1".to_owned(),
            format!("data x1 Data1 +0 {}", hex(&payload[..6])),
            "end x1 Aborted".to_owned(),
            format!("gap {gap}"),
            "begin x2 In 1.1".to_owned(),
            "handshake x2 Nak".to_owned(),
            "end x2 Nak".to_owned(),
            format!("eof {}", block.end()),
        ]
    );
    assert_eq!(lines[2], format!("end x1 Aborted @{}", gap.start));
}

#[test]
fn the_transactions_node_reads_usb_packets_only() {
    // A graph built at run time types its ports by their schemas alone, as this one does.
    let error = Harness::build(
        4,
        TimeBase::new(FULL),
        GraphConfig::default(),
        |graph, source| {
            let spi = graph.add(Spi::new(SpiConfig::default()).expect("a valid configuration"));
            graph.connect(source.samples, Spi::input(spi))?;
            let xact = graph.add(UsbTransactions::new());
            graph.connect(Spi::events(spi), InputRef::<SpiEvent>::of_node(xact, 0))?;
            Ok(UsbTransactions::transactions(xact))
        },
    )
    .expect_err("refused");
    assert_eq!(
        error,
        GraphError::SchemaMismatch {
            from: "spi#0.events".into(),
            to: "usb-transactions#1.packets".into(),
            output: SPI_EVENTS,
            input: USB_PACKETS,
        }
    );
    assert!(
        UsbTransactions::requires()
            .unmet(&config(Speed::Low).props())
            .is_none()
    );
}

#[test]
fn every_cut_and_pause_yields_the_same_transactions() {
    let block = rich_full();
    let gaps = gaps_in(&block);
    assert_block_invariant(
        |graph| transactions_only(Speed::Full, FULL, graph),
        &block,
        &gaps,
    );
}

#[test]
fn queues_stay_bounded_on_a_long_burst() {
    let payload: Vec<u8> = (0..1023u32).map(|i| (i % 251) as u8).collect();
    let mut waveform = waveform(Speed::Full, FULL);
    for _ in 0..4 {
        exchange(
            &mut waveform,
            Pid::Out,
            1,
            1,
            Some((Pid::Data0, &payload)),
            Some(Pid::Ack),
        );
    }
    let block = waveform.finish();
    let config = GraphConfig {
        queue_capacity: 4,
        budget: 16,
    };
    let (mut harness, packets) = stack(Speed::Full, FULL, config);
    harness.drain_on_full = 1;
    harness.feed_cut(&block, &[], &[997], &[2]);
    harness.end();
    let stats = harness.pipeline().stats();
    assert!(stats.within_capacity(), "{stats:?}");
    assert!(stats.high_water() <= 4, "{stats:?}");
    let bytes: Vec<u8> = harness
        .events()
        .into_iter()
        .filter_map(|event| match event.payload {
            TransactionEvent::Data { bytes, .. } => Some(bytes),
            _ => None,
        })
        .flatten()
        .collect();
    assert_eq!(bytes.len(), 4 * payload.len());
    assert!(bytes.chunks(payload.len()).all(|chunk| chunk == payload));
    assert_well_formed(&packets.items());
}

/// A stretch of random traffic.
#[derive(Clone, Debug)]
enum Segment {
    Token {
        pid: Pid,
        address: u8,
        endpoint: u8,
        damaged: bool,
    },
    Sof {
        frame: u16,
    },
    Data {
        pid: Pid,
        len: usize,
        damaged: bool,
    },
    Handshake {
        pid: Pid,
    },
    BadPid {
        code: u8,
        flip: u8,
    },
    StuffError,
    Reset {
        ticks: u64,
    },
    Preamble,
    Split {
        raw: u32,
    },
}

fn segment() -> impl Strategy<Value = Segment> {
    let token = prop::sample::select(vec![Pid::Out, Pid::In, Pid::Setup, Pid::Ping]);
    let data = prop::sample::select(vec![Pid::Data0, Pid::Data1, Pid::Data2, Pid::Mdata]);
    let handshake = prop::sample::select(vec![Pid::Ack, Pid::Nak, Pid::Stall, Pid::Nyet]);
    prop_oneof![
        6 => (token, 0u8..128, 0u8..16, prop::bool::weighted(0.1)).prop_map(
            |(pid, address, endpoint, damaged)| Segment::Token { pid, address, endpoint, damaged }
        ),
        2 => (0u16..2048).prop_map(|frame| Segment::Sof { frame }),
        5 => (data, 0usize..80, prop::bool::weighted(0.1))
            .prop_map(|(pid, len, damaged)| Segment::Data { pid, len, damaged }),
        5 => handshake.prop_map(|pid| Segment::Handshake { pid }),
        1 => (0u8..16, 0u8..4).prop_map(|(code, flip)| Segment::BadPid { code, flip }),
        1 => Just(Segment::StuffError),
        1 => (100u64..400).prop_map(|ticks| Segment::Reset { ticks }),
        1 => Just(Segment::Preamble),
        1 => (0u32..0x100_0000).prop_map(|raw| Segment::Split { raw }),
    ]
}

fn traffic(segments: &[Segment]) -> LogicBlock {
    let mut waveform = waveform(Speed::Full, FULL);
    let w = &mut waveform;
    for (n, segment) in segments.iter().enumerate() {
        match segment {
            Segment::Token {
                pid,
                address,
                endpoint,
                damaged,
            } => {
                let crc = crc5(token_fields(*address, *endpoint), 11) ^ u8::from(*damaged);
                w.token_with_crc(*pid, *address, *endpoint, crc);
            }
            Segment::Sof { frame } => w.sof(*frame),
            Segment::Data { pid, len, damaged } => {
                let payload: Vec<u8> = (0..*len).map(|i| (i * 29 + n * 7) as u8).collect();
                let crc = crc16(&payload) ^ if *damaged { 0x0100 } else { 0 };
                w.data_with_crc(*pid, &payload, crc);
            }
            Segment::Handshake { pid } => w.handshake(*pid),
            Segment::BadPid { code, flip } => {
                w.sync();
                w.byte(Pid::from_code(*code).byte() ^ (0x10 << flip));
                w.eop();
            }
            Segment::StuffError => {
                w.sync();
                w.pid(Pid::Data0);
                w.unstuffed_ones(8);
                w.eop();
            }
            Segment::Reset { ticks } => w.reset(*ticks),
            Segment::Preamble => {
                w.preamble();
                w.idle_bits(9);
            }
            Segment::Split { raw } => w.split(*raw),
        }
        w.idle_bits(2 + (n as u64 % 4) * 3);
    }
    w.idle_bits(8);
    waveform.finish()
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 48, ..ProptestConfig::default() })]

    /// Blocks of any size, a reader that pauses at random, tiny queues and budgets, samples
    /// cut short or lost in a gap: the packets and the transactions are those of one
    /// uninterrupted run, and well formed.
    #[test]
    fn random_traffic_decodes_alike_however_it_is_fed(
        segments in prop::collection::vec(segment(), 1..12),
        sizes in prop::collection::vec(1u64..300, 1..6),
        pauses in prop::collection::vec(0usize..12, 1..8),
        queue_capacity in 1usize..6,
        budget in 1u32..40,
        cut in prop::option::of(0.05f64..1.0),
        gap in prop::option::of((0.05f64..0.95, 1u64..300)),
    ) {
        let whole = traffic(&segments);
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
        let (mut reference, reference_packets) = stack(Speed::Full, FULL, GraphConfig::default());
        reference.feed_with_gaps(&block, &gaps, &[u64::MAX]);
        reference.end();
        assert_well_formed(&reference_packets.items());
        assert_transactions_well_formed(&reference.items);

        let config = GraphConfig { queue_capacity, budget };
        let (mut harness, packets) = stack(Speed::Full, FULL, config);
        harness.drain_on_full = 1;
        harness.feed_cut(&block, &gaps, &sizes, &pauses);
        harness.end();
        prop_assert_eq!(harness.without_progress(), reference.without_progress());
        prop_assert_eq!(packets.without_progress(), reference_packets.without_progress());
        let stats = harness.pipeline().stats();
        prop_assert!(stats.within_capacity(), "{stats:?}");
    }
}
