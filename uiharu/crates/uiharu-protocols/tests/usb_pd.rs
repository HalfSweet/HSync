// SPDX-License-Identifier: Apache-2.0

//! The USB Power Delivery decoder on synthesized BMC waveforms, and a decoder built on it
//! reading packets through the bus cursor.

use std::collections::BTreeSet;

use proptest::prelude::*;
use serde::Serialize;
use uiharu_core::testing::{self, Harness, assert_block_invariant};
use uiharu_core::{
    EventSchema, GraphConfig, GraphError, InputRef, InputSpec, Io, Item, LogicBlock, Node,
    NodeFuture, NodeId, NodeSpec, Outbox, OutputRef, OutputSpec, Props, Requires, SchemaId,
    SignalItem, Tick, TickRange, TimeBase,
};
use uiharu_protocols::usb_pd::synth::{
    Faults, Header, UsbPdWaveform, battery_pdo, crc32, extended_header, fixed_pdo, object_bytes,
    pps_apdo, request, variable_pdo,
};
use uiharu_protocols::usb_pd::{
    DATA_BATCH, EndReason, ErrorReason, ExtendedMessageHeader, Field, KCode, MessageHeader, Next,
    OrderedSet, PacketId, PowerObject, Read, ReadError, RequestObject, UsbPd, UsbPdBus,
    UsbPdConfig, UsbPdEvent,
};

const CHANNELS: u8 = 2;
/// 12 MHz: 40 ticks per bit at 300 kbit/s.
const RATE: u64 = 12_000_000;
/// 2.4 MHz: 8 ticks per bit.
const SLOW: u64 = 2_400_000;
/// Ticks per bit at [`RATE`].
const BIT: u64 = 40;

fn config() -> UsbPdConfig {
    UsbPdConfig {
        cc: 1,
        ..UsbPdConfig::default()
    }
}

/// Source → USB PD.
fn harness(config: &UsbPdConfig, rate: u64, graph: GraphConfig) -> Harness<UsbPdEvent> {
    let config = config.clone();
    Harness::build(
        CHANNELS,
        TimeBase::new(rate),
        graph,
        move |graph, source| {
            let pd = graph.add(UsbPd::new(config).expect("a valid configuration"));
            graph.connect(source.samples, UsbPd::input(pd))?;
            Ok(UsbPd::events(pd))
        },
    )
    .expect("builds")
}

fn decode_with(config: &UsbPdConfig, rate: u64, block: &LogicBlock) -> Vec<String> {
    let mut harness = harness(config, rate, GraphConfig::default());
    harness.run_all(block);
    assert_well_formed(&harness.items);
    describe(&harness.without_progress())
}

fn decode(block: &LogicBlock) -> Vec<String> {
    decode_with(&config(), RATE, block)
}

/// A waveform at `rate` whose line has been idle for 20 bits.
fn waveform(rate: u64) -> UsbPdWaveform {
    let mut waveform = UsbPdWaveform::new(&config(), CHANNELS, rate, Tick::ZERO);
    waveform.idle_bits(20);
    waveform
}

/// The name serde gives a value.
fn name<T: Serialize>(value: &T) -> String {
    serde_json::to_value(value)
        .expect("serializes")
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| "?".to_owned())
}

