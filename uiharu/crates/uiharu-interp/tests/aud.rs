// SPDX-License-Identifier: Apache-2.0
//! Independent AUD branch address vectors and bounded recovery.
mod common;
use common::program;
use serde_json::{Value, json};
use uiharu_core::testing::{Harness, assert_block_invariant, without_progress};
use uiharu_core::{
    GraphConfig, InputRef, Levels, LogicBlock, LogicBlockBuilder, OutputRef, Props, Tick,
    TickRange, TimeBase,
};
use uiharu_interp::{DynEvent, Interpreter, ToDyn};
use uiharu_protocols::aud::{Aud, Config, Event};
fn build(interpreted: bool, limits: GraphConfig) -> Harness<DynEvent> {
    Harness::build(6, TimeBase::new(1_000_000), limits, |g, s| {
        if interpreted {
            let n = g.add(Interpreter::new(program("aud"), &json!({})).expect("AUD"));
            g.connect(s.samples, Interpreter::logic_input(n))?;
            return Ok(Interpreter::events(n));
        }
        let n = g.add(Aud::new(Config::default()).expect("AUD"));
        g.connect(s.samples, Aud::input(n))?;
        let b = g.add(ToDyn::<Event>::new(Props::new()));
        g.connect(Aud::events(n), InputRef::of_node(b, 0))?;
        Ok(OutputRef::of_node(b, 0))
    })
    .expect("AUD graph")
}
fn symbol(b: &mut LogicBlockBuilder, sync: bool, value: u8) {
    let l = ((value as u64) << 2) | if sync { 2 } else { 0 };
    b.push_repeated(Levels(l), 2);
    b.push_repeated(Levels(l | 1), 2);
}
fn trace() -> LogicBlock {
    let mut b = LogicBlockBuilder::new(Tick::ZERO, 6);
    b.push_repeated(Levels(2), 8);
    symbol(&mut b, true, 8);
    symbol(&mut b, false, 1); // Unknown high address bits must remain unknown.
    symbol(&mut b, true, 11);
    for n in [8, 7, 6, 5, 4, 3, 2, 1] {
        symbol(&mut b, false, n);
    } // 0x12345678
    symbol(&mut b, true, 9);
    for n in [0xb, 0xa] {
        symbol(&mut b, false, n);
    } // 0x123456ab
    symbol(&mut b, true, 8);
    symbol(&mut b, false, 1);
    symbol(&mut b, false, 2); // Overlong compressed destination loses context.
    symbol(&mut b, true, 8);
    symbol(&mut b, false, 3);
    symbol(&mut b, true, 10);
    symbol(&mut b, false, 4); // Short then unfinished.
    symbol(&mut b, true, 9);
    symbol(&mut b, false, 5);
    b.finish().expect("AUD waveform")
}
#[test]
fn aud_destinations_corruption_and_partial_context() {
    let block = trace();
    let mut h = build(false, GraphConfig::default());
    h.feed(&block, &[u64::MAX]);
    h.end();
    let e: Vec<Value> = h.events().iter().map(|e| e.payload.0.clone()).collect();
    let addresses: Vec<_> = e
        .iter()
        .filter(|v| v["kind"] == "address")
        .map(|v| v["value"].clone())
        .collect();
    assert_eq!(
        addresses,
        vec![
            Value::Null,
            json!(0x12345678u32),
            json!(0x123456abu32),
            Value::Null
        ]
    );
    assert!(e.iter().any(|v| v["reason"] == "overlong"));
    assert!(e.iter().any(|v| v["reason"] == "incomplete"));
    assert!(e.iter().any(|v| v["reason"] == "aborted"));
}
#[test]
fn aud_scheduling_gaps_and_interpreter_match() {
    let block = trace();
    for gaps in [vec![], vec![TickRange::new(Tick(25), Tick(34))]] {
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
