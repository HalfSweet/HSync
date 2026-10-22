// SPDX-License-Identifier: Apache-2.0
//! Independent PS/2 Set 2, mouse and bidirectional command traces.
mod common;
use common::program;
use serde_json::{Value, json};
use uiharu_core::testing::{Harness, assert_block_invariant, without_progress};
use uiharu_core::{GraphConfig, InputRef, LogicBlock, OutputRef, Props, Tick, TickRange, TimeBase};
use uiharu_interp::{DynEvent, Interpreter, ToDyn};
use uiharu_protocols::ps2::{
    Ps2, Ps2Config, Ps2Event,
    synth::{Faults, Ps2Waveform},
};
use uiharu_protocols::ps2_device::{Config, Event, Ps2Device};
fn build(mode: &str, interpreted: bool, limits: GraphConfig) -> Harness<DynEvent> {
    Harness::build(2, TimeBase::new(1_000_000), limits, |g, s| {
        let physical = g.add(Ps2::new(Ps2Config::default()).expect("PS2"));
        g.connect(s.samples, Ps2::input(physical))?;
        if interpreted {
            let b = g.add(ToDyn::<Ps2Event>::new(Ps2Config::default().props()));
            g.connect(Ps2::events(physical), InputRef::of_node(b, 0))?;
            let n = g.add(
                Interpreter::new(program("ps2-device"), &json!({"mode":mode})).expect("overlay"),
            );
            g.connect(OutputRef::of_node(b, 0), Interpreter::bus_input(n))?;
            return Ok(Interpreter::events(n));
        }
        let c: Config = serde_json::from_value(json!({"mode":mode})).expect("config");
        let n = g.add(Ps2Device::new(c).expect("overlay"));
        g.connect(Ps2::events(physical), Ps2Device::input(n))?;
        let b = g.add(ToDyn::<Event>::new(Props::new()));
        g.connect(Ps2Device::events(n), InputRef::of_node(b, 0))?;
        Ok(OutputRef::of_node(b, 0))
    })
    .expect("graph")
}
fn wave() -> Ps2Waveform {
    let mut w = Ps2Waveform::new(
        Ps2Config::default(),
        2,
        TimeBase::new(1_000_000),
        Tick::ZERO,
    );
    w.idle_us(500);
    w
}
fn bytes(w: &mut Ps2Waveform, data: &[u8]) {
    for &b in data {
        w.device_byte(b)
    }
}
fn decode(mode: &str, b: &LogicBlock) -> Vec<Value> {
    let mut c = build(mode, false, GraphConfig::default());
    c.run_all(b);
    c.assert_progress_promise();
    for limits in [
        GraphConfig::default(),
        GraphConfig {
            queue_capacity: 1,
            budget: 3,
        },
    ] {
        let mut i = build(mode, true, limits);
        i.feed_cut(b, &[], &[1, 211, 13], &[0, 1, 7]);
        i.end();
        i.assert_progress_promise();
        assert_eq!(without_progress(&c.items), without_progress(&i.items))
    }
    c.events().iter().map(|e| e.payload.0.clone()).collect()
}
#[test]
fn set2_make_break_extended_pause_and_print_screen() {
    let mut w = wave();
    bytes(
        &mut w,
        &[
            0x1c, 0xf0, 0x1c, 0xe0, 0x75, 0xe0, 0xf0, 0x75, 0xe1, 0x14, 0x77, 0xe1, 0xf0, 0x14,
            0xf0, 0x77, 0xe0, 0x12, 0xe0, 0x7c, 0xe0, 0xf0, 0x7c, 0xe0, 0xf0, 0x12,
        ],
    );
    let e = decode("keyboard-set2", &w.finish());
    assert_eq!(e.iter().filter(|v| v["kind"] == "key").count(), 4);
    assert!(e.iter().any(|v| v["kind"] == "key"
        && v["code"] == 0x75
        && v["extended"] == true
        && v["released"] == true));
    assert_eq!(e.iter().filter(|v| v["kind"] == "pause").count(), 1);
    assert_eq!(e.iter().filter(|v| v["kind"] == "print-screen").count(), 2);
    assert!(!e.iter().any(|v| v["kind"] == "error"))
}
#[test]
fn mouse_signed_nine_bit_data_and_command_reply_boundaries() {
    let mut w = wave();
    bytes(&mut w, &[0x3b, 0, 0xff, 0xc8, 0x80, 0xfa]);
    w.host_byte(0xf2);
    bytes(&mut w, &[0xfa, 0]);
    w.host_byte(0xe9);
    bytes(&mut w, &[0xfa, 0x20, 2, 100]);
    w.host_byte(0xeb);
    bytes(&mut w, &[0xfa, 8, 1, 2]);
    let e = decode("mouse", &w.finish());
    let packets: Vec<_> = e.iter().filter(|v| v["kind"] == "mouse").collect();
    assert_eq!(packets.len(), 3);
    assert_eq!(packets[0]["x"], 256);
    assert_eq!(packets[0]["y"], 1);
    assert_eq!(packets[0]["xNegative"], true);
    assert_eq!(packets[1]["y"], 250);
    assert_eq!(packets[1]["yOverflow"], true);
    assert_eq!(packets[2]["x"], 1);
    assert_eq!(
        e.iter()
            .filter(|v| v["kind"] == "reply" && v["command"] == 0xe9)
            .count(),
        4
    )
}
#[test]
fn keyboard_host_parameters_set_changes_reset_and_errors() {
    let mut w = wave();
    w.host_byte(0xed);
    w.device_byte(0xfa);
    w.host_byte(7);
    w.device_byte(0xfa);
    w.host_byte(0xf0);
    w.device_byte(0xfa);
    w.host_byte(1);
    bytes(&mut w, &[0xfa, 0x1c]);
    w.host_byte(0xff);
    bytes(&mut w, &[0xfa, 0xaa, 0x1c, 0xe0, 0xe0, 0x1b, 0xf0]);
    let e = decode("keyboard-set2", &w.finish());
    assert!(
        e.iter()
            .any(|v| v["kind"] == "command" && v["parameter"] == true && v["value"] == 7)
    );
    assert!(e.iter().any(|v| v["reason"] == "unsupported-set"));
    assert!(e.iter().any(|v| v["reason"] == "bad-sequence"));
    assert!(e.iter().any(|v| v["reason"] == "aborted"));
    assert_eq!(
        e.iter()
            .filter(|v| v["kind"] == "key" && v["code"] == 0x1c)
            .count(),
        1
    )
}
#[test]
fn malformed_physical_input_and_gaps_preserve_bounded_progress() {
    let mut w = wave();
    bytes(&mut w, &[0xe1, 0x14]);
    w.device_byte_with(
        0x77,
        Faults {
            parity: true,
            ..Faults::NONE
        },
    );
    bytes(&mut w, &[0x1c, 0xe0, 0x75, 0xf0]);
    let b = w.finish();
    let e = decode("keyboard-set2", &b);
    assert!(e.iter().any(|v| v["reason"] == "input-error"));
    for mode in ["keyboard-set2", "mouse"] {
        for interpreted in [false, true] {
            assert_block_invariant(
                |g| build(mode, interpreted, g),
                &b,
                &[TickRange::new(Tick(1500), Tick(1700))],
            )
        }
    }
}