fn hex(bytes: &[u8]) -> String {
    if bytes.is_empty() {
        return "-".to_owned();
    }
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn corrupt(corrupt: &Option<u8>) -> String {
    corrupt.map_or(String::new(), |position| format!(" corrupt {position}"))
}

fn what(event: &UsbPdEvent) -> String {
    match event {
        UsbPdEvent::Begin { pkt } => format!("begin p{}", pkt.0),
        UsbPdEvent::Sop {
            pkt,
            set,
            corrupt: c,
        } => {
            format!("sop p{} {}{}", pkt.0, name(set), corrupt(c))
        }
        UsbPdEvent::Header {
            pkt,
            header:
                MessageHeader {
                    message_type,
                    control,
                    data,
                    extended_message,
                    data_role,
                    power_role,
                    cable_plug,
                    revision,
                    message_id,
                    objects,
                    extended,
                },
        } => {
            let named = control
                .map(|message| name(&message))
                .or(data.map(|message| name(&message)))
                .or(extended_message.map(|message| name(&message)))
                .unwrap_or_else(|| format!("type {message_type}"));
            let roles = match (data_role, power_role, cable_plug) {
                (Some(data), Some(power), None) => format!("{} {}", name(data), name(power)),
                (None, None, Some(true)) => "plug".to_owned(),
                (None, None, Some(false)) => "port".to_owned(),
                other => format!("{other:?}"),
            };
            let extended = if *extended { " extended" } else { "" };
            format!(
                "header p{} {named} objects {objects} id {message_id} rev {revision} {roles}{extended}",
                pkt.0
            )
        }
        UsbPdEvent::ExtendedHeader {
            pkt,
            header:
                ExtendedMessageHeader {
                    data_size,
                    request_chunk,
                    chunk_number,
                    chunked,
                },
        } => format!(
            "extended-header p{} size {data_size} chunk {chunk_number}{}{}",
            pkt.0,
            if *chunked { " chunked" } else { "" },
            if *request_chunk { " request" } else { "" }
        ),
        UsbPdEvent::Object { pkt, index, raw } => format!("object p{} #{index} {raw:08x}", pkt.0),
        UsbPdEvent::Pdo { pkt, index, object } => {
            let fields = match object {
                PowerObject::Fixed {
                    millivolts,
                    milliamps,
                    flags,
                } => format!("fixed {millivolts}mV {milliamps}mA flags {flags:03x}"),
                PowerObject::Battery {
                    max_millivolts,
                    min_millivolts,
                    milliwatts,
                } => format!("battery {min_millivolts}-{max_millivolts}mV {milliwatts}mW"),
                PowerObject::Variable {
                    max_millivolts,
                    min_millivolts,
                    milliamps,
                } => format!("variable {min_millivolts}-{max_millivolts}mV {milliamps}mA"),
                PowerObject::Augmented {
                    apdo_type,
                    max_millivolts,
                    min_millivolts,
                    milliamps,
                } => format!(
                    "augmented {apdo_type} {min_millivolts:?}-{max_millivolts:?}mV {milliamps:?}mA"
                ),
            };
            format!("pdo p{} #{index} {fields}", pkt.0)
        }
        UsbPdEvent::Rdo {
            pkt,
            request:
                RequestObject {
                    position,
                    give_back,
                    capability_mismatch,
                    usb_communications_capable,
                    no_usb_suspend,
                    unchunked_extended,
                    operating,
                    maximum,
                },
        } => {
            let flags: String = [
                (*give_back, " give-back"),
                (*capability_mismatch, " mismatch"),
                (*usb_communications_capable, " usb"),
                (*no_usb_suspend, " no-suspend"),
                (*unchunked_extended, " unchunked"),
            ]
            .iter()
            .filter(|(set, _)| *set)
            .map(|(_, flag)| *flag)
            .collect();
            format!(
                "rdo p{} position {position} operating {operating} maximum {maximum}{flags}",
                pkt.0
            )
        }
        UsbPdEvent::Data { pkt, offset, bytes } => {
            format!("data p{} +{offset} {}", pkt.0, hex(bytes))
        }
        UsbPdEvent::Crc { pkt, value, ok } => format!(
            "crc p{} {value:08x}{}",
            pkt.0,
            if *ok { "" } else { " bad" }
        ),
        UsbPdEvent::HardReset { pkt, corrupt: c } => format!("hard-reset p{}{}", pkt.0, corrupt(c)),
        UsbPdEvent::CableReset { pkt, corrupt: c } => {
            format!("cable-reset p{}{}", pkt.0, corrupt(c))
        }
        UsbPdEvent::Error { pkt, reason } => format!("error p{} {}", pkt.0, name(reason)),
        UsbPdEvent::End { pkt, reason } => format!("end p{} {}", pkt.0, name(reason)),
    }
}

fn at(span: TickRange, settled: Tick) -> String {
    if span.is_empty() {
        format!("@{settled}")
    } else {
        format!("[{},{})@{settled}", span.start, span.end)
    }
}

/// One line per item, with where an event comes from.
fn describe(items: &[Item<UsbPdEvent>]) -> Vec<String> {
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

/// Checks the envelope: events settle in order, every packet that begins ends once, nothing
/// comes outside a packet, batches are bounded and progress is never passed.
fn assert_well_formed(items: &[Item<UsbPdEvent>]) {
    let mut settled = Tick::ZERO;
    let mut progress = Tick::ZERO;
    let mut open = BTreeSet::new();
    for item in items {
        let event = match item {
            Item::Event(event) => event,
            Item::Progress(tick) => {
                progress = progress.max(*tick);
                continue;
            }
            Item::Gap(range) => {
                assert!(
                    range.start >= progress,
                    "gap {range} after progress {progress}"
                );
                continue;
            }
            Item::End(tick) => {
                assert!(*tick >= progress, "end {tick} after progress {progress}");
                continue;
            }
        };
        assert!(event.settled >= settled, "settled out of order: {event:?}");
        assert!(
            event.span.start >= progress,
            "{event:?} after progress {progress}"
        );
        settled = event.settled;
        let pkt = event.payload.pkt();
        match &event.payload {
            UsbPdEvent::Begin { .. } => assert!(open.insert(pkt), "{pkt:?} began twice"),
            UsbPdEvent::End { .. } => assert!(open.remove(&pkt), "{pkt:?} ended unopened"),
            UsbPdEvent::Data { bytes, .. } => {
                assert!(open.contains(&pkt), "{event:?} outside its packet");
                assert!(!bytes.is_empty() && bytes.len() <= DATA_BATCH, "{event:?}");
            }
            _ => assert!(open.contains(&pkt), "{event:?} outside its packet"),
        }
    }
    assert!(open.is_empty(), "packets left open: {open:?}");
}

/// Source_Capabilities from a DFP source.
fn source_capabilities(id: u8, objects: u8) -> u16 {
    Header {
        message_type: 1,
        data_role: true,
        power_role: true,
        revision: 2,
        message_id: id,
        objects,
        extended: false,
    }
    .value()
}

/// A control message of `message_type` from a UFP sink.
fn control(message_type: u8, id: u8) -> u16 {
    Header {
        message_type,
        revision: 2,
        message_id: id,
        ..Header::default()
    }
    .value()
}

#[test]
fn source_capabilities_and_good_crc_are_placed_on_their_bits() {
    let pdos = [
        fixed_pdo(5000, 3000, 0b10_0110_0000),
        fixed_pdo(9000, 2000, 0),
    ];
    let mut waveform = waveform(RATE);
    let first = waveform.position();
    waveform.message(
        OrderedSet::Sop,
        source_capabilities(0, 2),
        &object_bytes(&pdos),
    );
    waveform.idle_bits(20);
    let second = waveform.position();
    waveform.message(OrderedSet::Sop, control(1, 0), &[]);
    waveform.idle_bits(20);
    let block = waveform.finish();
    // The preamble's 64 bits, then the ordered set's 20, the header's 20, 40 per object, the
    // CRC's 40 and the EOP's 5, 40 ticks each.
    let s = |bits: u64| first.offset(BIT * bits);
    let g = |bits: u64| second.offset(BIT * bits);
    let mut bytes = source_capabilities(0, 2).to_le_bytes().to_vec();
    bytes.extend(object_bytes(&pdos));
    let caps_crc = crc32(&bytes);
    let good_crc = crc32(&control(1, 0).to_le_bytes());
    assert_eq!(
        decode(&block),
        vec![
            format!("begin p1 [{},{})@{}", s(0), s(64), s(64)),
            format!("sop p1 sop [{},{})@{}", s(64), s(84), s(84)),
            format!(
                "header p1 source-capabilities objects 2 id 0 rev 2 dfp source [{},{})@{}",
                s(84),
                s(104),
                s(104)
            ),
            format!(
                "object p1 #1 {:08x} [{},{})@{}",
                pdos[0],
                s(104),
                s(144),
                s(144)
            ),
            format!(
                "pdo p1 #1 fixed 5000mV 3000mA flags 260 [{},{})@{}",
                s(104),
                s(144),
                s(144)
            ),
            format!(
                "object p1 #2 {:08x} [{},{})@{}",
                pdos[1],
                s(144),
                s(184),
                s(184)
            ),
            format!(
                "pdo p1 #2 fixed 9000mV 2000mA flags 000 [{},{})@{}",
                s(144),
                s(184),
                s(184)
            ),
            format!("crc p1 {caps_crc:08x} [{},{})@{}", s(184), s(224), s(224)),
            format!("end p1 eop [{},{})@{}", s(224), s(229), s(229)),
            format!("begin p2 [{},{})@{}", g(0), g(64), g(64)),
            format!("sop p2 sop [{},{})@{}", g(64), g(84), g(84)),
            format!(
                "header p2 good-crc objects 0 id 0 rev 2 ufp sink [{},{})@{}",
                g(84),
                g(104),
                g(104)
            ),
            format!("crc p2 {good_crc:08x} [{},{})@{}", g(104), g(144), g(144)),
            format!("end p2 eop [{},{})@{}", g(144), g(149), g(149)),
            format!("eof {}", block.end()),
        ]
    );
}

/// A Request from a UFP sink.
fn request_header(id: u8) -> u16 {
    Header {
        message_type: 2,
        revision: 2,
        message_id: id,
        objects: 1,
        ..Header::default()
    }
    .value()
}

/// The CRC of a message: its header and the bytes after it.
fn crc_of(header: u16, payload: &[u8]) -> u32 {
    crc32(&[&header.to_le_bytes()[..], payload].concat())
}

#[test]
fn a_request_carries_its_request_data_object() {
    let mut waveform = waveform(RATE);
    let rdo = request(2, 1500, 3000, 0b00110);
    waveform.message(OrderedSet::Sop, request_header(1), &object_bytes(&[rdo]));
    waveform.idle_bits(20);
    // Every flag — GiveBack, Capability Mismatch, USB Communications Capable, No USB Suspend,
    // Unchunked Extended Messages Supported — and a position past 7, as EPR has them.
    let all = request(9, 100, 50, 0b11111);
    waveform.message(OrderedSet::Sop, request_header(2), &object_bytes(&[all]));
    waveform.idle_bits(20);
    let block = waveform.finish();
    assert_eq!(
        bare(&decode(&block)),
        vec![
            "begin p1".to_owned(),
            "sop p1 sop".to_owned(),
            "header p1 request objects 1 id 1 rev 2 ufp sink".to_owned(),
            format!("object p1 #1 {rdo:08x}"),
            "rdo p1 position 2 operating 150 maximum 300 usb no-suspend".to_owned(),
            format!(
                "crc p1 {:08x}",
                crc_of(request_header(1), &object_bytes(&[rdo]))
            ),
            "end p1 eop".to_owned(),
            "begin p2".to_owned(),
            "sop p2 sop".to_owned(),
            "header p2 request objects 1 id 2 rev 2 ufp sink".to_owned(),
            format!("object p2 #1 {all:08x}"),
            "rdo p2 position 9 operating 10 maximum 5 give-back mismatch usb no-suspend unchunked"
                .to_owned(),
            format!(
                "crc p2 {:08x}",
                crc_of(request_header(2), &object_bytes(&[all]))
            ),
            "end p2 eop".to_owned(),
            format!("eof {}", block.end()),
        ]
    );
}

#[test]
fn every_control_message_is_named_by_its_type() {
    let mut waveform = waveform(SLOW);
    for message_type in 0..32u8 {
        waveform.message(
            OrderedSet::Sop,
            control(message_type, message_type % 8),
            &[],
        );
        waveform.idle_bits(10);
    }
    let block = waveform.finish();
    let lines = bare(&decode_with(&config(), SLOW, &block));
    let names = [
        "good-crc",
        "goto-min",
        "accept",
        "reject",
        "ping",
        "ps-rdy",
        "get-source-cap",
        "get-sink-cap",
        "dr-swap",
        "pr-swap",
        "vconn-swap",
        "wait",
        "soft-reset",
        "data-reset",
        "data-reset-complete",
        "not-supported",
        "get-source-cap-extended",
        "get-status",
        "fr-swap",
        "get-pps-status",
        "get-country-codes",
        "get-sink-cap-extended",
        "get-source-info",
        "get-revision",
    ];
    let headers: Vec<&String> = lines
        .iter()
        .filter(|line| line.starts_with("header"))
        .collect();
    assert_eq!(headers.len(), 32);
    for (message_type, header) in headers.iter().enumerate() {
        let named = match message_type {
            1..=24 => names[message_type - 1].to_owned(),
            _ => format!("type {message_type}"),
        };
        assert_eq!(
            **header,
            format!(
                "header p{} {named} objects 0 id {} rev 2 ufp sink",
                message_type + 1,
                message_type % 8
            )
        );
    }
    assert!(
        !lines
            .iter()
            .any(|line| line.contains("bad") || line.contains("error"))
    );
}

#[test]
fn data_messages_are_named_and_only_capabilities_and_requests_read_further() {
    let mut waveform = waveform(RATE);
    let object = fixed_pdo(5000, 900, 0);
    for message_type in 1..16u8 {
        let header = Header {
            message_type,
            revision: 2,
            objects: 1,
            ..Header::default()
        };
        waveform.message(OrderedSet::Sop, header.value(), &object_bytes(&[object]));
        waveform.idle_bits(10);
    }
    let block = waveform.finish();
    let lines = bare(&decode(&block));
    let names = [
        "source-capabilities",
        "request",
        "bist",
        "sink-capabilities",
        "battery-status",
        "alert",
        "get-country-info",
        "enter-usb",
        "epr-request",
        "epr-mode",
        "source-info",
        "revision",
        "type 13",
        "type 14",
        "vendor-defined",
    ];
    let headers: Vec<String> = lines
        .iter()
        .filter(|line| line.starts_with("header"))
        .map(|line| line.split(" objects").next().unwrap_or_default().to_owned())
        .collect();
    let expected: Vec<String> = names
        .iter()
        .enumerate()
        .map(|(n, name)| format!("header p{} {name}", n + 1))
        .collect();
    assert_eq!(headers, expected);
    let companions: Vec<&String> = lines
        .iter()
        .filter(|line| line.starts_with("pdo") || line.starts_with("rdo"))
        .collect();
    assert_eq!(
        companions,
        vec![
            "pdo p1 #1 fixed 5000mV 900mA flags 000",
            "rdo p2 position 0 operating 100 maximum 90",
            "pdo p4 #1 fixed 5000mV 900mA flags 000",
        ]
    );
    assert_eq!(
        lines
            .iter()
            .filter(|line| line.starts_with("object"))
            .count(),
        15
    );
}

#[test]
fn power_data_objects_of_every_type_are_read() {
    let pdos = [
        fixed_pdo(5000, 3000, 0b10_0110_0000),
        variable_pdo(21_000, 3_300, 2_000),
        battery_pdo(20_000, 5_000, 60_000),
        pps_apdo(21_000, 3_300, 3_000),
        (3 << 30) | (1 << 28) | 0x0012_3456,
    ];
    let mut waveform = waveform(RATE);
    waveform.message(
        OrderedSet::Sop,
        source_capabilities(4, 5),
        &object_bytes(&pdos),
    );
    waveform.idle_bits(20);
    let sink = Header {
        message_type: 4,
        revision: 2,
        message_id: 5,
        objects: 1,
        ..Header::default()
    };
    waveform.message(
        OrderedSet::Sop,
        sink.value(),
        &object_bytes(&[fixed_pdo(5000, 500, 0)]),
    );
    waveform.idle_bits(20);
    let block = waveform.finish();
    let lines = bare(&decode(&block));
    let pdos: Vec<&String> = lines
        .iter()
        .filter(|line| line.starts_with("pdo"))
        .collect();
    assert_eq!(
        pdos,
        vec![
            "pdo p1 #1 fixed 5000mV 3000mA flags 260",
            "pdo p1 #2 variable 3300-21000mV 2000mA",
            "pdo p1 #3 battery 5000-20000mV 60000mW",
            "pdo p1 #4 augmented 0 Some(3300)-Some(21000)mV Some(3000)mA",
            "pdo p1 #5 augmented 1 None-NonemV NonemA",
            "pdo p2 #1 fixed 5000mV 500mA flags 000",
        ]
    );
}

#[test]
fn messages_to_cable_plugs_say_who_sent_them() {
    let discover = 0xff00_8001u32;
    let mut waveform = waveform(RATE);
    for (set, plug) in [
        (OrderedSet::SopPrime, false),
        (OrderedSet::SopPrime, true),
        (OrderedSet::SopDoublePrime, false),
        (OrderedSet::SopDoublePrime, true),
        (OrderedSet::SopPrimeDebug, true),
        (OrderedSet::SopDoublePrimeDebug, false),
    ] {
        let header = Header {
            message_type: 15,
            data_role: true,
            power_role: plug,
            revision: 2,
            objects: 1,
            ..Header::default()
        };
        waveform.message(set, header.value(), &object_bytes(&[discover]));
        waveform.idle_bits(12);
    }
    let block = waveform.finish();
    let lines = bare(&decode(&block));
    let sets: Vec<&String> = lines
        .iter()
        .filter(|line| line.starts_with("sop"))
        .collect();
    assert_eq!(
        sets,
        vec![
            "sop p1 sop-prime",
            "sop p2 sop-prime",
            "sop p3 sop-double-prime",
            "sop p4 sop-double-prime",
            "sop p5 sop-prime-debug",
            "sop p6 sop-double-prime-debug",
        ]
    );
    // Bit 5, the data role of SOP messages, is reserved in these and not reported.
    let roles: Vec<&str> = lines
        .iter()
        .filter(|line| line.starts_with("header"))
        .map(|line| line.rsplit(' ').next().unwrap_or_default())
        .collect();
    assert_eq!(roles, ["port", "plug", "port", "plug", "plug", "port"]);
    assert!(
        lines
            .iter()
            .any(|line| line == &format!("object p1 #1 {discover:08x}"))
    );
    assert!(!lines.iter().any(|line| line.contains("bad")));
}

/// An extended message header from a DFP source.
fn extended(message_type: u8, id: u8, objects: u8) -> u16 {
    Header {
        message_type,
        data_role: true,
        power_role: true,
        revision: 2,
        message_id: id,
        objects,
        extended: true,
    }
    .value()
}

/// A Source_Capabilities_Extended chunk of a 30-byte message: its extended header and data.
fn chunk_of(chunk_number: u8, data: &[u8]) -> Vec<u8> {
    let mut bytes = extended_header(30, chunk_number, false, true)
        .to_le_bytes()
        .to_vec();
    bytes.extend(data);
    bytes
}

#[test]
fn an_extended_message_comes_in_chunks() {
    // Source_Capabilities_Extended: 30 bytes in two chunks of 26 and 4, the second one asked
    // for by the sink.
    let data: Vec<u8> = (1..=30).collect();
    let mut waveform = waveform(RATE);
    let first = waveform.position();
    let chunk = chunk_of(0, &data[..26]);
    waveform.message(OrderedSet::Sop, extended(1, 0, 7), &chunk);
    waveform.idle_bits(20);
    let asking = Header {
        message_type: 1,
        revision: 2,
        objects: 1,
        extended: true,
        ..Header::default()
    };
    let request = extended_header(0, 1, true, true).to_le_bytes();
    waveform.message(
        OrderedSet::Sop,
        asking.value(),
        &[request[0], request[1], 0, 0],
    );
    waveform.idle_bits(20);
    let mut last = chunk_of(1, &data[26..]);
    last.extend([0, 0]);
    waveform.message(OrderedSet::Sop, extended(1, 1, 2), &last);
    waveform.idle_bits(20);
    let block = waveform.finish();
    let lines = decode(&block);
    let s = |bits: u64| first.offset(BIT * bits);
    // After the header's 20 bits, the extended header's 20, then 26 bytes of 10 bits each.
    assert_eq!(
        lines[..7],
        vec![
            format!("begin p1 [{},{})@{}", s(0), s(64), s(64)),
            format!("sop p1 sop [{},{})@{}", s(64), s(84), s(84)),
            format!(
                "header p1 source-capabilities-extended objects 7 id 0 rev 2 dfp source extended \
                 [{},{})@{}",
                s(84),
                s(104),
                s(104)
            ),
            format!(
                "extended-header p1 size 30 chunk 0 chunked [{},{})@{}",
                s(104),
                s(124),
                s(124)
            ),
            format!(
                "data p1 +0 {} [{},{})@{}",
                hex(&data[..26]),
                s(124),
                s(384),
                s(384)
            ),
            format!(
                "crc p1 {:08x} [{},{})@{}",
                crc_of(extended(1, 0, 7), &chunk),
                s(384),
                s(424),
                s(424)
            ),
            format!("end p1 eop [{},{})@{}", s(424), s(429), s(429)),
        ]
    );
    // The request for chunk 1 carries two bytes of padding; the last chunk four bytes of data
    // and two of padding.
    assert_eq!(
        bare(&lines[7..]),
        vec![
            "begin p2".to_owned(),
            "sop p2 sop".to_owned(),
            "header p2 source-capabilities-extended objects 1 id 0 rev 2 ufp sink extended"
                .to_owned(),
            "extended-header p2 size 0 chunk 1 chunked request".to_owned(),
            "data p2 +0 0000".to_owned(),
            format!(
                "crc p2 {:08x}",
                crc_of(asking.value(), &[request[0], request[1], 0, 0])
            ),
            "end p2 eop".to_owned(),
            "begin p3".to_owned(),
            "sop p3 sop".to_owned(),
            "header p3 source-capabilities-extended objects 2 id 1 rev 2 dfp source extended"
                .to_owned(),
            "extended-header p3 size 30 chunk 1 chunked".to_owned(),
            format!("data p3 +0 {}0000", hex(&data[26..])),
            format!("crc p3 {:08x}", crc_of(extended(1, 1, 2), &last)),
            "end p3 eop".to_owned(),
            format!("eof {}", block.end()),
        ]
    );
}

#[test]
fn an_unchunked_extended_message_runs_to_its_eop() {
    // 40 bytes in one message, as Status: each byte goes out once four more have followed it,
    // the last four before the EOP being the CRC.
    let data: Vec<u8> = (0..40u8).map(|i| i.wrapping_mul(37)).collect();
    let mut payload = extended_header(40, 0, false, false).to_le_bytes().to_vec();
    payload.extend(&data);
    let mut waveform = waveform(RATE);
    let first = waveform.position();
    waveform.message(OrderedSet::Sop, extended(2, 3, 0), &payload);
    waveform.idle_bits(20);
    let block = waveform.finish();
    let s = |bits: u64| first.offset(BIT * bits);
    // Byte n spans bits 124 + 10 n to 134 + 10 n; the CRC is bytes 40 to 43, the EOP bits 564
    // to 568.
    assert_eq!(
        decode(&block)[2..],
        vec![
            format!(
                "header p1 status objects 0 id 3 rev 2 dfp source extended [{},{})@{}",
                s(84),
                s(104),
                s(104)
            ),
            format!(
                "extended-header p1 size 40 chunk 0 [{},{})@{}",
                s(104),
                s(124),
                s(124)
            ),
            format!(
                "data p1 +0 {} [{},{})@{}",
                hex(&data[..32]),
                s(124),
                s(444),
                s(484)
            ),
            format!(
                "data p1 +32 {} [{},{})@{}",
                hex(&data[32..]),
                s(444),
                s(524),
                s(569)
            ),
            format!(
                "crc p1 {:08x} [{},{})@{}",
                crc_of(extended(2, 3, 0), &payload),
                s(524),
                s(564),
                s(569)
            ),
            format!("end p1 eop [{},{})@{}", s(564), s(569), s(569)),
            format!("eof {}", block.end()),
        ]
    );
}

#[test]
fn a_damaged_crc_is_reported_and_the_packet_still_ends_at_its_eop() {
    let mut waveform = waveform(RATE);
    let faults = Faults {
        crc_xor: 0x8000_0001,
        ..Faults::default()
    };
    waveform.message_with(OrderedSet::Sop, control(3, 2), &[], &faults);
    waveform.idle_bits(20);
    waveform.message(OrderedSet::Sop, control(1, 2), &[]);
    waveform.idle_bits(20);
    let block = waveform.finish();
    let lines = bare(&decode(&block));
    let sent = crc_of(control(3, 2), &[]) ^ 0x8000_0001;
    assert_eq!(
        lines[3..5],
        [format!("crc p1 {sent:08x} bad"), "end p1 eop".to_owned()]
    );
    assert_eq!(
        lines[8],
        format!("crc p2 {:08x}", crc_of(control(1, 2), &[]))
    );
}

#[test]
fn a_reserved_symbol_ends_the_packet() {
    let mut waveform = waveform(RATE);
    let first = waveform.position();
    // The header's second symbol, symbol 5 counted from the ordered set's first, becomes 00000.
    let faults = Faults {
        symbol: Some((5, 0b00000)),
        ..Faults::default()
    };
    waveform.message_with(OrderedSet::Sop, control(3, 0), &[], &faults);
    waveform.idle_bits(20);
    waveform.message(OrderedSet::Sop, control(1, 0), &[]);
    waveform.idle_bits(20);
    let block = waveform.finish();
    let s = |bits: u64| first.offset(BIT * bits);
    let lines = decode(&block);
    assert_eq!(
        lines[..4],
        vec![
            format!("begin p1 [{},{})@{}", s(0), s(64), s(64)),
            format!("sop p1 sop [{},{})@{}", s(64), s(84), s(84)),
            format!("error p1 symbol [{},{})@{}", s(89), s(94), s(94)),
            format!("end p1 error [{},{})@{}", s(89), s(94), s(94)),
        ]
    );
    // The rest of the packet is passed over; the next one decodes.
    assert_eq!(
        bare(&lines[4..]),
        vec![
            "begin p2".to_owned(),
            "sop p2 sop".to_owned(),
            "header p2 good-crc objects 0 id 0 rev 2 ufp sink".to_owned(),
            format!("crc p2 {:08x}", crc_of(control(1, 0), &[])),
            "end p2 eop".to_owned(),
            format!("eof {}", block.end()),
        ]
    );
}

/// The lines of one message sent as `faults` say, then a good one.
fn damaged(header: u16, payload: &[u8], faults: Faults) -> Vec<String> {
    let mut waveform = waveform(RATE);
    waveform.message_with(OrderedSet::Sop, header, payload, &faults);
    waveform.idle_bits(20);
    waveform.message(OrderedSet::Sop, control(1, 0), &[]);
    waveform.idle_bits(20);
    bare(&decode(&waveform.finish()))
}

/// Faults that send `code` as the symbol with this index.
fn symbol(index: usize, code: u8) -> Faults {
    Faults {
        symbol: Some((index, code)),
        ..Faults::default()
    }
}

#[test]
fn symbols_out_of_place_end_the_packet() {
    // Sync-1 among the header's symbols.
    let lines = damaged(control(3, 0), &[], symbol(6, KCode::Sync1.code()));
    assert_eq!(lines[2..5], ["error p1 symbol", "end p1 error", "begin p2"]);
    // An EOP in the third symbol of the CRC: too early.
    let lines = damaged(control(3, 0), &[], symbol(10, KCode::Eop.code()));
    assert_eq!(
        lines[3..6],
        ["error p1 early-eop", "end p1 error", "begin p2"]
    );
    // A data symbol where the EOP belongs.
    let lines = damaged(control(3, 0), &[], symbol(16, 0b11110));
    assert_eq!(
        lines[4..7],
        ["error p1 missing-eop", "end p1 error", "begin p2"]
    );
    // A reserved code there.
    let lines = damaged(control(3, 0), &[], symbol(16, 0b11111));
    assert_eq!(lines[4..7], ["error p1 symbol", "end p1 error", "begin p2"]);
    // A header that announces two objects where one was sent: the EOP comes early.
    let one = object_bytes(&[fixed_pdo(5000, 100, 0)]);
    let lines = damaged(source_capabilities(0, 2), &one, Faults::default());
    assert_eq!(lines[3], "object p1 #1 0001900a");
    assert_eq!(
        lines[5],
        format!(
            "object p1 #2 {:08x}",
            crc_of(source_capabilities(0, 2), &one)
        )
    );
    assert_eq!(
        lines[7..10],
        ["error p1 early-eop", "end p1 error", "begin p2"]
    );
    // One announced, two sent: the second is read as the CRC, and no EOP follows.
    let two = object_bytes(&[fixed_pdo(5000, 100, 0), fixed_pdo(9000, 100, 0)]);
    let lines = damaged(source_capabilities(0, 1), &two, Faults::default());
    assert_eq!(lines[5], "crc p1 0002d00a bad");
    assert_eq!(
        lines[6..9],
        ["error p1 missing-eop", "end p1 error", "begin p2"]
    );
}

#[test]
fn an_ordered_set_needs_three_k_codes_in_place() {
    // SOP with its third K-code corrupted is still SOP.
    let lines = damaged(control(3, 0), &[], symbol(2, 0b00000));
    assert_eq!(lines[1], "sop p1 sop corrupt 3");
    assert_eq!(lines[2], "header p1 accept objects 0 id 0 rev 2 ufp sink");
    assert_eq!(lines[4], "end p1 eop");
    // Sync-1 Sync-1 Sync-1 Sync-3 is one K-code from SOP and one from SOP': no set.
    let lines = damaged(control(3, 0), &[], symbol(3, KCode::Sync3.code()));
    assert_eq!(
        lines[1..4],
        ["error p1 ordered-set", "end p1 error", "begin p2"]
    );
    // Data symbols make no set.
    let lines = damaged(control(3, 0), &[], symbol(1, 0b11110));
    assert_eq!(lines[1], "sop p1 sop corrupt 2");
    let mut waveform = waveform(RATE);
    waveform.frame(&[0b11110; 6], &Faults::default());
    waveform.idle_bits(20);
    assert_eq!(
        bare(&decode(&waveform.finish()))[..3],
        ["begin p1", "error p1 ordered-set", "end p1 error"]
    );
}

#[test]
fn hard_and_cable_resets_end_after_their_ordered_set() {
    let mut waveform = waveform(RATE);
    let first = waveform.position();
    waveform.hard_reset();
    waveform.idle_bits(20);
    let second = waveform.position();
    waveform.cable_reset();
    waveform.idle_bits(20);
    // RST-1 corrupted in second place.
    let codes = [KCode::Rst1, KCode::Rst1, KCode::Rst1, KCode::Rst2].map(KCode::code);
    waveform.frame(&codes, &symbol(1, 0b10101));
    waveform.idle_bits(20);
    let block = waveform.finish();
    let s = |bits: u64| first.offset(BIT * bits);
    let c = |bits: u64| second.offset(BIT * bits);
    let lines = decode(&block);
    assert_eq!(
        lines[..6],
        vec![
            format!("begin p1 [{},{})@{}", s(0), s(64), s(64)),
            format!("hard-reset p1 [{},{})@{}", s(64), s(84), s(84)),
            format!("end p1 hard-reset @{}", s(84)),
            format!("begin p2 [{},{})@{}", c(0), c(64), c(64)),
            format!("cable-reset p2 [{},{})@{}", c(64), c(84), c(84)),
            format!("end p2 cable-reset @{}", c(84)),
        ]
    );
    assert_eq!(
        bare(&lines[6..]),
        vec![
            "begin p3".to_owned(),
            "hard-reset p3 corrupt 2".to_owned(),
            "end p3 hard-reset".to_owned(),
            format!("eof {}", block.end()),
        ]
    );
}

#[test]
fn a_bit_out_of_time_ends_the_packet() {
    // Accept's header: nibbles 3, 8, 0, 0. Bit 89 is the first bit of the symbol of 8, 10010
    // (sent 0, 1, 0, 0, 1): a 0; bit 90 a 1. 40 ticks a bit; a middle transition is accepted
    // 15 to 25 ticks into its bit, the bit's end 35 to 45 ticks into it.
    let cases = [
        // A 0 lasting 52 ticks: nothing by 46 ticks.
        (89, 130, 46),
        // A 0 lasting 28 ticks: an end too early to be one, a middle too late to be one.
        (89, 70, 28),
        // A 1 lasting 52 ticks: its middle at 26 ticks.
        (90, 130, 26),
        // A 1 lasting 28 ticks: its middle at 14 ticks.
        (90, 70, 14),
    ];
    for (bit, percent, offset) in cases {
        let faults = Faults {
            stretch: Some((bit, percent)),
            ..Faults::default()
        };
        let mut waveform = waveform(RATE);
        let first = waveform.position();
        waveform.message_with(OrderedSet::Sop, control(3, 0), &[], &faults);
        waveform.idle_bits(20);
        waveform.message(OrderedSet::Sop, control(1, 0), &[]);
        waveform.idle_bits(20);
        let lines = decode(&waveform.finish());
        let start = first.offset(BIT * bit as u64);
        let at = start.offset(offset);
        assert_eq!(
            lines[2..4],
            [
                format!("error p1 timing [{start},{at})@{at}"),
                format!("end p1 error [{start},{at})@{at}"),
            ],
            "bit {bit} at {percent} %"
        );
        assert_eq!(bare(&lines[4..6]), ["begin p2", "sop p2 sop"]);
    }
    // A bit 8 % long, its middle 4 % late, is within the tolerance.
    let lines = damaged(
        control(3, 0),
        &[],
        Faults {
            stretch: Some((90, 108)),
            ..Faults::default()
        },
    );
    assert_eq!(lines[4], "end p1 eop");
}

#[test]
fn a_packet_the_transmitter_cuts_off_ends_in_a_timing_error() {
    // Cut before bit 100, and before the final transition: the bit before either never ends,
    // so 46 ticks pass with no transition.
    for cut in [100, 149] {
        let mut waveform = waveform(RATE);
        let first = waveform.position();
        let faults = Faults {
            cut: Some(cut),
            ..Faults::default()
        };
        waveform.message_with(OrderedSet::Sop, control(3, 0), &[], &faults);
        waveform.stay_bits(5);
        waveform.idle_bits(20);
        waveform.message(OrderedSet::Sop, control(1, 0), &[]);
        waveform.idle_bits(20);
        let lines = decode(&waveform.finish());
        let start = first.offset(BIT * (cut as u64 - 1));
        let at = start.offset(46);
        let error = lines
            .iter()
            .position(|line| line.starts_with("error"))
            .expect("an error");
        assert_eq!(
            lines[error..error + 2],
            [
                format!("error p1 timing [{start},{at})@{at}"),
                format!("end p1 error [{start},{at})@{at}"),
            ],
            "cut before bit {cut}"
        );
        assert_eq!(
            bare(&lines[error + 2..error + 4]),
            ["begin p2", "sop p2 sop"]
        );
    }
}

#[test]
fn a_preamble_at_the_wrong_bit_rate_ends_in_a_timing_error() {
    for (bit_rate, period) in [(250_000u64, 48u64), (345_000, 0)] {
        let mut waveform =
            UsbPdWaveform::new(&config(), CHANNELS, RATE, Tick::ZERO).with_bit_rate(bit_rate);
        waveform.idle_bits(20);
        let first = waveform.position();
        waveform.message(OrderedSet::Sop, control(3, 0), &[]);
        waveform.idle_bits(20);
        let block = waveform.finish();
        let lines = decode(&block);
        assert_eq!(
            bare(&lines),
            [
                "begin p1".to_owned(),
                "error p1 timing".to_owned(),
                "end p1 error".to_owned(),
                format!("eof {}", block.end()),
            ],
            "{bit_rate} bit/s"
        );
        if period > 0 {
            // The error covers the preamble: 64 bits of 48 ticks.
            let end = first.offset(64 * period);
            assert_eq!(lines[1], format!("error p1 timing [{first},{end})@{end}"));
        }
    }
    // Far off, no preamble is recognised at all.
    for bit_rate in [200_000, 500_000] {
        let mut waveform =
            UsbPdWaveform::new(&config(), CHANNELS, RATE, Tick::ZERO).with_bit_rate(bit_rate);
        waveform.idle_bits(20);
        waveform.message(OrderedSet::Sop, control(3, 0), &[]);
        waveform.idle_bits(20);
        let block = waveform.finish();
        assert_eq!(
            decode(&block),
            [format!("eof {}", block.end())],
            "{bit_rate} bit/s"
        );
    }
}

/// A conversation: capabilities, a request, accept, PS_RDY, each with its GoodCRC.
fn conversation(waveform: &mut UsbPdWaveform) {
    let pdos = object_bytes(&[
        fixed_pdo(5000, 3000, 0b10_0110_0000),
        pps_apdo(21_000, 3_300, 3_000),
    ]);
    waveform.message(OrderedSet::Sop, source_capabilities(0, 2), &pdos);
    waveform.idle_bits(10);
    waveform.message(OrderedSet::Sop, control(1, 0), &[]);
    waveform.idle_bits(30);
    let rdo = object_bytes(&[request(1, 3000, 3000, 0b00110)]);
    waveform.message(OrderedSet::Sop, request_header(0), &rdo);
    waveform.idle_bits(10);
    waveform.message(OrderedSet::Sop, source_capabilities(0, 0) & !0x1f | 1, &[]);
    waveform.idle_bits(30);
    waveform.message(OrderedSet::Sop, source_capabilities(1, 0) & !0x1f | 3, &[]);
    waveform.idle_bits(10);
    waveform.message(OrderedSet::Sop, control(1, 1), &[]);
    waveform.idle_bits(30);
    waveform.message(OrderedSet::Sop, source_capabilities(2, 0) & !0x1f | 6, &[]);
    waveform.idle_bits(10);
    waveform.message(OrderedSet::Sop, control(1, 2), &[]);
    waveform.idle_bits(20);
}

/// What [`conversation`] decodes to.
fn conversation_lines() -> Vec<&'static str> {
    vec![
        "header p1 source-capabilities objects 2 id 0 rev 2 dfp source",
        "header p2 good-crc objects 0 id 0 rev 2 ufp sink",
        "header p3 request objects 1 id 0 rev 2 ufp sink",
        "header p4 good-crc objects 0 id 0 rev 2 dfp source",
        "header p5 accept objects 0 id 1 rev 2 dfp source",
        "header p6 good-crc objects 0 id 1 rev 2 ufp sink",
        "header p7 ps-rdy objects 0 id 2 rev 2 dfp source",
        "header p8 good-crc objects 0 id 2 rev 2 ufp sink",
    ]
}

#[test]
fn transmitters_ten_percent_off_decode_at_forty_and_eight_ticks_per_bit() {
    for rate in [RATE, SLOW, 1_800_000] {
        for bit_rate in [270_000, 285_000, 300_000, 316_000, 330_000] {
            for idle_high in [false, true] {
                let mut waveform = UsbPdWaveform::new(&config(), CHANNELS, rate, Tick::ZERO)
                    .with_bit_rate(bit_rate);
                if idle_high {
                    waveform = waveform.with_idle_high();
                }
                waveform.idle_bits(20);
                conversation(&mut waveform);
                let lines = bare(&decode_with(&config(), rate, &waveform.finish()));
                let headers: Vec<&str> = lines
                    .iter()
                    .filter(|line| line.starts_with("header"))
                    .map(String::as_str)
                    .collect();
                let what = format!("{bit_rate} bit/s at {rate} Hz, idle high {idle_high}");
                assert_eq!(headers, conversation_lines(), "{what}");
                assert_eq!(
                    lines.iter().filter(|line| line.ends_with(" eop")).count(),
                    8,
                    "{what}: {lines:?}"
                );
                assert!(
                    !lines
                        .iter()
                        .any(|line| line.contains("bad") || line.contains("error")),
                    "{what}: {lines:?}"
                );
            }
        }
    }
}

#[test]
fn the_line_released_after_a_packet_starts_nothing() {
    // The line idles high: after each packet it is held low, longer than idle, then released
    // high — an edge on a quiet line that no preamble follows.
    let mut waveform = UsbPdWaveform::new(&config(), CHANNELS, RATE, Tick::ZERO)
        .with_idle_high()
        .with_hold_low(3 * BIT);
    waveform.idle_bits(20);
    conversation(&mut waveform);
    let lines = bare(&decode(&waveform.finish()));
    let headers: Vec<&str> = lines
        .iter()
        .filter(|line| line.starts_with("header"))
        .map(String::as_str)
        .collect();
    assert_eq!(headers, conversation_lines());
    assert_eq!(lines.len(), 8 * 5 + 4 + 2 + 1);
}

#[test]
fn pulses_on_the_idle_line_start_no_packet_until_a_preamble_begins() {
    let mut waveform = waveform(RATE);
    // A one-tick glitch, a pulse as long as a bit, and a lone 0: no preamble.
    waveform.hold(true, 1);
    waveform.idle_bits(5);
    waveform.hold(true, BIT);
    waveform.idle_bits(5);
    waveform.bit(false);
    waveform.idle_bits(5);
    // A 0 then a 1 open a packet, even when nothing follows them.
    let first = waveform.position();
    waveform.bit(false);
    waveform.bit(true);
    waveform.idle_bits(5);
    waveform.message(OrderedSet::Sop, control(1, 0), &[]);
    waveform.idle_bits(20);
    let lines = decode(&waveform.finish());
    let (start, at) = (first.offset(2 * BIT), first.offset(2 * BIT + 46));
    assert_eq!(
        lines[..3],
        [
            format!("begin p1 [{first},{start})@{start}"),
            format!("error p1 timing [{start},{at})@{at}"),
            format!("end p1 error [{start},{at})@{at}"),
        ]
    );
    assert_eq!(
        bare(&lines[3..6]),
        [
            "begin p2",
            "sop p2 sop",
            "header p2 good-crc objects 0 id 0 rev 2 ufp sink"
        ]
    );
}

#[test]
fn a_packet_cut_off_by_the_end_of_the_samples_is_aborted() {
    let pdos = object_bytes(&[fixed_pdo(5000, 3000, 0), fixed_pdo(9000, 2000, 0)]);
    let mut waveform = waveform(RATE);
    let first = waveform.position();
    waveform.message(OrderedSet::Sop, source_capabilities(0, 2), &pdos);
    waveform.idle_bits(20);
    let block = waveform.finish();
    let s = |bits: u64| first.offset(BIT * bits);
    let upto = |end: Tick| {
        decode(
            &block
                .slice(TickRange::new(Tick::ZERO, end))
                .expect("a slice"),
        )
    };
    // In the second object's third symbol: the first object is whole, the second dropped.
    let cut = s(144 + 12).offset(7);
    assert_eq!(
        bare(&upto(cut)),
        vec![
            "begin p1".to_owned(),
            "sop p1 sop".to_owned(),
            "header p1 source-capabilities objects 2 id 0 rev 2 dfp source".to_owned(),
            format!("object p1 #1 {:08x}", fixed_pdo(5000, 3000, 0)),
            "pdo p1 #1 fixed 5000mV 3000mA flags 000".to_owned(),
            "end p1 aborted".to_owned(),
            format!("eof {cut}"),
        ]
    );
    assert_eq!(upto(cut)[5], format!("end p1 aborted @{cut}"));
    // In the preamble: `begin` covers what was read of it.
    let cut = s(30).offset(5);
    assert_eq!(
        upto(cut),
        vec![
            format!("begin p1 [{},{})@{}", s(0), s(30), s(30)),
            format!("end p1 aborted @{cut}"),
            format!("eof {cut}"),
        ]
    );
    // Before the preamble's first two bits are read, nothing is reported.
    let cut = s(1).offset(30);
    assert_eq!(upto(cut), vec![format!("eof {cut}")]);
}

#[test]
fn data_read_before_a_cut_is_reported() {
    let data: Vec<u8> = (1..=30).collect();
    let mut waveform = waveform(RATE);
    let first = waveform.position();
    waveform.message(
        OrderedSet::Sop,
        extended(1, 0, 7),
        &chunk_of(0, &data[..26]),
    );
    waveform.idle_bits(20);
    let block = waveform.finish();
    let s = |bits: u64| first.offset(BIT * bits);
    // In the fourth bit of byte 10: bytes 0 to 9 are whole.
    let cut = s(124 + 103).offset(9);
    let lines = decode(
        &block
            .slice(TickRange::new(Tick::ZERO, cut))
            .expect("a slice"),
    );
    assert_eq!(
        lines[4..],
        vec![
            format!(
                "data p1 +0 {} [{},{})@{cut}",
                hex(&data[..10]),
                s(124),
                s(224)
            ),
            format!("end p1 aborted @{cut}"),
            format!("eof {cut}"),
        ]
    );
}

#[test]
fn a_gap_aborts_the_packet_and_the_next_one_decodes() {
    let pdos = object_bytes(&[fixed_pdo(5000, 3000, 0), fixed_pdo(9000, 2000, 0)]);
    let mut waveform = waveform(RATE);
    let first = waveform.position();
    waveform.message(OrderedSet::Sop, source_capabilities(0, 2), &pdos);
    waveform.idle_bits(20);
    waveform.message(OrderedSet::Sop, control(1, 0), &[]);
    waveform.idle_bits(20);
    let block = waveform.finish();
    let start = first.offset(BIT * 120 + 13);
    let gap = TickRange::new(start, start.offset(100));
    let mut harness = harness(&config(), RATE, GraphConfig::default());
    harness.feed_with_gaps(&block, &[gap], &[777]);
    harness.end();
    assert_well_formed(&harness.items);
    let lines = describe(&harness.without_progress());
    assert_eq!(
        bare(&lines),
        vec![
            "begin p1".to_owned(),
            "sop p1 sop".to_owned(),
            "header p1 source-capabilities objects 2 id 0 rev 2 dfp source".to_owned(),
            "end p1 aborted".to_owned(),
            format!("gap {gap}"),
            "begin p2".to_owned(),
            "sop p2 sop".to_owned(),
            "header p2 good-crc objects 0 id 0 rev 2 ufp sink".to_owned(),
            format!("crc p2 {:08x}", crc_of(control(1, 0), &[])),
            "end p2 eop".to_owned(),
            format!("eof {}", block.end()),
        ]
    );
    assert_eq!(lines[3], format!("end p1 aborted @{start}"));
}

#[test]
fn a_packet_under_way_at_the_first_sample_is_skipped() {
    let mut waveform = waveform(RATE);
    let first = waveform.position();
    waveform.message(OrderedSet::Sop, control(3, 0), &[]);
    waveform.idle_bits(20);
    waveform.message(OrderedSet::Sop, control(1, 0), &[]);
    waveform.idle_bits(20);
    let block = waveform.finish();
    // In the header, in the preamble, at its first transition, and too little idle before it:
    // the line must be seen idle for 51 ticks before a preamble may start.
    for from in [
        first.offset(BIT * 90 + 3),
        first.offset(BIT * 10),
        first,
        Tick(first.get() - 50),
    ] {
        let tail = block
            .slice(TickRange::new(from, block.end()))
            .expect("a slice");
        assert_eq!(
            bare(&decode(&tail)),
            vec![
                "begin p1".to_owned(),
                "sop p1 sop".to_owned(),
                "header p1 good-crc objects 0 id 0 rev 2 ufp sink".to_owned(),
                format!("crc p1 {:08x}", crc_of(control(1, 0), &[])),
                "end p1 eop".to_owned(),
                format!("eof {}", block.end()),
            ],
            "from {from}"
        );
    }
    // Idle for 51 ticks is enough.
    let tail = block
        .slice(TickRange::new(Tick(first.get() - 51), block.end()))
        .expect("a slice");
    assert_eq!(
        bare(&decode(&tail))[2],
        "header p1 accept objects 0 id 0 rev 2 ufp sink"
    );
}

#[test]
fn fields_are_reported_as_soon_as_they_are_read() {
    let pdos = object_bytes(&[fixed_pdo(5000, 3000, 0), fixed_pdo(9000, 2000, 0)]);
    let mut waveform = waveform(RATE);
    let first = waveform.position();
    waveform.message(OrderedSet::Sop, source_capabilities(0, 2), &pdos);
    waveform.idle_bits(20);
    let block = waveform.finish();
    let mut harness = harness(&config(), RATE, GraphConfig::default());
    // Up to the transition that ends the first object.
    let end = first.offset(BIT * 144 + 1);
    let head = block
        .slice(TickRange::new(Tick::ZERO, end))
        .expect("a slice");
    harness.push(SignalItem::Data(std::sync::Arc::new(head)));
    harness.collect();
    let lines = bare(&describe(&harness.without_progress()));
    assert_eq!(
        lines[3..],
        [
            "object p1 #1 0001912c",
            "pdo p1 #1 fixed 5000mV 3000mA flags 000"
        ]
    );
}

#[test]
fn the_source_must_sample_six_times_per_bit() {
    let build = |rate: u64, cc: u8| {
        Harness::build(
            CHANNELS,
            TimeBase::new(rate),
            GraphConfig::default(),
            |graph, source| {
                let config = UsbPdConfig {
                    cc,
                    ..UsbPdConfig::default()
                };
                let pd = graph.add(UsbPd::new(config).expect("a valid configuration"));
                graph.connect(source.samples, UsbPd::input(pd))?;
                Ok(UsbPd::events(pd))
            },
        )
        .err()
    };
    assert_eq!(
        build(1_000_000, 1),
        Some(GraphError::Requirement {
            from: "logic#0.samples".into(),
            to: "usb-pd#0.logic".into(),
            key: "rateHz",
            requirement: ">= 1800000".into(),
            found: "1000000".into(),
        })
    );
    assert!(build(1_800_000, 1).is_none());
    assert!(matches!(
        build(RATE, 2),
        Some(GraphError::Requirement {
            key: "channels",
            ..
        })
    ));
}

#[test]
fn the_synthesized_crc_leaves_the_published_residual() {
    // The receiver's check as the specification describes it: a register preset to all ones,
    // shifted left, the polynomial added whenever its top bit differs from the bit coming in.
    fn register(bytes: &[u8]) -> u32 {
        let mut register = u32::MAX;
        for byte in bytes {
            for i in 0..8 {
                let top = register >> 31;
                register <<= 1;
                if u32::from(byte >> i & 1) != top {
                    register ^= 0x04c1_1db7;
                }
            }
        }
        register
    }
    let caps = object_bytes(&[fixed_pdo(5000, 3000, 0), fixed_pdo(9000, 2000, 0)]);
    let packets = [
        control(1, 0).to_le_bytes().to_vec(),
        [&source_capabilities(0, 2).to_le_bytes()[..], &caps].concat(),
        vec![0xff; 30],
        Vec::new(),
    ];
    for bytes in packets {
        let crc = crc32(&bytes);
        let mut sent = bytes.clone();
        sent.extend(crc.to_le_bytes());
        assert_eq!(register(&sent), 0xc704_dd7b, "{bytes:02x?}");
        sent[0] ^= 1;
        assert_ne!(register(&sent), 0xc704_dd7b, "{bytes:02x?}");
    }
    assert_eq!(crc32(b"123456789"), 0xcbf4_3926);
}

/// Traffic of every kind: a conversation, cable plugs, chunked and unchunked extended
/// messages, damaged packets and resets, at `rate`.
fn rich(rate: u64) -> LogicBlock {
    let mut waveform = UsbPdWaveform::new(&config(), CHANNELS, rate, Tick::ZERO);
    let w = &mut waveform;
    w.idle_bits(20);
    conversation(w);
    let vdm = object_bytes(&[0xff00_8001, 0x1234_5678]);
    let to_plug = Header {
        message_type: 15,
        revision: 2,
        objects: 2,
        data_role: true,
        ..Header::default()
    };
    w.message(OrderedSet::SopPrime, to_plug.value(), &vdm);
    w.idle_bits(12);
    let data: Vec<u8> = (0..80u8).map(|i| i.wrapping_mul(29)).collect();
    w.message(
        OrderedSet::Sop,
        extended(1, 3, 7),
        &chunk_of(0, &data[..26]),
    );
    w.idle_bits(12);
    let mut unchunked = extended_header(70, 0, false, false).to_le_bytes().to_vec();
    unchunked.extend(&data[..70]);
    w.message(OrderedSet::SopDoublePrime, extended(2, 4, 0), &unchunked);
    w.idle_bits(12);
    w.message_with(
        OrderedSet::Sop,
        control(3, 5),
        &[],
        &Faults {
            crc_xor: 4,
            ..Faults::default()
        },
    );
    w.idle_bits(12);
    w.message_with(OrderedSet::Sop, control(3, 5), &[], &symbol(9, 0b00001));
    w.idle_bits(12);
    w.message_with(
        OrderedSet::Sop,
        control(3, 5),
        &[],
        &Faults {
            stretch: Some((95, 140)),
            ..Faults::default()
        },
    );
    w.idle_bits(12);
    w.hard_reset();
    w.idle_bits(12);
    w.cable_reset();
    w.idle_bits(12);
    w.message(OrderedSet::Sop, control(13, 0), &[]);
    w.idle_bits(20);
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
fn rich_traffic_decodes_whole_and_well_formed() {
    let block = rich(SLOW);
    let lines = bare(&decode_with(&config(), SLOW, &block));
    for kind in [
        "header p8 good-crc",
        "sop p9 sop-prime",
        "extended-header p10 size 30 chunk 0 chunked",
        "data p11 +64 ",
        "crc p12 ",
        "error p13 symbol",
        "error p14 timing",
        "hard-reset p15",
        "cable-reset p16",
        "header p17 soft-reset",
    ] {
        assert!(
            lines.iter().any(|line| line.starts_with(kind)),
            "no {kind} in {lines:?}"
        );
    }
    assert_eq!(lines.iter().filter(|line| line.contains(" bad")).count(), 1);
    let mut harness = harness(&config(), SLOW, GraphConfig::default());
    harness.feed_with_gaps(&block, &gaps_in(&block), &[u64::MAX]);
    harness.end();
    assert_well_formed(&harness.items);
}

#[test]
fn every_cut_and_pause_yields_the_same_events() {
    let block = rich(SLOW);
    let gaps = gaps_in(&block);
    assert_block_invariant(|graph| harness(&config(), SLOW, graph), &block, &gaps);
    let mut waveform = waveform(RATE);
    waveform.message(
        OrderedSet::Sop,
        source_capabilities(0, 1),
        &object_bytes(&[fixed_pdo(5000, 3000, 0)]),
    );
    waveform.idle_bits(12);
    waveform.hard_reset();
    waveform.idle_bits(12);
    let block = waveform.finish();
    let gaps = gaps_in(&block);
    assert_block_invariant(|graph| harness(&config(), RATE, graph), &block, &gaps);
}

#[test]
fn progress_never_passes_what_is_under_way() {
    let block = rich(SLOW);
    let gaps = gaps_in(&block);
    for sizes in [&[1u64][..], &[3], &[7, 64], &[97]] {
        let mut harness = harness(&config(), SLOW, GraphConfig::default());
        harness.feed_cut(&block, &gaps, sizes, &[1]);
        harness.end();
        assert_well_formed(&harness.items);
        let progress = harness
            .items
            .iter()
            .filter(|item| matches!(item, Item::Progress(_)))
            .count();
        assert!(progress > 10, "blocks of {sizes:?}");
    }
}

#[test]
fn queues_stay_bounded_on_a_long_message() {
    let data: Vec<u8> = (0..=255u8).collect();
    let mut payload = extended_header(256, 0, false, false).to_le_bytes().to_vec();
    payload.extend(&data);
    let mut waveform = waveform(SLOW);
    for id in 0..3 {
        waveform.message(OrderedSet::Sop, extended(2, id, 0), &payload);
        waveform.idle_bits(12);
    }
    let block = waveform.finish();
    let config = GraphConfig {
        queue_capacity: 4,
        budget: 16,
    };
    let mut harness = harness(&self::config(), SLOW, config);
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
            UsbPdEvent::Data { bytes, .. } => Some(bytes),
            _ => None,
        })
        .flatten()
        .collect();
    assert_eq!(bytes, [&data[..], &data[..], &data[..]].concat());
}

