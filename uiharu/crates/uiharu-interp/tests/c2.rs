// SPDX-License-Identifier: Apache-2.0
//! AN127 transport vectors, variable WAIT and bounded reset recovery.
mod common;
use common::program;
use serde_json::{Value, json};
use uiharu_core::testing::{Harness, assert_block_invariant, without_progress};
use uiharu_core::{
    GraphConfig, InputRef, Levels, LogicBlock, LogicBlockBuilder, OutputRef, Props, Tick,
    TickRange, TimeBase,
};
use uiharu_interp::{DynEvent, Interpreter, ToDyn};
use uiharu_protocols::c2::{C2, Config, Event};
fn build(interpreted: bool, limits: GraphConfig) -> Harness<DynEvent> {
    Harness::build(2, TimeBase::new(1_000_000), limits, |g, s| {
        if interpreted {
            let n =
                g.add(Interpreter::new(program("c2"), &json!({"max_wait_cycles":3})).expect("C2"));
            g.connect(s.samples, Interpreter::logic_input(n))?;
            return Ok(Interpreter::events(n));
        }
        let n = g.add(
            C2::new(Config {
                max_wait_cycles: 3,
                ..Config::default()
            })
            .expect("C2"),
        );
        g.connect(s.samples, C2::input(n))?;
        let b = g.add(ToDyn::<Event>::new(Props::new()));
        g.connect(C2::events(n), InputRef::of_node(b, 0))?;
        Ok(OutputRef::of_node(b, 0))
    })
    .expect("C2 graph")
}
fn bit(b: &mut LogicBlockBuilder, value: bool) {
    let l = if value { 2 } else { 0 };
    b.push_repeated(Levels(l), 2);
    b.push_repeated(Levels(l | 1), 2);
}
fn bits(b: &mut LogicBlockBuilder, value: u8, count: u8) {
    for i in 0..count {
        bit(b, value & (1 << i) != 0);
    }
}
fn reset(b: &mut LogicBlockBuilder) {
    b.push_repeated(Levels(0), 25);
    b.push_repeated(Levels(1), 2);
}
fn trace() -> LogicBlock {
    let mut b = LogicBlockBuilder::new(Tick::ZERO, 2);
    b.push_repeated(Levels(3), 8);
    reset(&mut b);
    bit(&mut b, true);
    bits(&mut b, 0, 2);
    bits(&mut b, 1, 2);
    bits(&mut b, 0, 2);
    bit(&mut b, true);
    bits(&mut b, 0xa5, 8);
    bits(&mut b, 0x5a, 8);
    bit(&mut b, true);
    bit(&mut b, true);
    bits(&mut b, 3, 2);
    bits(&mut b, 0x2a, 8);
    bit(&mut b, true);
    bit(&mut b, true);
    bits(&mut b, 2, 2);
    bits(&mut b, 0xf0, 8);
    bit(&mut b, true);
    bit(&mut b, true);
    bits(&mut b, 1, 2);
    bits(&mut b, 0, 2);
    bits(&mut b, 0x55, 8);
    bit(&mut b, false);
    bit(&mut b, true);
    bit(&mut b, true);
    bit(&mut b, true);
    bits(&mut b, 0, 2);
    bits(&mut b, 0, 2);
    bits(&mut b, 0, 3); // Wait limit loses synchronization.
    reset(&mut b);
    bit(&mut b, true);
    bits(&mut b, 1, 2);
    bits(&mut b, 0, 2);
    bits(&mut b, 3, 3); // Truncated data.
    b.finish().expect("C2 waveform")
}
#[test]
fn c2_instruction_data_wait_reset_and_truncation() {
    let block = trace();
    let mut h = build(false, GraphConfig::default());
    h.feed(&block, &[u64::MAX]);
    h.end();
    let e: Vec<Value> = h.events().iter().map(|e| e.payload.0.clone()).collect();
    assert_eq!(
        e.iter()
            .filter(|v| v["kind"] == "byte")
            .map(|v| v["value"].clone())
            .collect::<Vec<_>>(),
        vec![
            json!(0xa5),
            json!(0x5a),
            json!(0x2a),
            json!(0xf0),
            json!(0x55)
        ]
    );
    assert!(
        e.iter()
            .any(|v| v["kind"] == "wait" && v["cycles"] == 3 && v["ready"] == true)
    );
    assert!(e.iter().any(|v| v["reason"] == "wait-limit"));
    assert!(e.iter().any(|v| v["reason"] == "aborted"));
}
#[test]
fn c2_block_budget_pause_gap_and_interpreter_match() {
    let block = trace();
    for gaps in [vec![], vec![TickRange::new(Tick(55), Tick(67))]] {
        for limits in [
            GraphConfig::default(),
            GraphConfig {
                queue_capacity: 1,
                budget: 3,
            },
        ] {
            let mut c = build(false, limits);
            let mut i = build(true, limits);
            c.feed_cut(&block, &gaps, &[1, 7, 31], &[0, 1, 7]);
            i.feed_cut(&block, &gaps, &[1, 7, 31], &[0, 1, 7]);
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
