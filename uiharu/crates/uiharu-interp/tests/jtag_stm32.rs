// SPDX-License-Identifier: Apache-2.0

//! A new stream decoder has the same observable behavior compiled or interpreted.

mod common;

use common::{as_dyn, program};
use uiharu_core::testing::Harness;
use uiharu_core::{GraphConfig, InputRef, OutputRef, Props, Tick, TickRange, TimeBase};
use uiharu_interp::{DynEvent, Interpreter, ToDyn};
use uiharu_protocols::jtag::{Jtag, JtagConfig, JtagEvent, Register, synth::JtagWaveform};
use uiharu_protocols::jtag_stm32::{JtagStm32, JtagStm32Config, JtagStm32Event};

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
                Interpreter::new(program("jtag-stm32"), &serde_json::json!({}))
                    .expect("interpreter"),
            );
            graph.connect(OutputRef::of_node(bridge, 0), Interpreter::bus_input(node))?;
            Ok(Interpreter::events(node))
        } else {
            let node = graph.add(JtagStm32::new(JtagStm32Config {}).expect("STM32"));
            graph.connect(Jtag::events(tap), JtagStm32::input(node))?;
            let bridge = graph.add(ToDyn::<JtagStm32Event>::new(Props::new()));
            graph.connect(JtagStm32::events(node), InputRef::of_node(bridge, 0))?;
            Ok(OutputRef::of_node(bridge, 0))
        }
    })
    .expect("pipeline")
}
#[test]
fn stm32_chain_context_and_errors_match() {
    let mut wave = JtagWaveform::new(JtagConfig::default(), 5, 2, Tick::ZERO);
    wave.idle(8);
    wave.reset();
    wave.scan(
        Register::Dr,
        &[0; 8],
        &0x164100413ba00477u64.to_le_bytes(),
        64,
    );
    for (ir, input, output, bits) in [
        (0x1fau16, 7u64, 2u64, 36),
        (0x1fb, 7, 1, 36),
        (0x1fb, 7, 0x112233442, 36),
        (0x1f8, 8, 0, 36),
        (0x1fe, 0, 0x3ba00477, 33),
        (0x1fe, 0, 0, 32),
    ] {
        wave.scan(Register::Ir, &ir.to_le_bytes(), &[0x11, 0], 9);
        wave.scan(
            Register::Dr,
            &input.to_le_bytes(),
            &output.to_le_bytes(),
            bits,
        );
    }
    wave.idle(8);
    let block = wave.finish();
    for gaps in [vec![], vec![TickRange::new(Tick(450), Tick(480))]] {
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