/// A packet of random traffic.
#[derive(Clone, Debug)]
enum Segment {
    Control {
        set: OrderedSet,
        message_type: u8,
        id: u8,
    },
    Data {
        message_type: u8,
        objects: Vec<u32>,
    },
    Chunk {
        data: Vec<u8>,
    },
    Unchunked {
        data: Vec<u8>,
    },
    Damaged {
        faults: Faults,
    },
    HardReset,
    CableReset,
}

fn segment() -> impl Strategy<Value = Segment> {
    let set = prop::sample::select(vec![
        OrderedSet::Sop,
        OrderedSet::SopPrime,
        OrderedSet::SopDoublePrime,
    ]);
    prop_oneof![
        5 => (set, 0u8..32, 0u8..8).prop_map(|(set, message_type, id)| Segment::Control {
            set,
            message_type,
            id
        }),
        5 => (1u8..16, prop::collection::vec(any::<u32>(), 1..8))
            .prop_map(|(message_type, objects)| Segment::Data { message_type, objects }),
        2 => prop::collection::vec(any::<u8>(), 0..27).prop_map(|data| Segment::Chunk { data }),
        2 => prop::collection::vec(any::<u8>(), 0..60)
            .prop_map(|data| Segment::Unchunked { data }),
        3 => (
            any::<u32>(),
            prop::option::of((0usize..20, 0u8..32)),
            prop::option::of((0usize..200, 50u32..160)),
            prop::option::of(0usize..200),
        )
            .prop_map(|(crc_xor, symbol, stretch, cut)| Segment::Damaged {
                faults: Faults { crc_xor, symbol, stretch, cut },
            }),
        1 => Just(Segment::HardReset),
        1 => Just(Segment::CableReset),
    ]
}

