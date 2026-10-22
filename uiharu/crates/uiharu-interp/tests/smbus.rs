// SPDX-License-Identifier: Apache-2.0
//! SMBus3.2 independent PEC division and all scalar/block transaction shapes.
mod common;
use common::program;
use serde_json::{Value, json};
use uiharu_core::testing::{Harness, assert_block_invariant, without_progress};
use uiharu_core::{GraphConfig, InputRef, LogicBlock, OutputRef, Tick, TickRange, TimeBase};
use uiharu_interp::{DynEvent, Interpreter, ToDyn};
use uiharu_protocols::i2c::{I2c, I2cConfig, I2cEvent, synth::I2cWaveform};
use uiharu_protocols::smbus::{Config, Event, Smbus};
fn build(config: &Value, interpreted: bool, limits: GraphConfig) -> Harness<DynEvent> {
    Harness::build(2, TimeBase::new(1_000_000), limits, |g, s| {
        let bus = g.add(I2c::new(I2cConfig::default()).expect("I2C"));
        g.connect(s.samples, I2c::input(bus))?;
        if interpreted {
            let b = g.add(ToDyn::<I2cEvent>::new(I2cConfig::default().props()));
            g.connect(I2c::events(bus), InputRef::of_node(b, 0))?;
            let n = g.add(Interpreter::new(program("smbus"), config).expect("SMBus"));
            g.connect(OutputRef::of_node(b, 0), Interpreter::bus_input(n))?;
            return Ok(Interpreter::events(n));
        }
        let c: Config = serde_json::from_value(config.clone()).expect("config");
        let decoder = Smbus::new(c).expect("SMBus");
        let props = uiharu_core::Node::spec(&decoder).outputs[0].props.clone();
        let n = g.add(decoder);
        g.connect(I2c::events(bus), Smbus::input(n))?;
        let b = g.add(ToDyn::<Event>::new(props));
        g.connect(Smbus::events(n), InputRef::of_node(b, 0))?;
        Ok(OutputRef::of_node(b, 0))
    })
    .expect("SMBus graph")
}
fn pec(bytes: &[u8]) -> u8 {
    let mut bits: Vec<bool> = bytes
        .iter()
        .flat_map(|&b| (0..8).rev().map(move |i| b & (1 << i) != 0))
        .collect();
    let n = bits.len();
    bits.extend([false; 8]);
    for i in 0..n {
        if bits[i] {
            for (j, p) in [true, false, false, false, false, false, true, true, true]
                .into_iter()
                .enumerate()
            {
                bits[i + j] ^= p;
            }
        }
    }
    bits[n..].iter().fold(0, |v, &b| (v << 1) | u8::from(b))
}
fn wave() -> I2cWaveform {
    let mut w = I2cWaveform::new(I2cConfig::default(), 2, 2, Tick::ZERO);
    w.idle(8);
    w
}
fn packet(
    w: &mut I2cWaveform,
    address: u8,
    write: &[u8],
    read: Option<&[u8]>,
    with_pec: bool,
    bad: bool,
) {
    let mut covered = vec![];
    w.start();
    if !write.is_empty() || read.is_none() {
        w.byte(address * 2, true);
        covered.push(address * 2);
        for &b in write {
            w.byte(b, true);
            covered.push(b);
        }
        if read.is_some() {
            w.restart();
        }
    }
    if let Some(data) = read {
        w.byte(address * 2 + 1, true);
        covered.push(address * 2 + 1);
        for (i, &b) in data.iter().enumerate() {
            w.byte(b, with_pec || i + 1 < data.len());
            covered.push(b);
        }
        if with_pec {
            w.byte(pec(&covered) ^ u8::from(bad), false);
        }
    } else if with_pec {
        w.byte(pec(&covered) ^ u8::from(bad), true);
    }
    w.stop();
    w.idle(8);
}
fn decode(mode: &str, pec_enabled: bool, block: &LogicBlock) -> Vec<Value> {
    let config = json!({"mode":mode,"pec":pec_enabled});
    let mut c = build(&config, false, GraphConfig::default());
    c.feed(block, &[u64::MAX]);
    c.end();
    c.assert_progress_promise();
    for limits in [
        GraphConfig::default(),
        GraphConfig {
            queue_capacity: 1,
            budget: 3,
        },
    ] {
        let mut i = build(&config, true, limits);
        i.feed_cut(block, &[], &[1, 13, 211], &[0, 1, 7]);
        i.end();
        i.assert_progress_promise();
        assert_eq!(without_progress(&c.items), without_progress(&i.items));
    }
    c.events().iter().map(|e| e.payload.0.clone()).collect()
}
#[test]
fn published_read_word_pec_and_every_scalar_shape() {
    assert_eq!(pec(&[0xb4, 7, 0xb5, 0xd2, 0x3a]), 0x30);
    for (mode, write, read) in [
        ("send-byte", vec![7], None),
        ("receive-byte", vec![], Some(vec![0x55])),
        ("write-byte", vec![7, 0x55], None),
        ("write-word", vec![7, 0x34, 0x12], None),
        ("read-byte", vec![7], Some(vec![0x55])),
        ("read-word", vec![7], Some(vec![0xd2, 0x3a])),
        ("process-call", vec![7, 0x34, 0x12], Some(vec![0x78, 0x56])),
        ("write-32", vec![7, 1, 2, 3, 4], None),
        ("read-32", vec![7], Some(vec![1, 2, 3, 4])),
        (
            "write-64",
            vec![7, 0xef, 0xcd, 0xab, 0x89, 0x67, 0x45, 0x23, 1],
            None,
        ),
        (
            "read-64",
            vec![7],
            Some(vec![0xef, 0xcd, 0xab, 0x89, 0x67, 0x45, 0x23, 1]),
        ),
    ] {
        for use_pec in [false, true] {
            let mut w = wave();
            packet(&mut w, 0x5a, &write, read.as_deref(), use_pec, false);
            let e = decode(mode, use_pec, &w.finish());
            assert!(
                e.iter().any(|v| v["kind"] == "end" && v["valid"] == true),
                "{mode}: {e:?}"
            );
            if mode.ends_with("64") {
                assert!(e.iter().any(|v| v["kind"] == "value"
                    && v["low"] == 0x89abcdefu32
                    && v["high"] == 0x01234567u32));
            }
        }
    }
}
#[test]
fn blocks_zero_maximum_and_combined_limit() {
    for (mode, write, read) in [
        ("block-write", vec![7, 0], None),
        ("block-read", vec![7], Some(vec![0])),
        ("block-process-call", vec![7, 0], Some(vec![0])),
        (
            "block-process-call",
            vec![7, 2, 1, 2],
            Some(vec![3, 3, 4, 5]),
        ),
    ] {
        let mut w = wave();
        packet(&mut w, 0x16, &write, read.as_deref(), true, false);
        let e = decode(mode, true, &w.finish());
        assert!(
            e.iter().any(|v| v["valid"] == true && v["kind"] == "end"),
            "{mode}: {e:?}"
        );
    }
    let mut w = wave();
    let mut payload = vec![7, 255];
    payload.extend(0..255);
    packet(&mut w, 0x16, &payload, None, true, false);
    assert!(
        decode("block-write", true, &w.finish())
            .iter()
            .any(|v| v["kind"] == "end" && v["valid"] == true)
    );
    let mut w = wave();
    let mut write = vec![7, 128];
    write.extend([1; 128]);
    let mut read = vec![128];
    read.extend([2; 128]);
    packet(&mut w, 0x16, &write, Some(&read), true, false);
    assert!(
        decode("block-process-call", true, &w.finish())
            .iter()
            .any(|v| v["kind"] == "end" && v["valid"] == false)
    );
}
#[test]
fn quick_host_notify_pec_errors_nacks_and_truncation() {
    let mut w = wave();
    packet(&mut w, 0x16, &[], None, false, false);
    assert!(
        decode("quick", true, &w.finish())
            .iter()
            .any(|v| v["kind"] == "end" && v["valid"] == true)
    );
    let mut w = wave();
    packet(&mut w, 8, &[0xb4, 0x34, 0x12], None, false, false);
    let e = decode("host-notify", true, &w.finish());
    assert!(
        e.iter()
            .any(|v| v["kind"] == "notify" && v["device"] == 0x5a)
    );
    assert!(e.iter().any(|v| v["kind"] == "end" && v["valid"] == true));
    let mut w = wave();
    packet(&mut w, 0x5a, &[7], Some(&[0xd2, 0x3a]), true, true);
    assert!(
        decode("read-word", true, &w.finish())
            .iter()
            .any(|v| v["reason"] == "bad-pec")
    );
    let mut w = wave();
    w.start();
    w.byte(0x2c, true);
    w.byte(7, true);
    w.byte(0x34, false);
    w.stop();
    w.idle(8);
    assert!(
        decode("write-word", false, &w.finish())
            .iter()
            .any(|v| v["reason"] == "nack")
    );
    let mut w = wave();
    w.start();
    w.byte(0x2c, true);
    w.byte(7, true);
    w.byte(0x34, true);
    w.idle(8);
    assert!(
        decode("write-word", true, &w.finish())
            .iter()
            .any(|v| v["reason"] == "aborted")
    );
}
#[test]
fn host_notify_dont_care_bit_and_early_read_nack() {
    // SMBus 3.2 section 2.5.3 allows either value of the Host Notify X bit.
    for byte in [0xb4, 0xb5] {
        let mut w = wave();
        packet(&mut w, 8, &[byte, 0x34, 0x12], None, false, false);
        let e = decode("host-notify", true, &w.finish());
        assert!(
            e.iter()
                .any(|v| v["kind"] == "notify" && v["device"] == 0x5a)
        );
        assert!(e.iter().any(|v| v["kind"] == "end" && v["valid"] == true));
    }
    for repeated_start in [false, true] {
        let mut w = wave();
        w.start();
        w.byte(0xb5, true);
        w.byte(0x12, false);
        if repeated_start {
            w.restart();
            w.byte(0xb5, true);
        }
        w.byte(0x34, false);
        w.stop();
        w.idle(8);
        let e = decode("raw", false, &w.finish());
        assert!(
            e.iter()
                .any(|v| v["kind"] == "end" && v["reason"] == "invalid")
        );
        assert!(!e.iter().any(|v| v["kind"] == "end" && v["valid"] == true));
    }
}
#[test]
fn smbus_gaps_budgets_and_block_invariance() {
    let mut w = wave();
    for _ in 0..3 {
        packet(&mut w, 0x5a, &[7], Some(&[0xd2, 0x3a]), true, false);
    }
    let b = w.finish();
    let config = json!({"mode":"read-word"});
    for gaps in [vec![], vec![TickRange::new(Tick(180), Tick(200))]] {
        for interpreted in [false, true] {
            assert_block_invariant(|g| build(&config, interpreted, g), &b, &gaps);
        }
    }
}
