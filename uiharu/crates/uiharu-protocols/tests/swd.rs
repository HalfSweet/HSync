// SPDX-License-Identifier: Apache-2.0

//! The SWD decoder on synthesized waveforms.

use proptest::prelude::*;
use serde::Serialize;
use uiharu_core::testing::{Harness, assert_block_invariant};
use uiharu_core::{
    EventSchema, GraphConfig, InputRef, InputSpec, Io, Item, LogicBlock, Node, NodeFuture,
    NodeSpec, Outbox, OutputRef, OutputSpec, Props, Requires, SchemaId, Tick, TickRange, TimeBase,
};
use uiharu_protocols::swd::synth::{SwdPacket, SwdWaveform};
use uiharu_protocols::swd::{
    ACTIVATE_JTAG_DP, ACTIVATE_SWD, Ack, Data, EndReason, Field, Interface, Next, OverrunDetect,
    Read, Request, Swd, SwdBus, SwdConfig, SwdEvent, TxId,
};

/// SWCLK and SWDIO on channels 0 and 1.
const CHANNELS: u8 = 2;
/// Samples per half SWCLK period: a clock takes four samples and rises at its third.
const HALF: u64 = 2;
const PERIOD: u64 = 2 * HALF;
/// Samples before the first clock.
const LEAD: u64 = 4;

fn waveform() -> SwdWaveform {
    let mut waveform = SwdWaveform::new(SwdConfig::default(), CHANNELS, HALF, Tick::ZERO);
    waveform.idle(LEAD);
    waveform
}

fn harness_as(config: &SwdConfig, graph: GraphConfig) -> Harness<SwdEvent> {
    Harness::build(
        CHANNELS,
        TimeBase::new(1_000_000),
        graph,
        |graph, source| {
            let swd = graph.add(Swd::new(config.clone()).expect("a valid configuration"));
            graph.connect(source.samples, Swd::input(swd))?;
            Ok(Swd::events(swd))
        },
    )
    .expect("builds")
}

fn harness(graph: GraphConfig) -> Harness<SwdEvent> {
    harness_as(&SwdConfig::default(), graph)
}

fn items_as(config: &SwdConfig, block: &LogicBlock, gaps: &[TickRange]) -> Vec<Item<SwdEvent>> {
    let mut harness = harness_as(config, GraphConfig::default());
    harness.feed_with_gaps(block, gaps, &[89]);
    harness.end();
    let items = harness.without_progress();
    assert_settled_in_order(&items);
    items
}

fn items(block: &LogicBlock, gaps: &[TickRange]) -> Vec<Item<SwdEvent>> {
    items_as(&SwdConfig::default(), block, gaps)
}

fn decode_as(config: &SwdConfig, block: &LogicBlock) -> Vec<String> {
    describe(&items_as(config, block, &[]))
}

fn decode(block: &LogicBlock) -> Vec<String> {
    describe(&items(block, &[]))
}

fn decode_with_gap(block: &LogicBlock, gap: TickRange) -> Vec<String> {
    describe(&items(block, &[gap]))
}

/// No event of the stream is settled before an earlier one.
fn assert_settled_in_order(items: &[Item<SwdEvent>]) {
    let mut last = Tick::ZERO;
    for event in items.iter().filter_map(Item::event) {
        assert!(event.settled >= last, "{event:?} settles before {last}");
        assert!(event.span.start <= event.settled, "{event:?}");
        last = event.settled;
    }
}

fn name(value: &impl Serialize) -> String {
    serde_json::to_value(value)
        .expect("serializes")
        .as_str()
        .expect("a name")
        .to_owned()
}

fn ack_name(ack: Ack) -> String {
    match ack {
        Ack::Other(bits) => format!("other({bits})"),
        known => name(&known),
    }
}

fn at(span: TickRange, settled: Tick) -> String {
    if span.is_empty() {
        format!("@{settled}")
    } else {
        format!("[{},{})@{settled}", span.start, span.end)
    }
}

fn what(event: &SwdEvent) -> String {
    match *event {
        SwdEvent::Begin { tx } => format!("begin p{}", tx.0),
        SwdEvent::Request {
            tx,
            apndp,
            rnw,
            addr,
            parity_ok,
        } => format!(
            "request p{} {} {} {addr:#x}{}",
            tx.0,
            if apndp { "ap" } else { "dp" },
            if rnw { "read" } else { "write" },
            if parity_ok { "" } else { " bad-parity" }
        ),
        SwdEvent::Ack { tx, value } => format!("ack p{} {}", tx.0, ack_name(value)),
        SwdEvent::Data {
            tx,
            value,
            parity_ok,
            from_target,
        } => format!(
            "data p{} {value:08x} {}{}",
            tx.0,
            if from_target { "target" } else { "host" },
            if parity_ok { "" } else { " bad-parity" }
        ),
        SwdEvent::End { tx, reason } => format!("end p{} {}", tx.0, name(&reason)),
        SwdEvent::LineReset => "line-reset".to_owned(),
        SwdEvent::Switch { to } => format!("switch {}", name(&to)),
    }
}