fn traffic(segments: &[Segment], rate: u64, bit_rate: u64) -> LogicBlock {
    let mut waveform =
        UsbPdWaveform::new(&config(), CHANNELS, rate, Tick::ZERO).with_bit_rate(bit_rate);
    let w = &mut waveform;
    w.idle_bits(12);
    for (n, segment) in segments.iter().enumerate() {
        match segment {
            Segment::Control {
                set,
                message_type,
                id,
            } => w.message(*set, control(*message_type, *id), &[]),
            Segment::Data {
                message_type,
                objects,
            } => {
                let header = Header {
                    message_type: *message_type,
                    revision: 2,
                    objects: objects.len() as u8,
                    ..Header::default()
                };
                w.message(OrderedSet::Sop, header.value(), &object_bytes(objects));
            }
            Segment::Chunk { data } => {
                let mut payload = extended_header(data.len() as u16, 0, false, true)
                    .to_le_bytes()
                    .to_vec();
                payload.extend(data);
                let objects = (payload.len() as u8).div_ceil(4);
                payload.resize(usize::from(objects) * 4, 0);
                w.message(OrderedSet::Sop, extended(1, 0, objects), &payload);
            }
            Segment::Unchunked { data } => {
                let mut payload = extended_header(data.len() as u16, 0, false, false)
                    .to_le_bytes()
                    .to_vec();
                payload.extend(data);
                w.message(OrderedSet::Sop, extended(2, 0, 0), &payload);
            }
            Segment::Damaged { faults } => {
                let objects = object_bytes(&[fixed_pdo(5000, 3000, 0), fixed_pdo(9000, 3000, 0)]);
                w.message_with(OrderedSet::Sop, source_capabilities(0, 2), &objects, faults);
            }
            Segment::HardReset => w.hard_reset(),
            Segment::CableReset => w.cable_reset(),
        }
        w.idle_bits(10 + (n as u64 % 4) * 3);
    }
    waveform.finish()
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 48, ..ProptestConfig::default() })]

    /// Messages of every kind from a transmitter within the tolerance, sampled at 6 to 40
    /// ticks per bit, cut into blocks of any size, read with random pauses through tiny queues
    /// and budgets, cut short or with samples lost: the events are those of one uninterrupted
    /// run, and well formed; undamaged, every packet decodes whole.
    #[test]
    fn random_traffic_decodes_alike_however_it_is_fed(
        segments in prop::collection::vec(segment(), 1..10),
        rate in prop::sample::select(vec![1_800_000u64, 2_400_000, 4_000_000, 12_000_000]),
        bit_rate in 271_000u64..330_000,
        sizes in prop::collection::vec(1u64..300, 1..6),
        pauses in prop::collection::vec(0usize..12, 1..8),
        queue_capacity in 1usize..6,
        budget in 1u32..40,
        cut in prop::option::of(0.05f64..1.0),
        gap in prop::option::of((0.05f64..0.95, 1u64..300)),
    ) {
        let whole = traffic(&segments, rate, bit_rate);
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
        let mut reference = harness(&config(), rate, GraphConfig::default());
        reference.feed_with_gaps(&block, &gaps, &[u64::MAX]);
        reference.end();
        assert_well_formed(&reference.items);
        if cut.is_none() && gaps.is_empty() {
            let whole = segments
                .iter()
                .filter(|segment| !matches!(segment, Segment::Damaged { .. }))
                .count();
            let ends = reference
                .events()
                .iter()
                .filter(|event| {
                    matches!(
                        &event.payload,
                        UsbPdEvent::End { reason, .. } if *reason != EndReason::Error
                    )
                })
                .count();
            prop_assert!(ends >= whole, "{} whole packets of {}", ends, whole);
        }

        let config = GraphConfig { queue_capacity, budget };
        let mut harness = harness(&self::config(), rate, config);
        harness.drain_on_full = 1;
        harness.feed_cut(&block, &gaps, &sizes, &pauses);
        harness.end();
        prop_assert_eq!(harness.without_progress(), reference.without_progress());
        assert_well_formed(&harness.items);
        let stats = harness.pipeline().stats();
        prop_assert!(stats.within_capacity(), "{:?}", stats);
    }
}

