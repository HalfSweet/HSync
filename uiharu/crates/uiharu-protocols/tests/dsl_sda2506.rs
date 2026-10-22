// SPDX-License-Identifier: Apache-2.0
//! SDA2506 original Siemens data-sheet instruction vectors, LSB first.
mod dsl_spi_support;
use serde_json::{Value, json};
use uiharu_core::testing::{Harness, assert_block_invariant};
use uiharu_core::{
    GraphConfig, Item, Levels, LogicBlock, LogicBlockBuilder, Tick, TickRange, TimeBase,
};
use uiharu_dsl_protocols::sda2506::{Config, Event, Sda2506};
struct Wire {
    b: LogicBlockBuilder,
    levels: u64,
}
impl Wire {
    fn new() -> Self {
        Self {
            b: LogicBlockBuilder::new(Tick::ZERO, 8),
            levels: 4,
        }
    }
    fn hold(&mut self) {
        self.b.push_repeated(Levels(self.levels), 2)
    }
    fn data(&mut self, bit: bool) {
        if bit {
            self.levels |= 2
        } else {
            self.levels &= !2
        }
    }
    fn clock(&mut self, bit: bool) {
        self.data(bit);
        self.levels &= !1;
        self.hold();
        self.levels |= 1;
        self.hold();
        self.levels &= !1;
        self.hold()
    }
    fn instruction(&mut self, value: u16, bits: u8) {
        for bit in 0..bits {
            self.clock(value & (1 << bit) != 0)
        }
    }
    fn enable(&mut self) {
        self.levels &= !4;
        self.hold()
    }
    fn disable(&mut self) {
        self.levels |= 4;
        self.hold()
    }
    fn finish(self) -> LogicBlock {
        self.b.finish().expect("valid test vector")
    }
}
fn harness(g: GraphConfig) -> Harness<Event> {
    Harness::build(8, TimeBase::new(1_000_000), g, |graph, source| {
        let n = graph.add(Sda2506::new(Config::default()).expect("valid test vector"));
        graph.connect(source.samples, Sda2506::input(n))?;
        Ok(Sda2506::events(n))
    })
    .expect("valid test vector")
}
fn events(h: &Harness<Event>) -> Vec<Value> {
    h.items
        .iter()
        .filter_map(|i| {
            if let Item::Event(e) = i {
                Some(serde_json::to_value(e.payload).expect("valid test vector"))
            } else {
                None
            }
        })
        .collect()
}
fn has(out: &[Value], wanted: Value) {
    assert!(
        out.iter().any(|v| wanted
            .as_object()
            .expect("valid test vector")
            .iter()
            .all(|(k, x)| v.get(k) == Some(x))),
        "missing {wanted}: {out:?}"
    )
}
#[test]
fn read_and_reused_program_instruction_follow_ce_sequences() {
    let mut w = Wire::new();
    w.hold();
    w.instruction(0x25, 8);
    w.enable();
    w.clock(false);
    for bit in 0..8 {
        w.clock(0xa5 & (1 << bit) != 0)
    }
    w.disable();
    // Last 16 input bits D0..D7,A0..A6,CB=1 => value5A,address25.
    w.instruction(0xa55a, 16);
    w.enable();
    w.clock(false);
    w.disable();
    w.enable();
    w.clock(true);
    w.disable();
    let block = w.finish();
    let mut h = harness(GraphConfig {
        queue_capacity: 1,
        budget: 1,
    });
    h.feed(&block, &[1, 7]);
    h.end();
    dsl_spi_support::logic_parity(
        "sda2506",
        serde_json::to_value(Config::default()).expect("valid test vector"),
        GraphConfig {
            queue_capacity: 1,
            budget: 1,
        },
        &block,
        &[],
        &[1, 7],
        &h.items,
    );
    let out = events(&h);
    has(
        &out,
        json!({"kind":"command","operation":"read","address":0x25,"reused":false}),
    );
    has(&out, json!({"kind":"data","address":0x25,"value":0xa5}));
    has(
        &out,
        json!({"kind":"command","operation":"write","address":0x25,"value":0x5a}),
    );
    has(
        &out,
        json!({"kind":"command","operation":"erase","address":0x25,"reused":true}),
    );
}
#[test]
fn gaps_drop_header_and_partial_reads_are_not_invented() {
    let mut w = Wire::new();
    w.hold();
    w.instruction(0x25, 8);
    w.enable();
    w.clock(false);
    w.clock(true);
    w.disable();
    let block = w.finish();
    assert_block_invariant(harness, &block, &[TickRange::new(Tick(13), Tick(17))]);
    let mut h = harness(GraphConfig::default());
    h.feed(&block, &[2]);
    h.end();
    assert!(!events(&h).iter().any(|v| v["kind"] == "data"));
}