/// One line per item, readable in assertions.
fn describe(items: &[Item<SwdEvent>]) -> Vec<String> {
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

/// The events among `items`, without their provenance.
fn payloads(items: &[Item<SwdEvent>]) -> Vec<SwdEvent> {
    items
        .iter()
        .filter_map(Item::event)
        .map(|event| event.payload)
        .collect()
}

/// The tick at which the clock starting at `position` rises.
fn rise_at(position: u64) -> u64 {
    position + HALF
}

/// An event decided at the sample `tick`.
fn one(what: &str, tick: u64) -> String {
    format!("{what} [{tick},{})@{tick}", tick + 1)
}

/// The lines of packet `tx`, whose first clock starts at `position`, up to its end: well
/// formed, or broken where the knobs say; turnarounds last one clock.
fn packet_lines(tx: u64, packet: &SwdPacket, position: u64) -> Vec<String> {
    packet_lines_as(tx, packet, position, 1)
}

/// The lines of packet `tx` on a port whose turnarounds last `trn` clocks.
fn packet_lines_as(tx: u64, packet: &SwdPacket, position: u64, trn: u64) -> Vec<String> {
    let rise = |index: u64| position + index * PERIOD + HALF;
    let (ack_first, ack_last) = (8 + trn, 10 + trn);
    let request = packet.request_bits();
    // The packet is known from the first low bit of its request.
    let decided = (1..8).find(|i| request >> i & 1 == 0).map_or(rise(8), rise);
    let event = SwdEvent::Request {
        tx: TxId(tx),
        apndp: packet.apndp,
        rnw: packet.rnw,
        addr: packet.addr & 0xc,
        parity_ok: !packet.bad_request_parity,
    };
    let mut lines = vec![
        format!("begin p{tx} [{},{})@{decided}", rise(0), rise(0) + 1),
        format!(
            "{} [{},{})@{}",
            what(&event),
            rise(0),
            rise(7) + 1,
            rise(7).max(decided)
        ),
    ];
    if packet.bad_request_parity || packet.stop || !packet.park {
        lines.push(format!(
            "end p{tx} protocol-error [{},{})@{}",
            rise(7),
            rise(7) + 1,
            rise(7).max(decided)
        ));
        return lines;
    }
    lines.push(format!(
        "ack p{tx} {} [{},{})@{}",
        ack_name(packet.ack),
        rise(ack_first),
        rise(ack_last) + 1,
        rise(ack_last)
    ));
    match packet.ack {
        // A TARGETSEL write's data phase follows whatever the acknowledge showed.
        _ if packet.ack == Ack::Ok || packet.is_targetsel() => {
            let (first, parity) = if packet.rnw {
                (ack_last + 1, ack_last + 33)
            } else {
                (ack_last + 1 + trn, ack_last + 33 + trn)
            };
            let event = SwdEvent::Data {
                tx: TxId(tx),
                value: packet.data,
                parity_ok: !packet.bad_data_parity,
                from_target: packet.rnw,
            };
            lines.push(format!(
                "{} [{},{})@{}",
                what(&event),
                rise(first),
                rise(parity) + 1,
                rise(parity)
            ));
            if packet.bad_data_parity {
                lines.push(one(&format!("end p{tx} protocol-error"), rise(parity)));
            } else {
                lines.push(one(
                    &format!("end p{tx} complete"),
                    rise(ack_last + 33 + trn),
                ));
            }
        }
        // Under overrun detection, a data phase with nothing to report comes first.
        Ack::Wait if packet.orundetect => {
            lines.push(one(&format!("end p{tx} wait"), rise(ack_last + 33 + trn)));
        }
        Ack::Fault if packet.orundetect => {
            lines.push(one(&format!("end p{tx} fault"), rise(ack_last + 33 + trn)));
        }
        Ack::Wait => lines.push(one(&format!("end p{tx} wait"), rise(ack_last + trn))),
        Ack::Fault => lines.push(one(&format!("end p{tx} fault"), rise(ack_last + trn))),
        Ack::Ok | Ack::Other(_) => {
            lines.push(one(&format!("end p{tx} protocol-error"), rise(ack_last)));
        }
    }
    lines
}

/// The line of a line reset of the clocks `from..to` (their first ticks), decided by the next.
fn line_reset(from: u64, to: u64) -> String {
    let last = rise_at(to - PERIOD);
    format!(
        "line-reset [{},{})@{}",
        rise_at(from),
        last + 1,
        rise_at(to)
    )
}

/// The line of a switch to `to` whose `bits` bits start with the clock at `from`.
fn switch_line(to: &str, from: u64, bits: u64) -> String {
    let last = rise_at(from + (bits - 1) * PERIOD);
    format!("switch {to} [{},{})@{last}", rise_at(from), last + 1)
}

/// A debugger connecting at `from`: the lines of its line resets and select sequence.
fn connect_lines(from: u64) -> Vec<String> {
    let select = from + 56 * PERIOD;
    let second = select + 16 * PERIOD;
    let idle = second + 56 * PERIOD;
    vec![
        line_reset(from, select),
        switch_line("swd", select, 16),
        line_reset(second, idle),
    ]
}

/// Bits in a wake: the Selection Alert, the four clocks the target ignores and a code of
/// `code_bits` bits.
fn wake_bits(code_bits: u64) -> u64 {
    128 + 4 + code_bits
}

#[test]
fn reads_and_writes_are_decoded_with_every_acknowledge() {
    let mut waveform = waveform();
    waveform.connect();
    let packets = [
        SwdPacket::read(false, 0x0, 0x2ba0_1477),
        SwdPacket::write(false, 0x8, 0x0000_00f0),
        SwdPacket::write(false, 0x4, 0x5000_0000),
        SwdPacket::read(true, 0xc, 0).with_ack(Ack::Wait),
        SwdPacket::write(true, 0x4, 0x2300_0012).with_ack(Ack::Wait),
        SwdPacket::read(true, 0xc, 0).with_ack(Ack::Fault),
        SwdPacket::write(false, 0x4, 0x5000_0000).with_ack(Ack::Fault),
        SwdPacket::read(true, 0x0, 0xffff_ffff),
    ];
    let mut expected = connect_lines(LEAD);
    for (n, packet) in packets.iter().enumerate() {
        let position = waveform.position().get();
        waveform.packet(packet);
        waveform.idle_cycles(2);
        expected.extend(packet_lines(n as u64 + 1, packet, position));
    }
    waveform.idle(4);
    let block = waveform.finish();
    expected.push(format!("eof {}", block.end()));
    assert_eq!(decode(&block), expected);
}

#[test]
fn the_targets_bits_are_read_at_the_falling_edge_whatever_its_delay() {
    // The target changes SWDIO on the rising edge of SWCLK (ADIv6 B4.3.1); the analyzer sees
    // the change in the edge's own sample or some later. The same traffic decodes the same.
    let steps = [
        Step::Switch(Interface::Swd, 56, 2),
        Step::Packet(SwdPacket::read(false, 0x0, 0x2ba0_1477), 0),
        Step::Packet(SwdPacket::write(false, 0x8, 0x0000_00f0), 1),
        Step::Packet(SwdPacket::read(true, 0xc, 0xffff_0000), 0),
        Step::Packet(SwdPacket::read(true, 0xc, 0).with_ack(Ack::Wait), 0),
        Step::Packet(
            SwdPacket::write(true, 0x4, 0x2300_0012).with_ack(Ack::Fault),
            2,
        ),
        Step::Packet(SwdPacket::read(true, 0x0, 0x5555_aaaa), 0),
    ];
    let (expected, steps) = simulate(&steps);
    assert!(expected.len() > 20, "{expected:?}");
    let half = 8;
    for delay in [0, 1, half - 1] {
        let mut waveform = SwdWaveform::new(SwdConfig::default(), CHANNELS, half, Tick::ZERO)
            .with_target_delay(delay);
        waveform.idle(LEAD);
        let block = build_on(waveform, &steps);
        assert_eq!(payloads(&items(&block, &[])), expected, "delay {delay}");
    }
}

#[test]
fn back_to_back_packets_are_decoded_with_and_without_idle_cycles() {
    let mut waveform = waveform();
    waveform.connect();
    let packets = [
        (SwdPacket::read(false, 0x0, 0x0bc1_1477), 0),
        (SwdPacket::write(true, 0x0, 0xa200_0002), 0),
        (
            SwdPacket::read(true, 0xc, 0x8000_0000).with_ack(Ack::Wait),
            0,
        ),
        (SwdPacket::read(true, 0xc, 0x8000_0000), 3),
        (SwdPacket::write(false, 0x8, 0x0000_0003), 1),
        (SwdPacket::read(false, 0x4, 0xf000_0040), 0),
    ];
    let mut expected = connect_lines(LEAD);
    for (n, (packet, idle)) in packets.iter().enumerate() {
        let position = waveform.position().get();
        waveform.packet(packet);
        waveform.idle_cycles(*idle);
        expected.extend(packet_lines(n as u64 + 1, packet, position));
    }
    waveform.idle(4);
    let block = waveform.finish();
    expected.push(format!("eof {}", block.end()));
    assert_eq!(decode(&block), expected);
}

#[test]
fn a_data_parity_error_ends_the_packet_and_waits_for_a_line_reset() {
    for rnw in [true, false] {
        let mut waveform = waveform();
        waveform.connect();
        let broken = SwdPacket {
            rnw,
            bad_data_parity: true,
            ..SwdPacket::read(true, 0xc, 0x1234_5678)
        };
        let position = waveform.position().get();
        waveform.packet(&broken);
        waveform.idle_cycles(2);
        // Not decoded: the decoder no longer knows where packets start.
        waveform.packet(&SwdPacket::read(false, 0x0, 1));
        waveform.idle_cycles(2);
        let reset = waveform.position().get();
        waveform.line_reset(50);
        let idle = waveform.position().get();
        waveform.idle_cycles(2);
        let again = waveform.position().get();
        let packet = SwdPacket::read(false, 0x4, 0xf000_0000);
        waveform.packet(&packet);
        waveform.idle(4);
        let block = waveform.finish();
        let mut expected = connect_lines(LEAD);
        expected.extend(packet_lines(1, &broken, position));
        expected.push(line_reset(reset, idle));
        expected.extend(packet_lines(2, &packet, again));
        expected.push(format!("eof {}", block.end()));
        assert_eq!(decode(&block), expected, "RnW {rnw}");
    }
}

#[test]
fn malformed_requests_end_the_packet_on_their_park_bit() {
    let base = SwdPacket::read(false, 0x0, 0).with_ack(Ack::Other(7));
    let cases = [
        SwdPacket {
            bad_request_parity: true,
            ..base
        },
        SwdPacket { stop: true, ..base },
        SwdPacket {
            park: false,
            ..base
        },
    ];
    for broken in cases {
        let mut waveform = waveform();
        waveform.connect();
        let position = waveform.position().get();
        waveform.packet(&broken);
        waveform.idle_cycles(1);
        let reset = waveform.position().get();
        waveform.line_reset(50);
        let idle = waveform.position().get();
        waveform.idle_cycles(3);
        let again = waveform.position().get();
        let packet = SwdPacket::write(false, 0x8, 0x0f);
        waveform.packet(&packet);
        waveform.idle(4);
        let block = waveform.finish();
        let mut expected = connect_lines(LEAD);
        expected.extend(packet_lines(1, &broken, position));
        expected.push(line_reset(reset, idle));
        expected.extend(packet_lines(2, &packet, again));
        expected.push(format!("eof {}", block.end()));
        assert_eq!(decode(&block), expected, "{broken:?}");
    }
}

/// `packet` with overrun detection on.
fn under_overrun(packet: SwdPacket) -> SwdPacket {
    SwdPacket {
        orundetect: true,
        ..packet
    }
}

#[test]
fn overrun_detection_is_followed_from_the_writes_to_ctrl_stat() {
    let mut waveform = waveform();
    waveform.connect();
    let mut expected = connect_lines(LEAD);
    let mut tx = 0;
    let mut send = |waveform: &mut SwdWaveform, expected: &mut Vec<String>, packet: SwdPacket| {
        tx += 1;
        let position = waveform.position().get();
        waveform.packet(&packet);
        waveform.idle_cycles(1);
        expected.extend(packet_lines(tx, &packet, position));
    };
    let wait = SwdPacket::read(true, 0xc, 0).with_ack(Ack::Wait);
    // Off after a powerup reset: WAIT ends with its turnaround.
    send(&mut waveform, &mut expected, wait);
    // CTRL/STAT.ORUNDETECT set: WAIT and FAULT carry a data phase from now on.
    send(
        &mut waveform,
        &mut expected,
        SwdPacket::write(false, 0x4, 0x5000_0001),
    );
    send(&mut waveform, &mut expected, under_overrun(wait));
    send(
        &mut waveform,
        &mut expected,
        under_overrun(SwdPacket::write(true, 0x4, 0x1234_5678).with_ack(Ack::Fault)),
    );
    // With SELECT.DPBANKSEL at 1, address 0x4 is DLCR: a write there changes nothing, and
    // neither does a write to CTRL/STAT the target answers WAIT.
    send(
        &mut waveform,
        &mut expected,
        SwdPacket::write(false, 0x8, 0x0000_0001),
    );
    send(
        &mut waveform,
        &mut expected,
        under_overrun(SwdPacket::write(false, 0x4, 0x0000_0000)),
    );
    send(
        &mut waveform,
        &mut expected,
        under_overrun(SwdPacket::write(false, 0x8, 0).with_ack(Ack::Wait)),
    );
    send(&mut waveform, &mut expected, under_overrun(wait));
    // A line reset puts the bank back to 0; CTRL/STAT keeps its value.
    let reset = waveform.position().get();
    waveform.line_reset(50);
    let idle = waveform.position().get();
    waveform.idle_cycles(2);
    expected.push(line_reset(reset, idle));
    send(&mut waveform, &mut expected, under_overrun(wait));
    send(
        &mut waveform,
        &mut expected,
        under_overrun(SwdPacket::write(false, 0x4, 0x5000_0000).with_ack(Ack::Wait)),
    );
    send(&mut waveform, &mut expected, under_overrun(wait));
    // Cleared: WAIT ends with its turnaround again.
    send(
        &mut waveform,
        &mut expected,
        SwdPacket::write(false, 0x4, 0x5000_0000),
    );
    send(&mut waveform, &mut expected, wait);
    // A write whose data fails its parity does not take effect; after the protocol error the
    // decoder waits for a line reset.
    send(
        &mut waveform,
        &mut expected,
        SwdPacket {
            bad_data_parity: true,
            ..SwdPacket::write(false, 0x4, 0x5000_0001)
        },
    );
    let reset = waveform.position().get();
    waveform.line_reset(50);
    let idle = waveform.position().get();
    waveform.idle_cycles(2);
    expected.push(line_reset(reset, idle));
    send(&mut waveform, &mut expected, wait);
    waveform.idle(4);
    let block = waveform.finish();
    expected.push(format!("eof {}", block.end()));
    assert_eq!(decode(&block), expected);
}

#[test]
fn overrun_detection_can_be_set_outright() {
    for (setting, on) in [(OverrunDetect::On, true), (OverrunDetect::Off, false)] {
        let config = SwdConfig {
            overrun_detect: setting,
            ..SwdConfig::default()
        };
        let mut waveform = waveform();
        waveform.connect();
        let mut expected = connect_lines(LEAD);
        let wait = SwdPacket {
            orundetect: on,
            ..SwdPacket::read(true, 0xc, 0).with_ack(Ack::Wait)
        };
        // The writes to CTRL/STAT say the opposite and change nothing.
        let packets = [
            wait,
            SwdPacket::write(false, 0x4, 0x5000_0000 | u32::from(!on)),
            wait,
        ];
        for (n, packet) in packets.iter().enumerate() {
            let position = waveform.position().get();
            waveform.packet(packet);
            waveform.idle_cycles(2);
            expected.extend(packet_lines(n as u64 + 1, packet, position));
        }
        waveform.idle(4);
        let block = waveform.finish();
        expected.push(format!("eof {}", block.end()));
        assert_eq!(decode_as(&config, &block), expected, "{setting:?}");
    }
}

#[test]
fn the_turnaround_lasts_as_many_clocks_as_configured() {
    for turnaround in [2u8, 4] {
        let config = SwdConfig {
            turnaround,
            ..SwdConfig::default()
        };
        let mut waveform = SwdWaveform::new(config.clone(), CHANNELS, HALF, Tick::ZERO);
        waveform.idle(LEAD);
        waveform.connect();
        let packets = [
            SwdPacket::read(false, 0x0, 0x2ba0_1477),
            SwdPacket::write(false, 0x8, 0x0000_00f0),
            SwdPacket::read(true, 0xc, 0).with_ack(Ack::Wait),
            SwdPacket::write(false, 0x4, 0x5000_0000).with_ack(Ack::Fault),
            SwdPacket::targetsel(0x0100_2927),
            SwdPacket::read(true, 0x0, 0xffff_ffff),
            SwdPacket::read(false, 0x0, 0).with_ack(Ack::Other(7)),
        ];
        let mut expected = connect_lines(LEAD);
        for (n, packet) in packets.iter().enumerate() {
            let position = waveform.position().get();
            waveform.packet(packet);
            waveform.idle_cycles(n as u32 % 2);
            expected.extend(packet_lines_as(
                n as u64 + 1,
                packet,
                position,
                u64::from(turnaround),
            ));
        }
        waveform.idle(4);
        let block = waveform.finish();
        expected.push(format!("eof {}", block.end()));
        assert_eq!(
            decode_as(&config, &block),
            expected,
            "turnaround {turnaround}"
        );
    }
}

#[test]
fn a_targetsel_write_goes_on_without_an_acknowledge() {
    // Multi-drop target selection (ADIv6 B4.3.4): a line reset, a TARGETSEL write that no
    // target acknowledges, then the DPIDR read and whatever follows, with no line reset between.
    let mut waveform = waveform();
    waveform.connect();
    let mut expected = connect_lines(LEAD);
    let mut tx = 0;
    let mut select =
        |waveform: &mut SwdWaveform, expected: &mut Vec<String>, packets: &[SwdPacket]| {
            let reset = waveform.position().get();
            waveform.line_reset(56);
            let idle = waveform.position().get();
            waveform.idle_cycles(2);
            expected.push(line_reset(reset, idle));
            for packet in packets {
                tx += 1;
                let position = waveform.position().get();
                waveform.packet(packet);
                waveform.idle_cycles(2);
                expected.extend(packet_lines(tx, packet, position));
            }
        };
    select(
        &mut waveform,
        &mut expected,
        &[
            SwdPacket::targetsel(0x0100_2927),
            SwdPacket::read(false, 0x0, 0x0bc1_2477),
            SwdPacket::read(false, 0x4, 0xf000_0040),
        ],
    );
    // Whatever the three undriven bits happened to show, the data phase follows.
    select(
        &mut waveform,
        &mut expected,
        &[
            SwdPacket::targetsel(0x1100_2927).with_ack(Ack::Other(5)),
            SwdPacket::read(false, 0x0, 0x0bc1_2477),
        ],
    );
    // A parity error in the data of a TARGETSEL write is a protocol error (B4.3.4): the
    // decoder waits for the next line reset, so the DPIDR read after it is not decoded.
    select(
        &mut waveform,
        &mut expected,
        &[SwdPacket {
            bad_data_parity: true,
            ..SwdPacket::targetsel(0x0100_2927)
        }],
    );
    waveform.packet(&SwdPacket::read(false, 0x0, 0x0bc1_2477));
    waveform.idle(4);
    let block = waveform.finish();
    expected.push(format!("eof {}", block.end()));
    let lines = decode(&block);
    assert_eq!(lines, expected);
    assert!(lines.iter().any(|line| line.starts_with("ack p1 other(7)")));
    assert!(lines.iter().any(|line| line.starts_with("end p1 complete")));
    assert!(
        lines
            .iter()
            .any(|line| line.starts_with("end p6 protocol-error"))
    );
}

#[test]
fn an_unanswered_request_is_a_protocol_error_and_its_high_bits_lead_into_the_line_reset() {
    let mut waveform = waveform();
    waveform.connect();
    let position = waveform.position().get();
    let unanswered = SwdPacket::read(false, 0x0, 0).with_ack(Ack::Other(7));
    waveform.packet(&unanswered);
    waveform.line_reset(50);
    let idle = waveform.position().get();
    waveform.idle_cycles(2);
    waveform.idle(4);
    let block = waveform.finish();
    let mut expected = connect_lines(LEAD);
    expected.extend(packet_lines(1, &unanswered, position));
    // Park, the turnaround and the three acknowledge bits were high already.
    expected.push(line_reset(position + 7 * PERIOD, idle));
    expected.push(format!("eof {}", block.end()));
    assert_eq!(decode(&block), expected);
}

#[test]
fn line_resets_and_both_select_sequences_are_reported() {
    let mut waveform = waveform();
    waveform.line_reset(60);
    let select = waveform.position().get();
    waveform.select(Interface::Jtag);
    let jtag = waveform.position().get();
    // JTAG traffic, then a line reset that does not select SWD: no packet is decoded.
    waveform.bits(0b0011_0110_0001, 12);
    waveform.idle_cycles(1);
    let reset = waveform.position().get();
    waveform.line_reset(52);
    let idle = waveform.position().get();
    waveform.idle_cycles(3);
    waveform.packet(&SwdPacket::read(false, 0x0, 7));
    waveform.idle_cycles(1);
    let back = waveform.position().get();
    waveform.line_reset(51);
    let select_swd = waveform.position().get();
    waveform.select(Interface::Swd);
    let second = waveform.position().get();
    waveform.line_reset(50);
    let after = waveform.position().get();
    waveform.idle_cycles(2);
    let position = waveform.position().get();
    let packet = SwdPacket::read(false, 0x0, 0x0bb1_1477);
    waveform.packet(&packet);
    waveform.idle(4);
    let block = waveform.finish();
    let mut expected = vec![
        line_reset(LEAD, select),
        format!(
            "switch jtag [{},{})@{}",
            rise_at(select),
            rise_at(jtag - PERIOD) + 1,
            rise_at(jtag - PERIOD)
        ),
        line_reset(reset, idle),
        line_reset(back, select_swd),
        format!(
            "switch swd [{},{})@{}",
            rise_at(select_swd),
            rise_at(second - PERIOD) + 1,
            rise_at(second - PERIOD)
        ),
        line_reset(second, after),
    ];
    expected.extend(packet_lines(1, &packet, position));
    expected.push(format!("eof {}", block.end()));
    assert_eq!(decode(&block), expected);
}

#[test]
fn the_dormant_state_is_entered_and_left_as_adiv6_has_it() {
    let mut waveform = waveform();
    waveform.connect();
    let mut expected = connect_lines(LEAD);
    // Into the dormant state: a line reset and 0xE3BC. Nothing is decoded there.
    let reset = waveform.position().get();
    waveform.line_reset(52);
    let select = waveform.position().get();
    waveform.select(Interface::Dormant);
    expected.push(line_reset(reset, select));
    expected.push(switch_line("dormant", select, 16));
    waveform.idle_cycles(3);
    waveform.packet(&SwdPacket::read(false, 0x0, 1));
    // A wake with a code the decoder does not know leaves the port dormant.
    waveform.wake(0x5a, 8);
    waveform.packet(&SwdPacket::read(false, 0x0, 1));
    // The Arm CoreSight SW-DP code wakes the port to SWD, where the target waits for a line
    // reset in the protocol error state: until it comes, nothing is decoded (this packet's
    // last bits are low, so that they do not lead into the line reset).
    let alert = waveform.position().get() + 8 * PERIOD;
    waveform.wake(u16::from(ACTIVATE_SWD), 8);
    expected.push(switch_line("swd", alert, wake_bits(8)));
    waveform.packet(&SwdPacket::read(false, 0x0, 0));
    let reset = waveform.position().get();
    waveform.line_reset(56);
    let idle = waveform.position().get();
    waveform.idle_cycles(2);
    expected.push(line_reset(reset, idle));
    let position = waveform.position().get();
    let packet = SwdPacket::read(false, 0x0, 0x0bc1_2477);
    waveform.packet(&packet);
    expected.extend(packet_lines(1, &packet, position));
    waveform.idle_cycles(2);
    // Dormant again, then woken to JTAG by the JTAG-DP code: no packet is decoded until a
    // line reset and a switch back to SWD.
    let reset = waveform.position().get();
    waveform.line_reset(50);
    let select = waveform.position().get();
    waveform.select(Interface::Dormant);
    expected.push(line_reset(reset, select));
    expected.push(switch_line("dormant", select, 16));
    let alert = waveform.position().get() + 8 * PERIOD;
    waveform.wake(u16::from(ACTIVATE_JTAG_DP), 8);
    expected.push(switch_line("jtag", alert, wake_bits(8)));
    waveform.bits(0x0123_4567, 32);
    waveform.packet(&packet);
    let reset = waveform.position().get();
    waveform.line_reset(50);
    let select = waveform.position().get();
    waveform.select(Interface::Swd);
    let second = waveform.position().get();
    waveform.line_reset(50);
    let idle = waveform.position().get();
    waveform.idle_cycles(2);
    expected.push(line_reset(reset, select));
    expected.push(switch_line("swd", select, 16));
    expected.push(line_reset(second, idle));
    let position = waveform.position().get();
    waveform.packet(&packet);
    expected.extend(packet_lines(2, &packet, position));
    // The JTAG-Serial code, twelve zero bits, wakes to JTAG as well.
    let reset = waveform.position().get();
    waveform.line_reset(50);
    let select = waveform.position().get();
    waveform.select(Interface::Dormant);
    expected.push(line_reset(reset, select));
    expected.push(switch_line("dormant", select, 16));
    let alert = waveform.position().get() + 8 * PERIOD;
    waveform.wake(0, 12);
    expected.push(switch_line("jtag", alert, wake_bits(12)));
    waveform.idle(4);
    let block = waveform.finish();
    expected.push(format!("eof {}", block.end()));
    assert_eq!(decode(&block), expected);
}

#[test]
fn a_selection_alert_is_recognised_in_swd_mode_as_well() {
    // A host that takes the port for dormant while the decoder saw it in SWD: the alert's
    // bits are decoded for what they look like, but the alert and its code are still a
    // switch, every packet begun ends, and the line reset after it brings packets back.
    let mut waveform = waveform();
    waveform.connect();
    let alert = waveform.position().get() + 8 * PERIOD;
    waveform.wake(u16::from(ACTIVATE_SWD), 8);
    let reset = waveform.position().get();
    waveform.line_reset(56);
    let idle = waveform.position().get();
    waveform.idle_cycles(2);
    let position = waveform.position().get();
    let packet = SwdPacket::read(false, 0x0, 0x0bc1_2477);
    waveform.packet(&packet);
    waveform.idle(4);
    let block = waveform.finish();
    let lines = decode(&block);
    let switch = switch_line("swd", alert, wake_bits(8));
    let at = lines
        .iter()
        .position(|line| *line == switch)
        .unwrap_or_else(|| panic!("no {switch} in {lines:?}"));
    let count = |what: &str| lines[..at].iter().filter(|l| l.starts_with(what)).count();
    assert_eq!(count("begin "), count("end "), "{lines:?}");
    let mut tail = vec![line_reset(reset, idle)];
    tail.extend(packet_lines(count("begin ") as u64 + 1, &packet, position));
    tail.push(format!("eof {}", block.end()));
    assert_eq!(lines[at + 1..], tail[..]);
}

#[test]
fn a_packet_is_reported_once_it_cannot_be_a_select_sequence() {
    // After a line reset and one idle cycle, an access port read of 0x4 sends 1, 1, 1, 1, 0:
    // the JTAG-to-SWD sequence starts 0, 1, 1, 1, 1, 0 too, and parts from the request only at
    // its parity bit.
    let mut waveform = waveform();
    waveform.line_reset(50);
    let idle = waveform.position().get();
    waveform.idle_cycles(1);
    let position = waveform.position().get();
    let packet = SwdPacket::read(true, 0x4, 0x0000_1234);
    waveform.packet(&packet);
    waveform.idle(4);
    let block = waveform.finish();
    let mut expected = vec![line_reset(LEAD, idle)];
    let mut lines = packet_lines(1, &packet, position);
    let parity = position + 5 * PERIOD + HALF;
    lines[0] = format!(
        "begin p1 [{},{})@{parity}",
        rise_at(position),
        rise_at(position) + 1
    );
    expected.extend(lines);
    expected.push(format!("eof {}", block.end()));
    assert_eq!(decode(&block), expected);
}

#[test]
fn nothing_is_decoded_before_the_first_line_reset() {
    let mut waveform = waveform();
    waveform.connect();
    let packets = [
        SwdPacket::read(false, 0x0, 0x2ba0_1477),
        SwdPacket::write(false, 0x4, 0x5000_0000),
        SwdPacket::read(false, 0x4, 0xf000_0000),
    ];
    for packet in &packets {
        waveform.packet(packet);
        waveform.idle_cycles(2);
    }
    let reset = waveform.position().get();
    waveform.line_reset(64);
    let idle = waveform.position().get();
    waveform.idle_cycles(4);
    let position = waveform.position().get();
    waveform.packet(&packets[0]);
    waveform.idle(4);
    let whole = waveform.finish();
    // Start the samples in the middle of the first packet's data.
    let from = Tick(LEAD + (56 + 16 + 56 + 2 + 20) * PERIOD + 1);
    let block = whole
        .slice(TickRange::new(from, whole.end()))
        .expect("a slice");
    let mut expected = vec![line_reset(reset, idle)];
    expected.extend(packet_lines(1, &packets[0], position));
    expected.push(format!("eof {}", block.end()));
    assert_eq!(decode(&block), expected);
}

#[test]
fn the_end_of_the_samples_aborts_the_packet_under_way() {
    let mut waveform = waveform();
    waveform.connect();
    let position = waveform.position().get();
    let packet = SwdPacket::read(true, 0xc, 0x1122_3344);
    waveform.packet(&packet);
    let whole = waveform.finish();
    // Stop before the final turnaround rises: the data is whole, the packet is not.
    let cut = Tick(position + 45 * PERIOD + 1);
    let block = whole
        .slice(TickRange::new(Tick::ZERO, cut))
        .expect("a slice");
    let mut expected = connect_lines(LEAD);
    let lines = packet_lines(1, &packet, position);
    expected.extend(lines[..4].iter().cloned());
    expected.push(format!("end p1 aborted @{cut}"));
    expected.push(format!("eof {cut}"));
    assert_eq!(decode(&block), expected);
}

#[test]
fn a_gap_aborts_the_packet_and_the_decoder_waits_for_a_line_reset() {
    let mut waveform = waveform();
    waveform.connect();
    let first = waveform.position().get();
    let packets = [
        SwdPacket::read(false, 0x0, 0x2ba0_1477),
        SwdPacket::write(true, 0x4, 0x1234_5678),
        SwdPacket::read(true, 0xc, 0),
    ];
    for packet in &packets {
        waveform.packet(packet);
        waveform.idle_cycles(2);
    }
    let reset = waveform.position().get();
    waveform.line_reset(50);
    let idle = waveform.position().get();
    waveform.idle_cycles(2);
    let position = waveform.position().get();
    let last = SwdPacket::read(true, 0xc, 0xcafe_f00d);
    waveform.packet(&last);
    waveform.idle(4);
    let block = waveform.finish();
    // Lose the middle of the write's data.
    let second = first + 48 * PERIOD;
    let gap = TickRange::new(Tick(second + 20 * PERIOD + 3), Tick(second + 30 * PERIOD));
    let mut expected = connect_lines(LEAD);
    expected.extend(packet_lines(1, &packets[0], first));
    expected.extend(packet_lines(2, &packets[1], second)[..3].iter().cloned());
    expected.push(format!("end p2 aborted @{}", gap.start));
    expected.push(format!("gap {gap}"));
    expected.push(line_reset(reset, idle));
    expected.extend(packet_lines(3, &last, position));
    expected.push(format!("eof {}", block.end()));
    assert_eq!(decode_with_gap(&block, gap), expected);
}

#[test]
fn a_run_of_high_bits_too_short_for_a_line_reset_is_a_malformed_request() {
    let mut waveform = waveform();
    waveform.connect();
    let position = waveform.position().get();
    waveform.line_reset(20);
    let low = waveform.position().get();
    waveform.idle_cycles(2);
    waveform.idle(4);
    let block = waveform.finish();
    let mut expected = connect_lines(LEAD);
    // Decided by the low bit after the run: a request of all ones.
    let decided = rise_at(low);
    let rise = |index: u64| position + index * PERIOD + HALF;
    expected.push(format!("begin p1 [{},{})@{decided}", rise(0), rise(0) + 1));
    expected.push(format!(
        "request p1 ap read 0xc bad-parity [{},{})@{decided}",
        rise(0),
        rise(7) + 1
    ));
    expected.push(format!(
        "end p1 protocol-error [{},{})@{decided}",
        rise(7),
        rise(7) + 1
    ));
    expected.push(format!("eof {}", block.end()));
    assert_eq!(decode(&block), expected);
}

#[test]
fn a_line_reset_cut_short_counts_when_it_is_long_enough() {
    for (clocks, reported) in [(60u64, true), (49, false)] {
        let mut waveform = waveform();
        waveform.line_reset(clocks as u32);
        let block = waveform.finish();
        let end = block.end();
        let mut expected = Vec::new();
        if reported {
            expected.push(format!(
                "line-reset [{},{})@{end}",
                rise_at(LEAD),
                rise_at(LEAD + (clocks - 1) * PERIOD) + 1
            ));
        }
        expected.push(format!("eof {end}"));
        assert_eq!(decode(&block), expected, "{clocks} clocks");
    }
    // A gap after 55 high clocks.
    let mut waveform = waveform();
    waveform.line_reset(80);
    waveform.idle_cycles(2);
    let block = waveform.finish();
    let gap = TickRange::new(Tick(LEAD + 55 * PERIOD), Tick(LEAD + 60 * PERIOD));
    assert_eq!(
        decode_with_gap(&block, gap),
        vec![
            format!(
                "line-reset [{},{})@{}",
                rise_at(LEAD),
                rise_at(LEAD + 54 * PERIOD) + 1,
                gap.start
            ),
            format!("gap {gap}"),
            format!("eof {}", block.end()),
        ]
    );
}

#[test]
fn progress_is_held_back_to_what_is_not_reported_yet() {
    let mut waveform = waveform();
    waveform.connect();
    let position = waveform.position().get();
    waveform.packet(&SwdPacket::read(false, 0x0, 0x2ba0_1477));
    waveform.idle_cycles(2);
    let block = waveform.finish();
    let rise = |index: u64| position + index * PERIOD + HALF;
    let mut harness = harness(GraphConfig::default());
    let mut from = Tick::ZERO;
    for (last, held) in [
        // Inside the first line reset, a run of high bits from 6.
        (86, 6),
        // Inside the select sequence, which begins with the low bit at 230.
        (246, 230),
        // Inside the request, from its Start bit.
        (rise(3), rise(0)),
        // On the Park bit: Stop and Park, 0 then 1, are how a Selection Alert starts.
        (rise(7), rise(6)),
        // Inside the data.
        (rise(30), rise(12)),
        // Once the packet is complete, only the turnaround bit, low here, which could start a
        // Selection Alert.
        (rise(45), rise(45)),
    ] {
        let cut = Tick(last + 1);
        let piece = block.slice(TickRange::new(from, cut)).expect("a slice");
        harness.feed(&piece, &[u64::MAX]);
        harness.collect();
        assert_eq!(
            harness.items.last(),
            Some(&Item::Progress(Tick(held))),
            "up to {cut}"
        );
        from = cut;
    }
    harness.assert_progress_promise();
}

/// Everything at once: connection, all kinds of packets back to back and apart, errors and
/// the line resets after them, a switch to JTAG and back.
fn rich_waveform() -> LogicBlock {
    let mut waveform = waveform();
    waveform.connect();
    for n in 0..12u32 {
        let data = n.wrapping_mul(0x9e37_79b9);
        let addr = (n as u8 % 4) * 4;
        let ack = [Ack::Ok, Ack::Ok, Ack::Wait, Ack::Ok, Ack::Fault, Ack::Ok][n as usize % 6];
        waveform.packet(&SwdPacket::read(n % 3 == 0, addr, data).with_ack(ack));
        waveform.packet(&SwdPacket::write(n % 2 == 0, addr, !data).with_ack(ack));
        waveform.idle_cycles(n % 4);
    }
    waveform.packet(&SwdPacket {
        bad_data_parity: true,
        ..SwdPacket::read(true, 0x8, 0x0bad_0bad)
    });
    waveform.idle_cycles(1);
    waveform.line_reset(55);
    waveform.idle_cycles(2);
    waveform.packet(&SwdPacket::read(false, 0x0, 0x2ba0_1477).with_ack(Ack::Other(7)));
    waveform.line_reset(52);
    waveform.select(Interface::Jtag);
    waveform.bits(0x0123_4567_89ab_cdef, 64);
    waveform.line_reset(50);
    waveform.select(Interface::Swd);
    waveform.line_reset(50);
    waveform.idle_cycles(2);
    for n in 0..5u32 {
        waveform.packet(&SwdPacket::write(true, 0x4, n << 28));
    }
    waveform.connect_through_dormant();
    waveform.packet(&SwdPacket::read(false, 0x0, 0x0bc1_2477));
    waveform.packet(&SwdPacket::write(false, 0x4, 0x5000_0001));
    waveform.packet(&under_overrun(
        SwdPacket::read(true, 0xc, 0).with_ack(Ack::Wait),
    ));
    waveform.idle(6);
    waveform.finish()
}

#[test]
fn every_cut_and_pause_yields_the_same_events() {
    let block = rich_waveform();
    let len = block.len();
    let gaps = [
        TickRange::new(Tick(len / 4), Tick(len / 4 + 41)),
        TickRange::new(Tick(len / 3), Tick(len / 3 + 2)),
        TickRange::new(Tick(len * 9 / 10), Tick(len * 9 / 10 + 13)),
    ];
    let lines = describe(&items(&block, &[]));
    assert!(lines.len() > 150, "{lines:?}");
    for what in [
        "end p5 wait",
        "end p9 fault",
        "end p25 protocol-error",
        "ack p26 other(7)",
        "switch jtag",
        "switch swd",
        "end p31 complete",
        "switch dormant",
        "end p32 complete",
        "end p34 wait",
    ] {
        assert!(
            lines.iter().any(|line| line.starts_with(what)),
            "{what}: {lines:?}"
        );
    }
    // After each gap the decoder waits for a line reset, so less is decoded.
    let with_gaps = describe(&items(&block, &gaps));
    assert!(with_gaps.len() > 60, "{with_gaps:?}");
    assert_block_invariant(harness, &block, &gaps);
}

/// What a scenario puts on the wire.
#[derive(Clone, Debug)]
enum Step {
    /// A well-formed packet, then idle cycles.
    Packet(SwdPacket, u32),
    /// A broken packet, then a line reset and idle cycles, as a host recovering would send.
    Broken(SwdPacket, u32, u32),
    /// A line reset and idle cycles.
    Reset(u32, u32),
    /// A line reset, a select sequence, another line reset and idle cycles.
    Switch(Interface, u32, u32),
    /// A line reset, the SWD-to-dormant sequence, then a wake, then idle cycles; a wake to
    /// SWD is followed by a line reset first.
    Dormant(Wake, u32, u32),
}

/// What a wake from the dormant state asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Wake {
    Swd,
    Jtag,
    JtagSerial,
    Unrecognized,
}

