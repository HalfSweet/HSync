// SPDX-License-Identifier: Apache-2.0

//! A new stream decoder has the same observable behavior compiled or interpreted.

mod common;

use common::{as_dyn, program};
use uiharu_core::testing::Harness;
use uiharu_core::{GraphConfig, InputRef, OutputRef, Props, Tick, TickRange, TimeBase};
use uiharu_interp::{DynEvent, Interpreter, ToDyn};
use uiharu_protocols::jtag::{Jtag, JtagConfig, JtagEvent, Register, synth::JtagWaveform};
use uiharu_protocols::jtag_chain::{JtagChain, JtagChainConfig};

fn config() -> JtagChainConfig {
    JtagChainConfig {
        ir_before: 3,
        ir_bits: 4,
        ir_after: 2,
        dr_before: 2,
        dr_bits: 35,
        dr_after: 1,
    }
}
fn build(interpreted: bool, graph_config: GraphConfig) -> Harness<DynEvent> {
    Harness::build(5, TimeBase::new(1000000), graph_config, |graph, source| {
        let tap = graph.add(Jtag::new(JtagConfig::default()).expect("TAP"));
        graph.connect(source.samples, Jtag::input(tap))?;
        if interpreted {
            let bridge = graph.add(ToDyn::<JtagEvent>::new(
                Props::new().with("tdi", true).with("tdo", true),
            ));
            graph.connect(Jtag::events(tap), InputRef::of_node(bridge, 0))?;
            let node = graph.add(
                Interpreter::new(
                    program("jtag-chain"),
                    &serde_json::to_value(config()).expect("config"),
                )
                .expect("interpreter"),
            );
            graph.connect(OutputRef::of_node(bridge, 0), Interpreter::bus_input(node))?;
            Ok(Interpreter::events(node))
        } else {
            let node = graph.add(JtagChain::new(config()).expect("chain"));
            graph.connect(Jtag::events(tap), JtagChain::input(node))?;
            let bridge = graph.add(ToDyn::<JtagEvent>::new(Props::new()));
            graph.connect(JtagChain::events(node), InputRef::of_node(bridge, 0))?;
            Ok(OutputRef::of_node(bridge, 0))
        }
    })
    .expect("pipeline")
}
#[test]
fn chain_selection_and_partial_scan_commit_match() {
    let mut wave = JtagWaveform::new(JtagConfig::default(), 5, 2, Tick::ZERO);
    wave.idle(8);
    wave.reset();
    wave.scan(
        Register::Ir,
        &((0xau64 << 3) | 5).to_le_bytes(),
        &(1u64 << 3).to_le_bytes(),
        9,
    );
    wave.scan(
        Register::Dr,
        &(4u64 << 2).to_le_bytes(),
        &(2u64 << 2).to_le_bytes(),
        38,
    );
    wave.scan(
        Register::Ir,
        &((0xbu64 << 3) | 5).to_le_bytes(),
        &(1u64 << 3).to_le_bytes(),
        8,
    );
    wave.idle(8);
    let block = wave.finish();
    for gaps in [vec![], vec![TickRange::new(Tick(170), Tick(200))]] {
        for config in [
            GraphConfig::default(),
            GraphConfig {
                queue_capacity: 1,
                budget: 3,
            },
        ] {
            let mut compiled = build(false, config);
            let mut interpreted = build(true, config);
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
