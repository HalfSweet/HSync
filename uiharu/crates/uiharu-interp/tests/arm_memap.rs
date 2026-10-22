// SPDX-License-Identifier: Apache-2.0

//! A new stream decoder has the same observable behavior compiled or interpreted.

mod common;

use common::{as_dyn, program};
use uiharu_core::testing::Harness;
use uiharu_core::{GraphConfig, InputRef, OutputRef, Props, Tick, TickRange, TimeBase};
use uiharu_interp::{DynEvent, Interpreter, ToDyn};
use uiharu_protocols::arm_memap::{ArmMemap, ArmMemapConfig, ArmMemapEvent};
use uiharu_protocols::jtag::{Jtag, JtagConfig, synth::JtagWaveform};
use uiharu_protocols::jtag_dp::{JtagDp, JtagDpConfig, JtagDpEvent, synth};

fn build(interpreted: bool, ap: u8, graph: GraphConfig) -> Harness<DynEvent> {
    Harness::build(5, TimeBase::new(1000000), graph, |graph, source| {
        let tap = graph.add(Jtag::new(JtagConfig::default()).expect("TAP"));
        graph.connect(source.samples, Jtag::input(tap))?;
        let transport = graph.add(JtagDp::new(JtagDpConfig {}));
        graph.connect(Jtag::events(tap), JtagDp::input(transport))?;
        if interpreted {
            let bridge = graph.add(ToDyn::<JtagDpEvent>::new(Props::new()));
            graph.connect(JtagDp::events(transport), InputRef::of_node(bridge, 0))?;
            let node = graph.add(
                Interpreter::new(
                    program("arm-memap"),
                    &serde_json::json!({"ap":ap,"tar_wrap_bits":10}),
                )
                .expect("interpreter"),
            );
            graph.connect(OutputRef::of_node(bridge, 0), Interpreter::bus_input(node))?;
            Ok(Interpreter::events(node))
        } else {
            let pdu = graph.add(
                ArmMemap::new(ArmMemapConfig {
                    ap,
                    tar_wrap_bits: 10,
                })
                .expect("ITM"),
            );
            graph.connect(JtagDp::events(transport), ArmMemap::input(pdu))?;
            let bridge = graph.add(ToDyn::<ArmMemapEvent>::new(Props::new()));
            graph.connect(ArmMemap::events(pdu), InputRef::of_node(bridge, 0))?;
            Ok(OutputRef::of_node(bridge, 0))
        }
    })
    .expect("pipeline")
}

fn access(w: &mut JtagWaveform, read: bool, address: u8, value: u32, response: u32, wait: bool) {
    synth::data_register(
        w,
        (u64::from(value) << 3) | (u64::from(address >> 2) << 1) | u64::from(read),
        (u64::from(response) << 3) | if wait { 1 } else { 2 },
    );
}
#[test]
fn memap_context_waits_posted_reads_and_gaps_match() {
    let mut wave = JtagWaveform::new(JtagConfig::default(), 5, 2, Tick::ZERO);
    wave.idle(8);
    wave.reset();
    synth::instruction(&mut wave, 10);
    access(&mut wave, false, 8, 0, 0, false);
    synth::instruction(&mut wave, 11);
    access(&mut wave, false, 0, 0x12, 0, false);
    access(&mut wave, false, 4, 0x200003fc, 0, false);
    access(&mut wave, true, 12, 0, 0, false);
    access(&mut wave, true, 12, 0, 0, true);
    access(&mut wave, true, 12, 0, 0x12345678, false);
    synth::instruction(&mut wave, 10);
    access(&mut wave, true, 12, 0, 0x87654321, false);
    wave.idle(8);
    let block = wave.finish();
    for ap in [0, 1] {
        for gaps in [vec![], vec![TickRange::new(Tick(700), Tick(800))]] {
            for graph in [
                GraphConfig::default(),
                GraphConfig {
                    queue_capacity: 1,
                    budget: 3,
                },
            ] {
                let mut compiled = build(false, ap, graph);
                let mut interpreted = build(true, ap, graph);
                compiled.feed_cut(&block, &gaps, &[13, 211, 1], &[0, 1, 7]);
                compiled.end();
                interpreted.feed_cut(&block, &gaps, &[13, 211, 1], &[0, 1, 7]);
                interpreted.end();
                assert_eq!(
                    as_dyn(&compiled.without_progress()),
                    as_dyn(&interpreted.without_progress())
                );
            }
        }
    }
}