impl Wake {
    /// The activation code and its length in bits.
    fn code(self) -> (u16, u32) {
        match self {
            Wake::Swd => (u16::from(ACTIVATE_SWD), 8),
            Wake::Jtag => (u16::from(ACTIVATE_JTAG_DP), 8),
            Wake::JtagSerial => (0, 12),
            Wake::Unrecognized => (0x5a, 8),
        }
    }
}

fn packet() -> impl Strategy<Value = SwdPacket> {
    (any::<bool>(), any::<bool>(), 0u8..4, any::<u32>(), 0u8..3).prop_map(
        |(apndp, rnw, addr, data, ack)| {
            let ack = [Ack::Ok, Ack::Wait, Ack::Fault][usize::from(ack)];
            SwdPacket {
                rnw,
                ..SwdPacket::read(apndp, addr * 4, data)
            }
            .with_ack(ack)
        },
    )
}

fn broken() -> impl Strategy<Value = SwdPacket> {
    (
        packet(),
        0u8..5,
        prop::sample::select(vec![0u8, 3, 5, 6, 7]),
    )
        .prop_map(|(packet, error, ack)| match error {
            0 => SwdPacket {
                bad_request_parity: true,
                ..packet.with_ack(Ack::Other(7))
            },
            1 => SwdPacket {
                stop: true,
                ..packet.with_ack(Ack::Other(7))
            },
            2 => SwdPacket {
                park: false,
                ..packet.with_ack(Ack::Other(7))
            },
            3 => packet.with_ack(Ack::Other(ack)),
            _ => SwdPacket {
                bad_data_parity: true,
                ..packet.with_ack(Ack::Ok)
            },
        })
}

