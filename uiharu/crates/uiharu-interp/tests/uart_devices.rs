// SPDX-License-Identifier: Apache-2.0

//! Independent UART device vectors, corruption, truncation and scheduling equivalence.
mod common;
use common::program;
use serde_json::{Value, json};
use uiharu_core::testing::{Harness, assert_block_invariant, without_progress};
use uiharu_core::{GraphConfig, InputRef, OutputRef, Props, Tick, TickRange, TimeBase};
use uiharu_interp::{DynEvent, Interpreter, ToDyn};
use uiharu_protocols::uart::{
    Uart, UartConfig, UartEvent,
    synth::{Faults, UartWaveform},
};
use uiharu_protocols::{amulet_ascii, pan1321};
fn config() -> UartConfig {
    UartConfig {
        baud: 9600,
        ..UartConfig::default()
    }
}
fn build(name: &str, interpreted: bool, limits: GraphConfig) -> Harness<DynEvent> {
    build_limit(name, interpreted, limits, 4096)
}
fn build_limit(
    name: &str,
    interpreted: bool,
    limits: GraphConfig,
    max_bytes: u32,
) -> Harness<DynEvent> {
    Harness::build(1, TimeBase::new(96000), limits, |g, s| {
        let uart = g.add(Uart::new(config()).expect("UART"));
        g.connect(s.samples, Uart::input(uart))?;
        if interpreted {
            let bridge = g.add(ToDyn::<UartEvent>::new(Props::new()));
            g.connect(Uart::events(uart), InputRef::of_node(bridge, 0))?;
            let node = g.add(
                Interpreter::new(program(name), &json!({"max_bytes":max_bytes})).expect("device"),
            );
            g.connect(OutputRef::of_node(bridge, 0), Interpreter::bus_input(node))?;
            return Ok(Interpreter::events(node));
        }
        macro_rules! compiled {
            ($m:ident,$n:ident) => {{
                let node = g.add($m::$n::new($m::DeviceConfig { max_bytes }).expect("device"));
                g.connect(Uart::events(uart), $m::$n::input(node))?;
                let bridge = g.add(ToDyn::<$m::DeviceEvent>::new(Props::new()));
                g.connect($m::$n::events(node), InputRef::of_node(bridge, 0))?;
                Ok(OutputRef::of_node(bridge, 0))
            }};
        }
        match name {
            "amulet-ascii" => compiled!(amulet_ascii, AmuletAscii),
            "pan1321" => compiled!(pan1321, Pan1321),
            _ => panic!("test protocol"),
        }
    })
    .expect("graph")
}
fn check(name: &str, bytes: &[u8]) -> Vec<Value> {
    let mut w = UartWaveform::new(config(), 1, 10, Tick::ZERO);
    w.idle(20);
    w.bytes(bytes.iter().copied());
    w.idle(20);
    let block = w.finish();
    let mut baseline = build(name, false, GraphConfig::default());
    baseline.feed(&block, &[u64::MAX]);
    baseline.end();
    baseline.assert_progress_promise();
    for gaps in [vec![], vec![TickRange::new(Tick(520), Tick(660))]] {
        for limits in [
            GraphConfig::default(),
            GraphConfig {
                queue_capacity: 1,
                budget: 3,
            },
        ] {
            let mut c = build(name, false, limits);
            let mut i = build(name, true, limits);
            c.feed_cut(&block, &gaps, &[1, 13, 211], &[0, 1, 7]);
            i.feed_cut(&block, &gaps, &[1, 13, 211], &[0, 1, 7]);
            c.end();
            i.end();
            c.assert_progress_promise();
            i.assert_progress_promise();
            assert_eq!(without_progress(&c.items), without_progress(&i.items));
            if gaps.is_empty() {
                assert_eq!(
                    without_progress(&c.items),
                    without_progress(&baseline.items)
                );
            }
        }
    }
    assert_block_invariant(|limits| build(name, false, limits), &block, &[]);
    baseline
        .events()
        .iter()
        .map(|e| e.payload.0.clone())
        .collect()
}
fn field(e: &[Value], name: &str, value: u32) {
    assert!(
        e.iter()
            .any(|e| e["field"] == name && e["value"] == value && e["valid"] == true),
        "{name}: {e:?}"
    );
}
#[test]
fn amulet_scalars_strings_arrays_and_page_checksum() {
    let e = check(
        "amulet-ascii",
        &[
            0xd0, b'1', b'A', 0xe0, b'1', b'A', b'F', b'F', 0xd6, b'0', b'2', b'1', b'2', b'3',
            b'4', 0xe2, b'0', b'3', b'H', b'i', 0, 0xee, b'0', b'4', b'A', b'B', b'C', b'D', 0,
            0xa0, 2, 0, 1, 0x5d, 0xf0, 0xf1,
        ],
    );
    field(&e, "address", 0x1a);
    field(&e, "value", 255);
    field(&e, "value", 0x1234);
    field(&e, "element", 0xabcd);
    field(&e, "page", 1);
    field(&e, "checksum", 0);
    field(&e, "ack", 0xf0);
    field(&e, "nack", 0xf1);
}
#[test]
fn amulet_invalid_hex_checksum_and_unfinished_message() {
    let e = check(
        "amulet-ascii",
        &[
            0xd5, b'0', b'1', b'G', b'0', 0xa0, 2, 0, 1, 0, 0xe2, b'0', b'0', b'x',
        ],
    );
    assert!(e.iter().any(|e| e["reason"] == "invalid"));
    assert!(e.iter().any(|e| e["reason"] == "aborted"));
}
#[test]
fn pan1321_binary_payload_does_not_end_at_embedded_crlf() {
    let e = check(
        "pan1321",
        b"AT+JAAC=1\r\nAT+JSDA=4,A\r\nB\r\nOK\r\nERR=12\r\nAT+JRES\r\n",
    );
    field(&e, "command", 1);
    field(&e, "command", 3);
    field(&e, "declared-length", 4);
    field(&e, "mode", 1);
    field(&e, "status", 0);
    field(&e, "error-code", 12);
    assert_eq!(
        e.iter()
            .filter(|e| e["kind"] == "end" && e["reason"] == "complete")
            .count(),
        5
    );
}
#[test]
fn uart_corruption_and_truncated_binary_abort() {
    check("pan1321", b"AT+JSDA=8,short");
    for name in ["pan1321", "amulet-ascii"] {
        let mut w = UartWaveform::new(config(), 1, 10, Tick::ZERO);
        w.idle(20);
        w.bytes(if name == "pan1321" {
            vec![b'A', b'T']
        } else {
            vec![0xe2, b'0', b'0']
        });
        w.frame(65, Faults::FRAMING);
        w.idle(30);
        let block = w.finish();
        let mut h = build(name, false, GraphConfig::default());
        h.feed(&block, &[u64::MAX]);
        h.end();
        assert!(
            h.events()
                .iter()
                .any(|e| e.payload.0["reason"] == "invalid")
        );
    }
}

