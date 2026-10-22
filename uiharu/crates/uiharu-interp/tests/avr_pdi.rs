// SPDX-License-Identifier: Apache-2.0
//! XMEGA PDI character/instruction vectors, repeats without buffering and context loss.
mod common;
use common::program;
use serde_json::{Value, json};
use uiharu_core::testing::{Harness, assert_block_invariant, without_progress};
use uiharu_core::{
    GraphConfig, InputRef, Levels, LogicBlock, LogicBlockBuilder, OutputRef, Props, Tick,
    TickRange, TimeBase,
};
use uiharu_interp::{DynEvent, Interpreter, ToDyn};
use uiharu_protocols::avr_pdi::{AvrPdi, Config, Event};
fn build(interpreted: bool, limits: GraphConfig) -> Harness<DynEvent> {
    Harness::build(2, TimeBase::new(1_000_000), limits, |g, s| {
        if interpreted {
            let n = g.add(Interpreter::new(program("avr-pdi"), &json!({})).expect("PDI"));
            g.connect(s.samples, Interpreter::logic_input(n))?;
            return Ok(Interpreter::events(n));
        }
        let n = g.add(AvrPdi::new(Config::default()).expect("PDI"));
        g.connect(s.samples, AvrPdi::input(n))?;
        let b = g.add(ToDyn::<Event>::new(Props::new()));
        g.connect(AvrPdi::events(n), InputRef::of_node(b, 0))?;
        Ok(OutputRef::of_node(b, 0))
    })
    .expect("PDI graph")
}
fn bit(b: &mut LogicBlockBuilder, value: bool) {
    let l = if value { 2 } else { 0 };
    b.push_repeated(Levels(l), 2);
    b.push_repeated(Levels(l | 1), 2);
}
fn byte(b: &mut LogicBlockBuilder, value: u8, bad: bool) {
    bit(b, false);
    for i in 0..8 {
        bit(b, value & (1 << i) != 0);
    }
    bit(b, !value.count_ones().is_multiple_of(2) ^ bad);
    bit(b, true);
    bit(b, true);
}
fn bytes(b: &mut LogicBlockBuilder, values: &[u8]) {
    for &v in values {
        byte(b, v, false);
    }
}
fn break_(b: &mut LogicBlockBuilder) {
    for _ in 0..13 {
        bit(b, false);
    }
    bit(b, true);
    bit(b, true);
}
fn trace() -> LogicBlock {
    let mut b = LogicBlockBuilder::new(Tick::ZERO, 2);
    b.push_repeated(Levels(2), 8);
    break_(&mut b);
    bytes(
        &mut b,
        &[
            0x6b, 0x78, 0x56, 0x34, 0x12, 0x64, 0xab, 0xa0, 2, 0x24, 0x11, 0x22, 0x33, 0xc1, 0x59,
            0x80, 2,
        ],
    );
    bytes(
        &mut b,
        &[
            0x0c, 0x78, 0x56, 0x34, 0x12, 0xa5, 0x4c, 0x78, 0x56, 0x34, 0x12, 0x5a,
        ],
    );
    bytes(
        &mut b,
        &[0xe0, 0xff, 0x88, 0xd8, 0xcd, 0x45, 0xab, 0x89, 0x12],
    );
    byte(&mut b, 0xc1, true);
    bytes(&mut b, &[0xc1, 0x59]); // Characters remain raw until BREAK after parity error.
    break_(&mut b);
    bytes(&mut b, &[0xa3, 0xff, 0xff, 0xff, 0xff, 0x60, 1]); // 2^32 repetitions, no allocation; EOF aborts remaining bytes.
    b.finish().expect("PDI waveform")
}
#[test]
fn pdi_instructions_repeat_key_pointer_and_error_context() {
    let block = trace();
    let mut h = build(false, GraphConfig::default());
    h.feed(&block, &[u64::MAX]);
    h.end();
    let e: Vec<Value> = h.events().iter().map(|e| e.payload.0.clone()).collect();
    for name in ["lds", "ld", "sts", "st", "ldcs", "repeat", "stcs", "key"] {
        assert!(e.iter().any(|v| v["command"] == name), "{name}: {e:?}");
    }
    let repeated: Vec<_> = e
        .iter()
        .filter(|v| {
            v["kind"] == "value" && [json!(0x11), json!(0x22), json!(0x33)].contains(&v["value"])
        })
        .map(|v| v["address"].clone())
        .collect();
    assert_eq!(
        repeated,
        vec![
            json!(0x12345679u32),
            json!(0x1234567au32),
            json!(0x1234567bu32)
        ]
    );
    assert_eq!(
        e.iter()
            .filter(|v| v["kind"] == "operand-byte" && v["role"] == "key")
            .count(),
        8
    );
    assert!(
        e.iter()
            .any(|v| v["kind"] == "instruction" && v["repeat"] == json!(4294967296u64))
    );
    assert!(
        e.iter()
            .any(|v| v["kind"] == "character" && v["parityOk"] == false)
    );
    assert!(e.iter().any(|v| v["reason"] == "aborted"));
}
#[test]
fn pdi_gaps_blocks_pause_and_interpreter_match() {
    let block = trace();
    for gaps in [vec![], vec![TickRange::new(Tick(210), Tick(230))]] {
        for limits in [
            GraphConfig::default(),
            GraphConfig {
                queue_capacity: 1,
                budget: 3,
            },
        ] {
            let mut c = build(false, limits);
            let mut i = build(true, limits);
            c.feed_cut(&block, &gaps, &[1, 17, 211], &[0, 1, 7]);
            i.feed_cut(&block, &gaps, &[1, 17, 211], &[0, 1, 7]);
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