fn step() -> impl Strategy<Value = Step> {
    prop_oneof![
        8 => (packet(), 0u32..4).prop_map(|(packet, idle)| Step::Packet(packet, idle)),
        2 => (broken(), 50u32..60, 3u32..6)
            .prop_map(|(packet, reset, idle)| Step::Broken(packet, reset, idle)),
        1 => (50u32..70, 3u32..8).prop_map(|(reset, idle)| Step::Reset(reset, idle)),
        1 => (
            prop::sample::select(vec![Interface::Swd, Interface::Jtag, Interface::Dormant]),
            50u32..60,
            3u32..6
        )
            .prop_map(|(to, reset, idle)| Step::Switch(to, reset, idle)),
        1 => (
            prop::sample::select(vec![
                Wake::Swd,
                Wake::Jtag,
                Wake::JtagSerial,
                Wake::Unrecognized
            ]),
            50u32..60,
            2u32..6
        )
            .prop_map(|(wake, reset, idle)| Step::Dormant(wake, reset, idle)),
    ]
}

fn build(steps: &[Step], delay: u64) -> LogicBlock {
    build_on(waveform().with_target_delay(delay), steps)
}

/// Puts `steps` on `waveform`.
fn build_on(mut waveform: SwdWaveform, steps: &[Step]) -> LogicBlock {
    for step in steps {
        match step {
            Step::Packet(packet, idle) => {
                waveform.packet(packet);
                waveform.idle_cycles(*idle);
            }
            Step::Broken(packet, reset, idle) => {
                waveform.packet(packet);
                waveform.line_reset(*reset);
                waveform.idle_cycles(*idle);
            }
            Step::Reset(reset, idle) => {
                waveform.line_reset(*reset);
                waveform.idle_cycles(*idle);
            }
            Step::Switch(to, reset, idle) => {
                waveform.line_reset(*reset);
                waveform.select(*to);
                waveform.line_reset(*reset);
                waveform.idle_cycles(*idle);
            }
            Step::Dormant(wake, reset, idle) => {
                waveform.line_reset(*reset);
                waveform.select(Interface::Dormant);
                let (code, bits) = wake.code();
                waveform.wake(code, bits);
                if *wake == Wake::Swd {
                    waveform.line_reset(*reset);
                }
                waveform.idle_cycles(*idle);
            }
        }
    }
    waveform.idle(4);
    waveform.finish()
}

