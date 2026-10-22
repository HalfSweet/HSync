// SPDX-License-Identifier: Apache-2.0

//! A new stream decoder has the same observable behavior compiled or interpreted.

mod common;

use common::{as_dyn, program};
use uiharu_core::testing::Harness;
use uiharu_core::{GraphConfig, InputRef, OutputRef, Props, Tick, TickRange, TimeBase};
use uiharu_interp::{DynEvent, Interpreter, ToDyn};
use uiharu_protocols::swim::{Direction, SwimConfig, synth::SwimWaveform};
use uiharu_protocols::swim_stm8::{Speed, SwimEvent, SwimStm8, SwimStm8Config};
fn build(
    interpreted: bool,
    options: SwimStm8Config,
    graph_config: GraphConfig,
) -> Harness<DynEvent> {
    Harness::build(
        1,
        TimeBase::new(16_000_000),
        graph_config,
        |graph, source| {
            if interpreted {
                let node = graph.add(
                    Interpreter::new(
                        program("swim-stm8"),
                        &serde_json::to_value(options).expect("config"),
                    )
                    .expect("interpreter"),
                );
                graph.connect(source.samples, Interpreter::logic_input(node))?;
                Ok(Interpreter::events(node))
            } else {
                let node = graph.add(SwimStm8::new(options).expect("SWIM"));
                graph.connect(source.samples, SwimStm8::input(node))?;
                let bridge = graph.add(ToDyn::<SwimEvent>::new(Props::new()));
                graph.connect(SwimStm8::events(node), InputRef::of_node(bridge, 0))?;
                Ok(OutputRef::of_node(bridge, 0))
            }
        },
    )
    .expect("pipeline")
}
#[test]
fn swim_calibration_packets_retries_and_cuts_match() {
    for calibrate_sync in [false, true] {
        let options = SwimStm8Config {
            data: 0,
            clock_hz: 8_000_000,
            speed: Speed::Low,
            start_active: false,
            calibrate_sync,
        };
        let mut wave = SwimWaveform::new(
            SwimConfig::default(),
            1,
            TimeBase::new(16_000_000),
            Tick::ZERO,
        );
        wave.idle(32);
        wave.entry();
        wave.reset();
        wave.command(2, true);
        for value in [2, 0, 0x7f, 0x80] {
            wave.byte(Direction::HostToTarget, value, true)
        }
        wave.byte(Direction::HostToTarget, 0x10, false);
        wave.byte(Direction::HostToTarget, 0x10, true);
        wave.byte(Direction::HostToTarget, 0, true);
        wave.set_speed(uiharu_protocols::swim::Speed::High);
        wave.command(0, true);
        wave.idle(32);
        let block = wave.finish();
        for gaps in [vec![], vec![TickRange::new(Tick(300), Tick(360))]] {
            for config in [
                GraphConfig::default(),
                GraphConfig {
                    queue_capacity: 1,
                    budget: 3,
                },
            ] {
                let mut compiled = build(false, options, config);
                let mut interpreted = build(true, options, config);
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