/// A decoder built on USB PD for the test: what it read of each packet through the bus cursor.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Summary {
    pkt: PacketId,
    fields: Vec<String>,
    ending: Result<(), ReadError>,
}

impl EventSchema for Summary {
    const SCHEMA: SchemaId = SchemaId::new("test.usb-pd-summary", 1);
}

/// Reads up to `fields` fields of each packet and drains the rest.
struct Summarize {
    fields: usize,
}

impl Summarize {
    fn input(node: NodeId) -> InputRef<UsbPdEvent> {
        InputRef::of_node(node, 0)
    }

    fn output(node: NodeId) -> OutputRef<Summary> {
        OutputRef::of_node(node, 0)
    }
}

impl Node for Summarize {
    fn spec(&self) -> NodeSpec {
        NodeSpec {
            name: "summarize".into(),
            inputs: vec![InputSpec::new::<UsbPdEvent>("packets", Requires::new())],
            outputs: vec![OutputSpec::new::<Summary>("summaries", Props::new())],
        }
    }

    fn start(self: Box<Self>, mut io: Io) -> NodeFuture {
        let bus = UsbPdBus::new(io.input(0));
        Box::pin(summarize(bus, io.output(0), self.fields))
    }
}

/// A field in a few words.
fn field_line(field: &Field) -> String {
    match field {
        Field::Sop { set, corrupt: c } => format!("sop {}{}", name(set), corrupt(c)),
        Field::HardReset { corrupt: c } => format!("hard-reset{}", corrupt(c)),
        Field::CableReset { corrupt: c } => format!("cable-reset{}", corrupt(c)),
        Field::Header(header) => format!("header {}", header.message_type),
        Field::ExtendedHeader(header) => format!("extended-header {}", header.data_size),
        Field::Object { index, raw } => format!("object #{index} {raw:08x}"),
        Field::Pdo { index, .. } => format!("pdo #{index}"),
        Field::Rdo(request) => format!("rdo {}", request.position),
        Field::Data { offset, bytes } => format!("data +{offset} {}", hex(bytes)),
        Field::Crc { ok, .. } => format!("crc{}", if *ok { "" } else { " bad" }),
    }
}