/// The events a scenario must decode to, from the protocol's rules alone — packets count only
/// between a line reset and a protocol error, in SWD — and the steps with each packet told
/// whether overrun detection is on, as the debug port writes decoded so far have set it: a
/// write to CTRL/STAT acknowledged OK and whole, in bank 0 of SELECT.DPBANKSEL, which a line
/// reset puts back to 0.
fn simulate(steps: &[Step]) -> (Vec<SwdEvent>, Vec<Step>) {
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Mode {
        Unaligned,
        Swd,
        Jtag,
        Dormant,
    }
    let mut mode = Mode::Unaligned;
    let mut next = 1;
    let mut events = Vec::new();
    let mut annotated = Vec::with_capacity(steps.len());
    let (mut orundetect, mut dpbanksel) = (false, 0u32);
    let after_reset = |mode: Mode| match mode {
        Mode::Jtag | Mode::Dormant => mode,
        Mode::Unaligned | Mode::Swd => Mode::Swd,
    };
    for step in steps {
        let mut step = step.clone();
        match &mut step {
            Step::Packet(packet, _) | Step::Broken(packet, _, _) => {
                packet.orundetect = orundetect;
                let request_ok = !packet.bad_request_parity && !packet.stop && packet.park;
                if mode == Mode::Swd
                    && request_ok
                    && !packet.apndp
                    && !packet.rnw
                    && packet.ack == Ack::Ok
                    && !packet.bad_data_parity
                {
                    match packet.addr & 0xc {
                        0x8 => dpbanksel = packet.data & 0xf,
                        0x4 if dpbanksel == 0 => orundetect = packet.data & 1 != 0,
                        _ => {}
                    }
                }
                if mode == Mode::Swd {
                    let tx = TxId(next);
                    next += 1;
                    events.push(SwdEvent::Begin { tx });
                    events.push(SwdEvent::Request {
                        tx,
                        apndp: packet.apndp,
                        rnw: packet.rnw,
                        addr: packet.addr,
                        parity_ok: !packet.bad_request_parity,
                    });
                    let reason = if packet.bad_request_parity || packet.stop || !packet.park {
                        EndReason::ProtocolError
                    } else {
                        events.push(SwdEvent::Ack {
                            tx,
                            value: packet.ack,
                        });
                        match packet.ack {
                            _ if packet.ack == Ack::Ok || packet.is_targetsel() => {
                                events.push(SwdEvent::Data {
                                    tx,
                                    value: packet.data,
                                    parity_ok: !packet.bad_data_parity,
                                    from_target: packet.rnw,
                                });
                                if packet.bad_data_parity {
                                    EndReason::ProtocolError
                                } else {
                                    EndReason::Complete
                                }
                            }
                            Ack::Wait => EndReason::Wait,
                            Ack::Fault => EndReason::Fault,
                            Ack::Ok | Ack::Other(_) => EndReason::ProtocolError,
                        }
                    };
                    events.push(SwdEvent::End { tx, reason });
                    if reason == EndReason::ProtocolError {
                        mode = Mode::Unaligned;
                    }
                }
                if let Step::Broken(..) = step {
                    events.push(SwdEvent::LineReset);
                    dpbanksel = 0;
                    mode = after_reset(mode);
                }
            }
            Step::Reset(..) => {
                events.push(SwdEvent::LineReset);
                dpbanksel = 0;
                mode = after_reset(mode);
            }
            Step::Switch(to, ..) => {
                events.push(SwdEvent::LineReset);
                events.push(SwdEvent::Switch { to: *to });
                events.push(SwdEvent::LineReset);
                dpbanksel = 0;
                mode = match to {
                    Interface::Swd => Mode::Swd,
                    Interface::Jtag => Mode::Jtag,
                    Interface::Dormant => Mode::Dormant,
                };
            }
            Step::Dormant(wake, ..) => {
                events.push(SwdEvent::LineReset);
                events.push(SwdEvent::Switch {
                    to: Interface::Dormant,
                });
                dpbanksel = 0;
                mode = match wake {
                    Wake::Swd => {
                        events.push(SwdEvent::Switch { to: Interface::Swd });
                        events.push(SwdEvent::LineReset);
                        Mode::Swd
                    }
                    Wake::Jtag | Wake::JtagSerial => {
                        events.push(SwdEvent::Switch {
                            to: Interface::Jtag,
                        });
                        Mode::Jtag
                    }
                    Wake::Unrecognized => Mode::Dormant,
                };
            }
        }
        annotated.push(step);
    }
    (events, annotated)
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 96, ..ProptestConfig::default() })]

    /// Random traffic decodes to what the protocol's rules say, in any blocks.
    #[test]
    fn random_traffic_decodes_as_the_rules_say(
        steps in prop::collection::vec(step(), 1..24),
        sizes in prop::collection::vec(1u64..300, 1..5),
        delay in 0..HALF,
    ) {
        let (expected, steps) = simulate(&steps);
        let block = build(&steps, delay);
        let decoded = items(&block, &[]);
        prop_assert_eq!(payloads(&decoded), expected);
        let mut cut = harness(GraphConfig { queue_capacity: 2, budget: 5 });
        cut.drain_on_full = 1;
        cut.feed_cut(&block, &[], &sizes, &[1, 0, 2]);
        cut.end();
        cut.assert_progress_promise();
        prop_assert_eq!(cut.without_progress(), decoded);
    }
}