#[test]
fn scan_sets_one_and_three_and_explicit_extended_mouse_profiles() {
    let mut w = wave();
    bytes(
        &mut w,
        &[
            0x1e, 0x9e, 0xe0, 0x48, 0xe0, 0xc8, 0xe1, 0x1d, 0x45, 0xe1, 0x9d, 0xc5, 0xe0, 0x2a,
            0xe0, 0x37, 0xe0, 0xb7, 0xe0, 0xaa,
        ],
    );
    let b = w.finish();
    let e = decode("keyboard-set1", &b);
    assert_eq!(e.iter().filter(|e| e["kind"] == "key").count(), 4);
    assert!(
        e.iter()
            .any(|e| e["kind"] == "key" && e["code"] == 0x1e && e["released"] == true)
    );
    assert_eq!(e.iter().filter(|e| e["kind"] == "pause").count(), 1);
    assert_eq!(e.iter().filter(|e| e["kind"] == "print-screen").count(), 2);
    assert!(!e.iter().any(|e| e["kind"] == "error"), "{e:?}");
    let mut w = wave();
    bytes(&mut w, &[0x1c, 0xf0, 0x1c, 0x57]);
    let e = decode("keyboard-set3", &w.finish());
    assert!(
        e.iter()
            .any(|e| e["kind"] == "key" && e["code"] == 0x1c && e["released"] == true)
    );
    for (mode, fourth, buttons, magnitude) in [
        ("mouse-wheel", 0x81, 3, 127),
        ("mouse-explorer", 0x3f, 27, 1),
    ] {
        let mut w = wave();
        bytes(&mut w, &[0x3b, 0, 0xff, fourth]);
        let b = w.finish();
        let e = decode(mode, &b);
        assert!(
            e.iter()
                .any(|e| e["kind"] == "mouse" && e["x"] == 256 && e["y"] == 1)
        );
        assert!(
            e.iter().any(|e| e["kind"] == "wheel"
                && e["buttons"] == buttons
                && e["magnitude"] == magnitude
                && e["negative"] == true),
            "{e:?}"
        );
        for interpreted in [false, true] {
            assert_block_invariant(
                |g| build(mode, interpreted, g),
                &b,
                &[TickRange::new(Tick(1500), Tick(1700))],
            );
        }
    }
}

#[test]
fn set1_left_shift_release_is_scan_data_but_reset_aa_remains_an_explicit_reply() {
    let mut w = wave();
    bytes(&mut w, &[0x2a, 0xaa, 0x2a, 0xaa]);
    w.host_byte(0xff);
    bytes(&mut w, &[0xfa, 0xaa]);
    let block = w.finish();
    let events = decode("keyboard-set1", &block);
    let keys: Vec<_> = events
        .iter()
        .filter(|event| event["kind"] == "key")
        .collect();
    assert_eq!(keys.len(), 4);
    for (index, event) in keys.iter().enumerate() {
        assert_eq!(event["code"], 0x2a);
        assert_eq!(event["extended"], false);
        assert_eq!(event["released"], index % 2 != 0);
    }
    assert!(
        !events
            .iter()
            .any(|event| event["kind"] == "status" && event["value"] == 0xaa)
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| event["kind"] == "reply"
                && event["command"] == 0xff
                && event["value"] == 0xaa)
            .count(),
        1
    );
    for interpreted in [false, true] {
        assert_block_invariant(
            |limits| build("keyboard-set1", interpreted, limits),
            &block,
            &[],
        );
        assert_block_invariant(
            |limits| build("keyboard-set1", interpreted, limits),
            &block,
            &[TickRange::new(Tick(1500), Tick(1700))],
        );
    }
    let mut w = wave();
    bytes(&mut w, &[0xaa]);
    assert_eq!(
        decode("keyboard-set2", &w.finish()),
        vec![json!({"kind":"status","value":0xaa})]
    );
}
