// SPDX-License-Identifier: Apache-2.0

use uiharu_core::testing::{Harness, assert_block_invariant};
use uiharu_core::{GraphConfig, Tick, TickRange, TimeBase};
use uiharu_protocols::sdcard_sd::synth::SdcardSdWaveform;
use uiharu_protocols::sdcard_sd::{SdcardSd, SdcardSdConfig, SdcardSdEvent};

#[test]
fn native_sd_command_and_read_block_streams_are_deterministic() {
    for data in [vec![], vec![2], vec![2, 3, 4, 5]] {
        let config = SdcardSdConfig {
            data,
            block_bytes: 9,
            ..Default::default()
        };
        let mut wave = SdcardSdWaveform::new(config.clone(), 6, 2, Tick::ZERO);
        wave.clocks(8);
        wave.token(true, 2, 0, false);
        wave.clocks(3);
        wave.r2([0x5a; 15]);
        wave.clocks(8);
        wave.token(true, 18, 0x1234, false);
        wave.clocks(3);
        wave.token(false, 18, 0x900, false);
        wave.clocks(8);
        if !config.data.is_empty() {
            wave.read_block(&[1, 2, 3, 4, 5, 6, 7, 8, 9]);
            wave.clocks(8);
            wave.read_block(&[0xaa; 9]);
            wave.clocks(8);
        }
        wave.token(true, 12, 0, false);
        wave.clocks(8);
        let block = wave.finish();
        let gaps = [TickRange::new(Tick(100), Tick(110))];
        assert_block_invariant(
            |graph_config: GraphConfig| {
                Harness::<SdcardSdEvent>::build(
                    6,
                    TimeBase::new(1_000_000),
                    graph_config,
                    |graph, source| {
                        let node =
                            graph.add(SdcardSd::new(config.clone()).expect("valid SD wiring"));
                        graph.connect(source.samples, SdcardSd::input(node))?;
                        Ok(SdcardSd::events(node))
                    },
                )
                .expect("native SD graph")
            },
            &block,
            &gaps,
        );
    }
}
