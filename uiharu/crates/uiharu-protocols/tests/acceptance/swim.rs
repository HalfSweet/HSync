// SPDX-License-Identifier: Apache-2.0

//! SWIM decoding is independent of block boundaries, pauses, queue capacities and budgets.

use uiharu_core::testing::{Harness, assert_block_invariant};
use uiharu_core::{GraphConfig, Tick, TickRange, TimeBase};
use uiharu_protocols::swim::synth::SwimWaveform;
use uiharu_protocols::swim::{Direction, Swim, SwimConfig, SwimEvent};

fn build(graph: GraphConfig) -> Harness<SwimEvent> {
    Harness::build(1, TimeBase::new(16_000_000), graph, |graph, source| {
        let node = graph.add(Swim::new(SwimConfig::default()).expect("valid configuration"));
        graph.connect(source.samples, Swim::input(node))?;
        Ok(Swim::events(node))
    })
    .expect("builds")
}

#[test]
fn swim_retries_errors_resets_and_gaps_are_block_invariant() {
    let mut wave = SwimWaveform::new(
        SwimConfig::default(),
        1,
        TimeBase::new(16_000_000),
        Tick::ZERO,
    );
    wave.idle(12);
    wave.command(2, true);
    for value in [2, 0, 0x12, 0x34] {
        wave.byte(Direction::HostToTarget, value, true);
    }
    wave.packet(false, 0xa5, 8, true, true);
    wave.byte(Direction::HostToTarget, 0xa5, false);
    wave.byte(Direction::HostToTarget, 0xa5, true);
    wave.byte(Direction::HostToTarget, 0x55, true);
    let gap_start = wave.position().offset(8);
    wave.command(1, true);
    wave.byte(Direction::HostToTarget, 1, true);
    wave.reset();
    wave.command(0, true);
    wave.idle(32);
    let block = wave.finish();
    let gaps = [TickRange::new(gap_start, gap_start.offset(80))];
    assert_block_invariant(build, &block, &gaps);
}