/// What the test reader makes of an SWD stream.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Seen {
    LineReset,
    Switch(Interface),
    /// A packet: the fields read and how the reading ended.
    Packet {
        request: Option<Request>,
        ack: Option<Ack>,
        data: Option<Data>,
        end: String,
    },
}

impl EventSchema for Seen {
    const SCHEMA: SchemaId = SchemaId::new("test.swd-reader", 1);
}

/// How the test reader reads a packet.
#[derive(Clone, Copy, Debug)]
enum Mode {
    /// Every field, then the end.
    All,
    /// The request alone; [`SwdBus::finish`] drains the rest.
    RequestOnly,
    /// Request, acknowledge and data with [`Packet::field`], which reports an end as an error.
    ///
    /// [`Packet::field`]: uiharu_protocols::swd::Packet::field
    Strict,
}

/// A decoder built on SWD that reads every packet through the cursor.
struct Reader {
    mode: Mode,
}

impl Node for Reader {
    fn spec(&self) -> NodeSpec {
        NodeSpec {
            name: "swd-reader".into(),
            inputs: vec![InputSpec::new::<SwdEvent>(
                "bus",
                Requires::new().equals("turnaround", 1),
            )],
            outputs: vec![OutputSpec::new::<Seen>("seen", Props::new())],
        }
    }

