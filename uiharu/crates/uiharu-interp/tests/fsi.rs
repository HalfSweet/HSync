// SPDX-License-Identifier: Apache-2.0
//! OpenFSI independent bit-division CRC vectors and interlocked transactions.
mod common;
use common::program;
use serde_json::{Value, json};
use uiharu_core::testing::{Harness, assert_block_invariant, without_progress};
use uiharu_core::{
    GraphConfig, InputRef, Levels, LogicBlock, LogicBlockBuilder, OutputRef, Props, Tick,
    TickRange, TimeBase,
};
use uiharu_interp::{DynEvent, Interpreter, ToDyn};
use uiharu_protocols::fsi::{Config, Event, Fsi};
fn build(interpreted: bool, limits: GraphConfig) -> Harness<DynEvent> {
    Harness::build(2, TimeBase::new(1_000_000), limits, |g, s| {
        if interpreted {
            let n = g.add(
                Interpreter::new(program("fsi"), &json!({"response_timeout":16})).expect("FSI"),
            );
            g.connect(s.samples, Interpreter::logic_input(n))?;
            return Ok(Interpreter::events(n));
        }
        let n = g.add(
            Fsi::new(Config {
                response_timeout: 16,
                ..Config::default()
            })
            .expect("FSI"),
        );
        g.connect(s.samples, Fsi::input(n))?;
        let b = g.add(ToDyn::<Event>::new(Props::new()));
        g.connect(Fsi::events(n), InputRef::of_node(b, 0))?;
        Ok(OutputRef::of_node(b, 0))
    })
    .expect("FSI graph")
}
fn bits(v: &mut Vec<bool>, n: u32, count: usize) {
    for i in (0..count).rev() {
        v.push(n & (1 << i) != 0);
    }
}
fn crc(payload: &[bool]) -> Vec<bool> {
    let mut division = payload.to_vec();
    division.extend([false; 4]);
    for i in 0..payload.len() {
        if division[i] {
            for (j, p) in [true, false, true, true, true].into_iter().enumerate() {
                division[i + j] ^= p;
            }
        }
    }
    division[payload.len()..].to_vec()
}
fn host(b: &mut LogicBlockBuilder, bit: bool) {
    let data = if bit { 0 } else { 2 };
    b.push_repeated(Levels(data), 2);
    b.push_repeated(Levels(data | 1), 2);
}
fn slave(b: &mut LogicBlockBuilder, bit: bool) {
    let data = if bit { 0 } else { 2 };
    b.push_repeated(Levels(data | 1), 2);
    b.push_repeated(Levels(data), 2);
}
fn request(b: &mut LogicBlockBuilder, payload: &[bool], bad: bool) {
    let mut packet = payload.to_vec();
    packet.extend(crc(payload));
    if bad {
        let last = packet.len() - 1;
        packet[last] = !packet[last];
    }
    for bit in packet {
        host(b, bit);
    }
    for _ in 0..3 {
        host(b, false);
    }
}
fn response(b: &mut LogicBlockBuilder, payload: &[bool], bad: bool) {
    let mut packet = payload.to_vec();
    packet.extend(crc(payload));
    if bad {
        let last = packet.len() - 1;
        packet[last] = !packet[last];
    }
    for bit in packet {
        slave(b, bit);
    }
    for _ in 0..3 {
        slave(b, false);
    }
}
fn access(id: u32, code: u32, read: bool, address: u32, size: bool, data: &[u8]) -> Vec<bool> {
    let mut v = vec![true];
    bits(&mut v, id, 2);
    bits(&mut v, code, if code == 3 { 2 } else { 3 });
    v.push(read);
    if code == 5 {
        v.push(false);
    }
    bits(
        &mut v,
        address,
        if code == 4 {
            21
        } else if code == 5 {
            8
        } else {
            2
        },
    );
    v.push(size);
    for &d in data {
        bits(&mut v, d.into(), 8);
    }
    v
}
fn poll(id: u32, code: u32) -> Vec<bool> {
    let mut v = vec![true];
    bits(&mut v, id, 2);
    bits(&mut v, code, 3);
    v
}
fn reply(id: u32, status: u32, data: &[u8]) -> Vec<bool> {
    let mut v = vec![true];
    bits(&mut v, id, 2);
    bits(&mut v, status, 2);
    for &d in data {
        bits(&mut v, d.into(), 8);
    }
    v
}
fn trace() -> LogicBlock {
    let mut b = LogicBlockBuilder::new(Tick::ZERO, 2);
    b.push_repeated(Levels(2), 8);
    for _ in 0..256 {
        host(&mut b, true);
    }
    for _ in 0..4 {
        host(&mut b, false);
    }
    request(
        &mut b,
        &access(2, 4, false, 0x101, true, &[0x11, 0x22, 0x33, 0x44]),
        false,
    );
    response(&mut b, &reply(2, 1, &[]), false); // BUSY preserves a pending write.
    request(&mut b, &poll(2, 2), false);
    response(&mut b, &reply(2, 0, &[]), false);
    request(&mut b, &access(2, 5, true, 4, false, &[]), false);
    response(&mut b, &reply(2, 0, &[0xa5]), false);
    request(&mut b, &access(2, 3, true, 2, true, &[]), false);
    response(&mut b, &reply(2, 0, &[0x12, 0x34]), false);
    let mut terminate = vec![true];
    bits(&mut terminate, 2, 2);
    bits(&mut terminate, 0x3f, 6);
    request(&mut b, &terminate, false);
    response(&mut b, &reply(2, 0, &[]), false);
    request(&mut b, &poll(2, 1), false);
    let mut irq = vec![true];
    bits(&mut irq, 2, 2);
    irq.push(false);
    bits(&mut irq, 2, 2);
    bits(&mut irq, 5, 3);
    response(&mut b, &irq, false);
    request(&mut b, &access(2, 4, true, 0x200, false, &[]), true);
    response(&mut b, &reply(2, 3, &[]), false);
    request(&mut b, &access(2, 3, true, 0, false, &[]), false);
    response(&mut b, &reply(2, 0, &[0x7f]), false); // Unknown base stays unknown.
    request(&mut b, &access(2, 4, true, 0x300, false, &[]), false);
    response(&mut b, &reply(2, 0, &[0x55]), true);
    request(&mut b, &access(2, 4, true, 0x400, false, &[]), false);
    for _ in 0..16 {
        slave(&mut b, false);
    } // Timeout loses alignment.
    b.finish().expect("FSI waveform")
}
#[test]
fn fsi_crc_address_busy_poll_interrupt_and_failures() {
    let block = trace();
    let mut h = build(false, GraphConfig::default());
    h.feed(&block, &[u64::MAX]);
    h.end();
    let e: Vec<Value> = h.events().iter().map(|e| e.payload.0.clone()).collect();
    assert!(e.iter().any(|v| v["kind"] == "break"));
    for command in [
        "absolute",
        "relative",
        "same",
        "data-poll",
        "interrupt-poll",
        "terminate",
    ] {
        assert!(
            e.iter().any(|v| v["command"] == command),
            "{command}: {e:?}"
        );
    }
    assert!(
        e.iter()
            .any(|v| v["command"] == "relative" && v["address"] == 260)
    );
    assert!(
        e.iter()
            .any(|v| v["command"] == "same" && v["address"] == 262)
    );
    assert!(
        e.iter()
            .any(|v| v["command"] == "same" && v["address"].is_null())
    );
    assert!(
        e.iter()
            .any(|v| v["kind"] == "interrupt" && v["levels"] == 2 && v["dma"] == 5)
    );
    assert!(e.iter().any(|v| v["success"] == true));
    for reason in ["busy", "bad-crc", "timeout"] {
        assert!(e.iter().any(|v| v["reason"] == reason), "{reason}: {e:?}");
    }
}
#[test]
fn fsi_gap_block_budget_pause_and_interpreter_match() {
    let block = trace();
    for gaps in [vec![], vec![TickRange::new(Tick(1170), Tick(1190))]] {
        for limits in [
            GraphConfig::default(),
            GraphConfig {
                queue_capacity: 1,
                budget: 3,
            },
        ] {
            let mut c = build(false, limits);
            let mut i = build(true, limits);
            c.feed_cut(&block, &gaps, &[1, 31, 211], &[0, 1, 7]);
            i.feed_cut(&block, &gaps, &[1, 31, 211], &[0, 1, 7]);
            c.end();
            i.end();
            c.assert_progress_promise();
            i.assert_progress_promise();
            assert_eq!(without_progress(&c.items), without_progress(&i.items));
        }
        for interpreted in [false, true] {
            assert_block_invariant(|g| build(interpreted, g), &block, &gaps);
        }
    }
}
