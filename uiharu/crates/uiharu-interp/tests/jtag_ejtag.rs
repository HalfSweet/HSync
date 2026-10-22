// SPDX-License-Identifier: Apache-2.0

//! A new stream decoder has the same observable behavior compiled or interpreted.

mod common;

use common::{as_dyn, program};
use uiharu_core::testing::Harness;
use uiharu_core::{GraphConfig, InputRef, OutputRef, Props, Tick, TickRange, TimeBase};
use uiharu_interp::{DynEvent, Interpreter, ToDyn};
use uiharu_protocols::jtag::{Jtag, JtagConfig, JtagEvent, Register, synth::JtagWaveform};
use uiharu_protocols::jtag_ejtag::{JtagEjtag, JtagEjtagConfig, JtagEjtagEvent};

fn build(interpreted: bool, width: u8, graph_config: GraphConfig) -> Harness<DynEvent> {
    Harness::build(5, TimeBase::new(1000000), graph_config, |graph, source| {
        let tap = graph.add(Jtag::new(JtagConfig::default()).expect("TAP"));
        graph.connect(source.samples, Jtag::input(tap))?;
        if interpreted {
            let bridge = graph.add(ToDyn::<JtagEvent>::new(
                Props::new().with("tdi", true).with("tdo", true),
            ));
            graph.connect(Jtag::events(tap), InputRef::of_node(bridge, 0))?;
            let node = graph.add(
                Interpreter::new(program("jtag-ejtag"), &serde_json::json!({"ir_bits":width}))
                    .expect("interpreter"),
            );
            graph.connect(OutputRef::of_node(bridge, 0), Interpreter::bus_input(node))?;
            Ok(Interpreter::events(node))
        } else {
            let node =
                graph.add(JtagEjtag::new(JtagEjtagConfig { ir_bits: width }).expect("EJTAG"));
            graph.connect(Jtag::events(tap), JtagEjtag::input(node))?;
            let bridge = graph.add(ToDyn::<JtagEjtagEvent>::new(Props::new()));
            graph.connect(JtagEjtag::events(node), InputRef::of_node(bridge, 0))?;
            Ok(OutputRef::of_node(bridge, 0))
        }
    })
    .expect("pipeline")
}
#[test]
fn ejtag_register_flags_context_and_errors_match() {
    for width in [5, 8] {
        let mut wave = JtagWaveform::new(JtagConfig::default(), 5, 2, Tick::ZERO);
        wave.idle(8);
        wave.reset();
        wave.scan(Register::Dr, &[0; 4], &0x04a07053u32.to_le_bytes(), 32);
        for (ir, input, output, bits) in [
            (10, 0x40000u64, 0xc0008u64, 32),
            (8, 0, 0xff200200, 32),
            (9, 0x3c1a1234, 0xaabbccdd, 32),
            (10, 0, 0xc0008, 32),
            (14, 0x11223344 << 1, (0x8899aabb << 1) | 1, 33),
            (10, 0, 0, 31),
        ] {
            wave.scan(Register::Ir, &[ir], &[1], u32::from(width));
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
                let mut compiled = build(false, width, config);
                let mut interpreted = build(true, width, config);
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