#[test]
fn pan_unknown_invalid_length_and_limit_recover_at_crlf() {
    let bytes = b"WHAT?\r\nAT+JAAC=10\r\nAT+JSDA=1,AX\r\nAT+JAAC=1\r\n";
    let e = check("pan1321", bytes);
    assert!(e.iter().any(|v| v["reason"] == "unsupported"));
    assert_eq!(e.iter().filter(|v| v["reason"] == "invalid").count(), 2);
    let mut w = UartWaveform::new(config(), 1, 10, Tick::ZERO);
    w.idle(20);
    w.bytes(*b"AT+JSDA=30,0123456789AT+JAAC=1\r\nOK\r\n");
    w.idle(20);
    let block = w.finish();
    let mut c = build_limit(
        "pan1321",
        false,
        GraphConfig {
            queue_capacity: 1,
            budget: 3,
        },
        16,
    );
    let mut i = build_limit(
        "pan1321",
        true,
        GraphConfig {
            queue_capacity: 1,
            budget: 3,
        },
        16,
    );
    c.feed(&block, &[1, 31]);
    i.feed(&block, &[1, 31]);
    c.end();
    i.end();
    assert_eq!(without_progress(&c.items), without_progress(&i.items));
    assert_eq!(
        c.events()
            .iter()
            .filter(|e| e.payload.0["kind"] == "begin")
            .count(),
        2
    );
    assert!(
        c.events()
            .iter()
            .any(|e| e.payload.0["field"] == "status" && e.payload.0["value"] == 0)
    );
}

#[test]
fn amulet_graphics_coordinates_colors_and_echoed_replies() {
    let mut bytes = vec![];
    for code in [0xd9, 0xda, 0xdb, 0xe9, 0xea, 0xeb] {
        bytes.push(code);
        bytes.extend_from_slice(b"000A0005010C00822546732");
    }
    let e = check("amulet-ascii", &bytes);
    assert_eq!(
        e.iter()
            .filter(|v| v["kind"] == "end" && v["reason"] == "complete")
            .count(),
        6
    );
    for (name, value) in [
        ("point-1-x", 10),
        ("point-1-y", 5),
        ("point-2-x", 268),
        ("point-2-y", 130),
        ("delta-x", 268),
        ("delta-y", 130),
        ("blue", 0x25),
        ("green", 0x46),
        ("red", 0x73),
        ("line-weight", 2),
        ("unused-line-weight", 2),
    ] {
        field(&e, name, value)
    }
    let mut invalid = vec![0xd9];
    invalid.extend_from_slice(b"000A0005010C0082254G732");
    let e = check("amulet-ascii", &invalid);
    assert!(e.iter().any(|v| v["reason"] == "invalid"));
    let e = check("amulet-ascii", &[0xda, b'0', b'0', b'0']);
    assert!(e.iter().any(|v| v["reason"] == "aborted"));
}