    fn start(self: Box<Self>, mut io: Io) -> NodeFuture {
        let out = io.output(0);
        Box::pin(read(
            SwdBus::new(io.input(0), out.progress_sink()),
            out,
            self.mode,
        ))
    }
}

async fn read(mut bus: SwdBus, mut out: Outbox<Seen>, mode: Mode) {
    loop {
        let seen = match bus.next().await {
            Next::Packet(mut packet) => {
                let (mut request, mut ack, mut data) = (None, None, None);
                let mut take = |field: Field| match field {
                    Field::Request(field) => request = Some(field),
                    Field::Ack(field) => ack = Some(field),
                    Field::Data(field) => data = Some(field),
                };
                let end = match mode {
                    Mode::All => loop {
                        match packet.field_or_end().await {
                            Ok(Read::Field(field)) => take(field),
                            Ok(Read::End(reason)) => break format!("{reason:?}"),
                            Err(error) => break format!("{error:?}"),
                        }
                    },
                    Mode::RequestOnly => match packet.field().await {
                        Ok(field) => {
                            take(field);
                            "stopped".to_owned()
                        }
                        Err(error) => format!("{error:?}"),
                    },
                    Mode::Strict => {
                        let mut end = "read three".to_owned();
                        for _ in 0..3 {
                            match packet.field().await {
                                Ok(field) => take(field),
                                Err(error) => {
                                    end = format!("{error:?}");
                                    break;
                                }
                            }
                        }
                        end
                    }
                };
                Seen::Packet {
                    request,
                    ack,
                    data,
                    end,
                }
            }
            Next::LineReset => Seen::LineReset,
            Next::Switch(to) => Seen::Switch(to),
            Next::Gap(range) => {
                out.gap(range).await;
                continue;
            }
            Next::Progress(tick) => {
                out.progress(tick).await;
                continue;
            }
            Next::End(tick) => {
                out.end(tick).await;
                return;
            }
        };
        let seen = match (bus.finish().await, seen) {
            (
                Err(error),
                Seen::Packet {
                    request,
                    ack,
                    data,
                    end,
                },
            ) => Seen::Packet {
                request,
                ack,
                data,
                end: format!("{end}, then {error:?}"),
            },
            (_, seen) => seen,
        };
        out.emit(bus.consumed(), seen).await;
    }
}

