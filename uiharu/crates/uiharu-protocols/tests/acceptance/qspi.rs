// SPDX-License-Identifier: Apache-2.0

use proptest::prelude::*;
use uiharu_core::testing::{Harness, assert_block_invariant};
use uiharu_core::{GraphConfig, Tick, TickRange, TimeBase};
use uiharu_protocols::qspi::synth::QspiWaveform;
use uiharu_protocols::qspi::{Qspi, QspiConfig, QspiEvent};

proptest! {
    #![proptest_config(ProptestConfig::with_cases(16))]
    #[test]
    fn random_qspi_is_invariant(words in prop::collection::vec(any::<u32>(),1..24), mode in 0u8..4, dual in any::<bool>(), lsb_first in any::<bool>()) {
        let config=QspiConfig {mode,lsb_first,data:if dual {vec![2,3]} else {vec![2,3,4,5]},word_bits:32,..Default::default()};
        let mut wave=QspiWaveform::new(config.clone(),6,1,Tick::ZERO);
        wave.idle(3);wave.select();for word in words {wave.word(word);}wave.release();wave.idle(3);
        let block=wave.finish();
        assert_block_invariant(|g:GraphConfig| Harness::<QspiEvent>::build(6,TimeBase::new(1_000_000),g,|graph,source| {
            let n=graph.add(Qspi::new(config.clone()).expect("valid"));graph.connect(source.samples,Qspi::input(n))?;Ok(Qspi::events(n))
        }).expect("build"),&block,&[TickRange::new(Tick(9),Tick(11))]);
    }
}
