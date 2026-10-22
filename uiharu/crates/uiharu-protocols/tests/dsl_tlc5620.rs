// SPDX-License-Identifier: Apache-2.0
//! TI SLAS081E LOAD/LDAC timing and ideal output-transfer vectors.
mod dsl_spi_support;
use serde_json::{Value, json};
use uiharu_core::testing::{Harness, assert_block_invariant};
use uiharu_core::{
    GraphConfig, Item, Levels, LogicBlock, LogicBlockBuilder, Tick, TickRange, TimeBase,
};
use uiharu_dsl_protocols::tlc5620::{Config, DeviceEvent, Tlc5620};
struct Wire {
    b: LogicBlockBuilder,
    levels: u64,
}
impl Wire {
    fn new() -> Self {
        Self {
            b: LogicBlockBuilder::new(Tick::ZERO, 8),
            levels: 13,
        }
    }
    fn hold(&mut self) {
        self.b.push_repeated(Levels(self.levels), 2)
    }
    fn set(&mut self, c: u8, high: bool) {
        if high {
            self.levels |= 1 << c
        } else {
            self.levels &= !(1 << c)
        }
    }
    fn word(&mut self, value: u16, bits: u8) {
        for bit in (0..bits).rev() {
            self.set(1, value & (1 << bit) != 0);
            self.set(0, true);
            self.hold();
            self.set(0, false);
            self.hold();
            self.set(0, true);
            self.hold()
        }
    }
    fn load(&mut self) {
        self.set(2, false);
        self.hold();
        self.set(2, true);
        self.hold()
    }
    fn ldac(&mut self) {
        self.set(3, false);
        self.hold();
        self.set(3, true);
        self.hold()
    }
    fn finish(self) -> LogicBlock {
        self.b.finish().expect("waveform")
    }
}
fn config() -> Config {
    serde_json::from_value(json!({"clk":0,"data":1,"load":2,"ldac":3,"vref_a_microvolts":2500000,"vref_b_microvolts":1000000,"vref_c_microvolts":3300000,"vref_d_microvolts":1800000})).expect("config")
}
fn harness(c: Config, g: GraphConfig) -> Harness<DeviceEvent> {
    Harness::build(8, TimeBase::new(1_000_000), g, |graph, source| {
        let n = graph.add(Tlc5620::new(c).expect("TLC"));
        graph.connect(source.samples, Tlc5620::input(n))?;
        Ok(Tlc5620::events(n))
    })
    .expect("graph")
}
fn events(h: &Harness<DeviceEvent>) -> Vec<Value> {
    h.items
        .iter()
        .filter_map(|i| {
            if let Item::Event(e) = i {
                Some(serde_json::to_value(e.payload).expect("event"))
            } else {
                None
            }
        })
        .collect()
}
#[test]
fn deferred_loads_update_all_four_dacs_with_independent_references() {
    let mut w = Wire::new();
    w.hold();
    for (word, bits) in [
        (0xf080, 16),
        ((1 << 9) | (1 << 8) | 128, 11),
        (2 << 9, 11),
        ((3 << 9) | (1 << 8) | 255, 11),
    ] {
        w.word(word, bits);
        w.load()
    }
    w.ldac();
    let block = w.finish();
    let c = config();
    let g = GraphConfig {
        queue_capacity: 1,
        budget: 1,
    };
    let mut h = harness(c, g);
    h.feed(&block, &[1, 9]);
    h.end();
    let out = events(&h);
    let updates = out
        .iter()
        .filter(|v| v["kind"] == "voltage-update")
        .collect::<Vec<_>>();
    assert_eq!(updates.len(), 4);
    for (channel, value) in [1250000, 1000000, 0, 3585937].into_iter().enumerate() {
        assert_eq!(updates[channel]["microvolts"], value);
        assert_eq!(updates[channel]["channel"], channel)
    }
    assert!(
        out.iter()
            .any(|v| v["kind"] == "update-all" && v["knownMask"] == 15)
    );
    h.assert_progress_promise();
    dsl_spi_support::logic_parity(
        "tlc5620",
        serde_json::to_value(c).expect("config"),
        g,
        &block,
        &[],
        &[1, 9],
        &h.items,
    );
}
#[test]
fn absent_ldac_explicit_low_assumption_updates_on_load() {
    let mut w = Wire::new();
    w.hold();
    w.word(128, 11);
    w.load();
    let block = w.finish();
    let mut c = config();
    c.ldac = None;
    c.ldac_low_when_absent = true;
    let mut h = harness(c, GraphConfig::default());
    h.feed(&block, &[3]);
    h.end();
    assert!(
        events(&h)
            .iter()
            .any(|v| v["kind"] == "voltage-update" && v["microvolts"] == 1250000)
    );
}
#[test]
fn no_load_requires_explicit_alignment_and_never_claims_output_latch() {
    let mut w = Wire::new();
    w.hold();
    w.word(128, 11);
    let block = w.finish();
    let mut c = config();
    c.load = None;
    c.ldac = None;
    c.word_aligned = true;
    let mut h = harness(c, GraphConfig::default());
    h.feed(&block, &[1]);
    h.end();
    let out = events(&h);
    assert!(
        out.iter()
            .any(|v| v["kind"] == "dac" && v["latched"] == false)
    );
    assert!(!out.iter().any(|v| v["kind"] == "voltage-update"));
    c.word_aligned = false;
    assert!(Tlc5620::new(c).is_err());
    assert!(
        uiharu_interp::Interpreter::new(
            dsl_spi_support::program("tlc5620"),
            &serde_json::to_value(c).expect("config"),
        )
        .is_err()
    );
}
#[test]
fn partial_word_gap_and_schedule_are_not_completed_latches() {
    let mut w = Wire::new();
    w.hold();
    w.word(0x55, 10);
    w.load();
    w.word(128, 11);
    w.load();
    w.ldac();
    let block = w.finish();
    let c = config();
    assert_block_invariant(
        |g| harness(c, g),
        &block,
        &[TickRange::new(Tick(81), Tick(85))],
    );
    let mut h = harness(c, GraphConfig::default());
    h.feed(&block, &[1]);
    h.end();
    assert!(
        events(&h)
            .iter()
            .any(|v| v["kind"] == "end" && v["reason"] == "incomplete")
    );
}
#[test]
fn a_gap_does_not_reassert_the_capture_start_alignment_assumption() {
    let mut w = Wire::new();
    w.hold();
    w.word(0x15, 5);
    w.word(0x123, 11);
    w.word(0x456, 11);
    let block = w.finish();
    let mut c = config();
    c.load = None;
    c.ldac = None;
    c.word_aligned = true;
    let gaps = [TickRange::new(Tick(20), Tick(24))];
    let g = GraphConfig {
        queue_capacity: 1,
        budget: 1,
    };
    let mut h = harness(c, g);
    h.feed_with_gaps(&block, &gaps, &[1, 13]);
    h.end();
    assert!(!events(&h).iter().any(|event| event["kind"] == "dac"));
    dsl_spi_support::logic_parity(
        "tlc5620",
        serde_json::to_value(c).expect("config"),
        g,
        &block,
        &gaps,
        &[1, 13],
        &h.items,
    );
    assert_block_invariant(|g| harness(c, g), &block, &gaps);
}