async fn summarize(mut bus: UsbPdBus, mut out: Outbox<Summary>, wanted: usize) {
    loop {
        match bus.next().await {
            Next::Packet(mut packet) => {
                let pkt = packet.id();
                let mut fields = Vec::new();
                let ending = loop {
                    if fields.len() == wanted {
                        break packet.drain().await;
                    }
                    match packet.field_or_end().await {
                        Ok(Read::Field(field)) => fields.push(field_line(&field)),
                        Ok(Read::End) => break Ok(()),
                        Err(error) => break Err(error),
                    }
                };
                let at = packet.consumed();
                let summary = Summary {
                    pkt,
                    fields,
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
        // Whatever a packet left unread was drained.
        let _ = bus.finish().await;
    }
}

/// Source → USB PD → summaries of `wanted` fields, fed in blocks of 57 with `gaps` left out,
/// through tiny queues.
fn summaries(block: &LogicBlock, gaps: &[TickRange], wanted: usize) -> Vec<Item<Summary>> {
    let mut harness = Harness::build(
        CHANNELS,
        TimeBase::new(RATE),
        GraphConfig {
            queue_capacity: 2,
            budget: 3,
        },
        |graph, source| {
            let pd = graph.add(UsbPd::new(config()).expect("a valid configuration"));
            graph.connect(source.samples, UsbPd::input(pd))?;
            let summarize = graph.add(Summarize { fields: wanted });
            graph.connect(UsbPd::events(pd), Summarize::input(summarize))?;
            Ok(Summarize::output(summarize))
        },
    )
    .expect("builds");
    harness.drain_on_full = 1;
    harness.feed_with_gaps(block, gaps, &[57]);
    harness.end();
    testing::without_progress(&harness.items)
}

#[test]
fn decoders_built_on_usb_pd_read_packets_through_the_bus_cursor() {
    let pdos = [fixed_pdo(5000, 3000, 0), fixed_pdo(9000, 2000, 0)];
    let data: Vec<u8> = (1..=26).collect();
    let rdo = request(1, 3000, 3000, 0);
    let mut waveform = waveform(RATE);
    waveform.message(
        OrderedSet::Sop,
        source_capabilities(0, 2),
        &object_bytes(&pdos),
    );
    waveform.idle_bits(12);
    waveform.message(OrderedSet::Sop, control(1, 0), &[]);
    waveform.idle_bits(12);
    waveform.message(OrderedSet::Sop, extended(1, 1, 7), &chunk_of(0, &data));
    waveform.idle_bits(12);
    waveform.message(OrderedSet::Sop, request_header(0), &object_bytes(&[rdo]));
    waveform.idle_bits(12);
    waveform.message_with(OrderedSet::Sop, control(3, 1), &[], &symbol(5, 0));
    waveform.idle_bits(12);
    waveform.hard_reset();
    waveform.idle_bits(12);
    let cut_off = waveform.position();
    waveform.message(OrderedSet::Sop, control(3, 2), &[]);
    waveform.idle_bits(12);
    waveform.message(OrderedSet::Sop, control(1, 2), &[]);
    waveform.idle_bits(12);
    let last = waveform.position();
    waveform.message(OrderedSet::Sop, control(6, 3), &[]);
    let whole = waveform.finish();
    // The seventh packet loses samples in its header, the last one is cut off in its CRC.
    let gap_start = cut_off.offset(BIT * 90);
    let gap = TickRange::new(gap_start, gap_start.offset(60));
    let end = last.offset(BIT * 120);
    let block = whole
        .slice(TickRange::new(Tick::ZERO, end))
        .expect("a slice");
    let crc = |ok: bool| if ok { "crc" } else { "crc bad" };
    let expected: Vec<(Vec<String>, Result<(), ReadError>)> = vec![
        (
            vec![
                "sop sop".to_owned(),
                "header 1".to_owned(),
                format!("object #1 {:08x}", pdos[0]),
                "pdo #1".to_owned(),
                format!("object #2 {:08x}", pdos[1]),
                "pdo #2".to_owned(),
                crc(true).to_owned(),
            ],
            Ok(()),
        ),
        (
            vec![
                "sop sop".to_owned(),
                "header 1".to_owned(),
                "crc".to_owned(),
            ],
            Ok(()),
        ),
        (
            vec![
                "sop sop".to_owned(),
                "header 1".to_owned(),
                "extended-header 30".to_owned(),
                format!("data +0 {}", hex(&data)),
                "crc".to_owned(),
            ],
            Ok(()),
        ),
        (
            vec![
                "sop sop".to_owned(),
                "header 2".to_owned(),
                format!("object #1 {rdo:08x}"),
                "rdo 1".to_owned(),
                "crc".to_owned(),
            ],
            Ok(()),
        ),
        (
            vec!["sop sop".to_owned()],
            Err(ReadError::Damaged(Some(ErrorReason::Symbol))),
        ),
        (vec!["hard-reset".to_owned()], Ok(())),
        (vec!["sop sop".to_owned()], Err(ReadError::Truncated)),
        (
            vec![
                "sop sop".to_owned(),
                "header 1".to_owned(),
                "crc".to_owned(),
            ],
            Ok(()),
        ),
        (
            vec!["sop sop".to_owned(), "header 6".to_owned()],
            Err(ReadError::Truncated),
        ),
    ];

    // Every field read, then the same packets read two fields at most and drained.
    for wanted in [usize::MAX, 2] {
        let items = summaries(&block, &[gap], wanted);
        let read: Vec<&Summary> = items
            .iter()
            .filter_map(Item::event)
            .map(|event| &event.payload)
            .collect();
        assert_eq!(read.len(), expected.len(), "{wanted} fields");
        for (n, (summary, (fields, ending))) in read.iter().zip(&expected).enumerate() {
            assert_eq!(summary.pkt, PacketId(n as u64 + 1));
            let fields: Vec<String> = fields.iter().take(wanted).cloned().collect();
            assert_eq!(summary.fields, fields, "packet {} of {wanted}", n + 1);
            assert_eq!(summary.ending, *ending, "packet {} of {wanted}", n + 1);
        }
        // The gap and the end follow the packets they cut off.
        assert!(matches!(items[7], Item::Gap(range) if range == gap));
        assert_eq!(items.last(), Some(&Item::End(end)));

        // Each summary covers its packet from its begin to its end, and settles with the end.
        let mut decoder = harness(&config(), RATE, GraphConfig::default());
        decoder.feed_with_gaps(&block, &[gap], &[u64::MAX]);
        decoder.end();
        for summary in items.iter().filter_map(Item::event) {
            let pkt = summary.payload.pkt;
            let of_packet: Vec<_> = decoder
                .events()
                .into_iter()
                .filter(|event| event.payload.pkt() == pkt)
                .collect();
            let begin = of_packet.first().expect("a begin");
            let end = of_packet.last().expect("an end");
            assert_eq!(
                summary.span,
                begin.span.join(end.span),
                "packet {pkt:?} of {wanted}"
            );
            assert_eq!(summary.settled, end.settled, "packet {pkt:?} of {wanted}");
        }
    }
}