fn reader_harness(mode: Mode, graph: GraphConfig) -> Harness<Seen> {
    Harness::build(
        CHANNELS,
        TimeBase::new(1_000_000),
        graph,
        |graph, source| {
            let swd = graph.add(Swd::new(SwdConfig::default()).expect("a valid configuration"));
            graph.connect(source.samples, Swd::input(swd))?;
            let reader = graph.add(Reader { mode });
            graph.connect(Swd::events(swd), InputRef::of_node(reader, 0))?;
            Ok(OutputRef::of_node(reader, 0))
        },
    )
    .expect("builds")
}

fn request_of(packet: &SwdPacket) -> Request {
    Request {
        apndp: packet.apndp,
        rnw: packet.rnw,
        addr: packet.addr,
        parity_ok: !packet.bad_request_parity,
    }
}

fn data_of(packet: &SwdPacket) -> Data {
    Data {
        value: packet.data,
        parity_ok: !packet.bad_data_parity,
        from_target: packet.rnw,
    }
}

#[test]
fn a_decoder_built_on_swd_reads_packets_field_by_field() {
    let packets = [
        SwdPacket::read(false, 0x0, 0x2ba0_1477),
        SwdPacket::write(false, 0x8, 0x0000_00f0),
        SwdPacket::read(true, 0xc, 0).with_ack(Ack::Wait),
        SwdPacket::write(false, 0x4, 0x5000_0000).with_ack(Ack::Fault),
        SwdPacket {
            bad_data_parity: true,
            ..SwdPacket::read(true, 0x8, 0x0bad_c0de)
        },
    ];
    let last = SwdPacket::read(false, 0x4, 0xf000_0000);
    let mut waveform = waveform();
    waveform.connect();
    let first = waveform.position().get();
    for packet in &packets {
        waveform.packet(packet);
        waveform.idle_cycles(1);
    }
    waveform.line_reset(50);
    waveform.idle_cycles(2);
    waveform.packet(&last);
    waveform.idle(4);
    let block = waveform.finish();

    let whole = |packet: &SwdPacket, end: &str| Seen::Packet {
        request: Some(request_of(packet)),
        ack: Some(packet.ack),
        data: (packet.ack == Ack::Ok).then(|| data_of(packet)),
        end: end.to_owned(),
    };
    let connect = [
        Seen::LineReset,
        Seen::Switch(Interface::Swd),
        Seen::LineReset,
    ];
    let expect = |ends: [&str; 6]| {
        let mut expected = connect.to_vec();
        for (packet, end) in packets.iter().zip(ends) {
            expected.push(whole(packet, end));
        }
        expected.push(Seen::LineReset);
        expected.push(whole(&last, ends[5]));
        expected
    };
    let seen = |mode: Mode| {
        let mut harness = reader_harness(mode, GraphConfig::default());
        harness.run_all(&block);
        harness.events()
    };

    let events = seen(Mode::All);
    let payloads: Vec<Seen> = events.iter().map(|event| event.payload.clone()).collect();
    assert_eq!(
        payloads,
        expect([
            "Complete",
            "Complete",
            "Wait",
            "Fault",
            "ProtocolError",
            "Complete"
        ])
    );
    // The first packet covers its Start bit through its final turnaround.
    let start = rise_at(first);
    assert_eq!(
        (events[3].span, events[3].settled),
        (
            TickRange::new(Tick(start), Tick(start + 45 * PERIOD + 1)),
            Tick(start + 45 * PERIOD)
        )
    );

    // `field` reports the end where a field was expected; draining finds it otherwise.
    let payloads: Vec<Seen> = seen(Mode::Strict)
        .into_iter()
        .map(|event| event.payload)
        .collect();
    assert_eq!(
        payloads,
        expect([
            "read three",
            "read three",
            "Ended(Wait)",
            "Ended(Fault)",
            "read three",
            "read three"
        ])
    );

    // A reader that takes the request alone leaves the rest to `finish`.
    let payloads: Vec<Seen> = seen(Mode::RequestOnly)
        .into_iter()
        .map(|event| event.payload)
        .collect();
    let mut expected = connect.to_vec();
    let request_only = |packet: &SwdPacket| Seen::Packet {
        request: Some(request_of(packet)),
        ack: None,
        data: None,
        end: "stopped".to_owned(),
    };
    expected.extend(packets.iter().map(request_only));
    expected.push(Seen::LineReset);
    expected.push(request_only(&last));
    assert_eq!(payloads, expected);
}

#[test]
fn a_reader_learns_that_a_packet_was_cut_off() {
    let mut waveform = waveform();
    waveform.connect();
    let first = waveform.position().get();
    let packet = SwdPacket::write(true, 0x4, 0x1234_5678);
    waveform.packet(&packet);
    waveform.idle_cycles(2);
    waveform.packet(&SwdPacket::read(false, 0x0, 1));
    waveform.idle(4);
    let block = waveform.finish();
    // Lose the middle of the write's data.
    let gap = TickRange::new(Tick(first + 20 * PERIOD + 3), Tick(first + 30 * PERIOD));
    let mut harness = reader_harness(Mode::All, GraphConfig::default());
    harness.feed_with_gaps(&block, &[gap], &[71]);
    harness.end();
    let items = harness.without_progress();
    let cut = Seen::Packet {
        request: Some(request_of(&packet)),
        ack: Some(Ack::Ok),
        data: None,
        end: "Aborted".to_owned(),
    };
    let payloads: Vec<Option<&Seen>> = items
        .iter()
        .map(|item| item.event().map(|event| &event.payload))
        .collect();
    assert_eq!(
        payloads,
        vec![
            Some(&Seen::LineReset),
            Some(&Seen::Switch(Interface::Swd)),
            Some(&Seen::LineReset),
            Some(&cut),
            None,
            None,
        ]
    );
    assert_eq!(items[4], Item::Gap(gap));
    assert!(items[5].is_end());
}

#[test]
fn the_reader_sees_the_same_whatever_the_blocks() {
    let block = rich_waveform();
    let len = block.len();
    let gaps = [
        TickRange::new(Tick(len / 4), Tick(len / 4 + 41)),
        TickRange::new(Tick(len * 9 / 10), Tick(len * 9 / 10 + 13)),
    ];
    for mode in [Mode::All, Mode::RequestOnly, Mode::Strict] {
        assert_block_invariant(|graph| reader_harness(mode, graph), &block, &gaps);
    }
}
