// SPDX-License-Identifier: Apache-2.0

use proptest::prelude::*;
use uiharu_core::testing::{Harness, assert_block_invariant};
use uiharu_core::{GraphConfig, Tick, TickRange, TimeBase};
use uiharu_protocols::jtag::synth::JtagWaveform;
use uiharu_protocols::jtag::{Jtag, JtagConfig};
use uiharu_protocols::jtag_dp::{JtagDp, JtagDpConfig, JtagDpEvent, synth};

proptest! {
    #![proptest_config(ProptestConfig::with_cases(16))]
    #[test]
    fn scans_waits_and_gaps_are_block_invariant(accesses in prop::collection::vec((any::<bool>(),0u8..4,any::<u32>(),prop::sample::select(vec![1u8,2,7])),1..16)) {
        let mut wave=JtagWaveform::new(JtagConfig::default(),5,1,Tick::ZERO);
        wave.idle(3);wave.reset();synth::instruction(&mut wave,0xa);
        let start=wave.position();
        for (read,addr,data,ack) in accesses {synth::data_register(&mut wave,synth::request(read,addr*4,data),synth::response(ack,data^0xa5a55a5a));}
        wave.idle(3);let block=wave.finish();
        assert_block_invariant(|g:GraphConfig| Harness::<JtagDpEvent>::build(5,TimeBase::new(1_000_000),g,|graph,source| {
            let j=graph.add(Jtag::new(JtagConfig::default()).expect("valid"));let dp=graph.add(JtagDp::new(JtagDpConfig {}));
            graph.connect(source.samples,Jtag::input(j))?;graph.connect(Jtag::events(j),JtagDp::input(dp))?;Ok(JtagDp::events(dp))
        }).expect("build"),&block,&[TickRange::new(Tick(start.0+5),Tick(start.0+8))]);
    }
}
