// SPDX-License-Identifier: Apache-2.0

//! Explicit chain concatenation follows IEEE1149.1 wire order, not inferred topology.
use uiharu_core::testing::{Harness, assert_block_invariant};
use uiharu_core::{GraphConfig, GraphError, LogicBlock, Tick, TickRange, TimeBase};
use uiharu_protocols::jtag::{
    EndReason, Jtag, JtagConfig, JtagEvent, Register, TapState, synth::JtagWaveform,
};
use uiharu_protocols::jtag_chain::{JtagChain, JtagChainConfig};
use uiharu_protocols::jtag_dp::{JtagDp, JtagDpConfig, JtagDpEvent};
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
fn build(c: JtagChainConfig, g: GraphConfig) -> Harness<JtagEvent> {
    Harness::build(5, TimeBase::new(1_000_000), g, |graph, source| {
        let tap = graph.add(Jtag::new(JtagConfig::default()).expect("TAP"));
        graph.connect(source.samples, Jtag::input(tap))?;
        let chain = graph.add(JtagChain::new(c).expect("chain"));
        graph.connect(Jtag::events(tap), JtagChain::input(chain))?;
        Ok(JtagChain::events(chain))
    })
    .expect("pipeline")
}
fn scan(w: &mut JtagWaveform, register: Register, input: u64, output: u64, bits: u32) {
    w.scan(register, &input.to_le_bytes(), &output.to_le_bytes(), bits)
}
fn wave() -> LogicBlock {
    let mut w = JtagWaveform::new(JtagConfig::default(), 5, 2, Tick::ZERO);
    w.idle(8);
    w.reset();
    scan(&mut w, Register::Ir, (0xa << 3) | 5 | (2 << 7), 1 << 3, 9);
    scan(&mut w, Register::Dr, 4 << 2, 2 << 2, 38);
    scan(
        &mut w,
        Register::Dr,
        3 << 2,
        ((0x11223344u64 << 3) | 2) << 2,
        38,
    );
    w.idle(8);
    w.finish()
}
fn value(e: &[JtagEvent], scan: u64, output: bool) -> u64 {
    let mut result = 0;
    for e in e {
        if let JtagEvent::Shift {
            scan: s,
            offset,
            tdi,
            tdo,
            bits,
        } = e
            && *s == scan
        {
            assert_eq!(*bits, 1);
            let data = if output { tdo } else { tdi };
            result |= u64::from(data.as_ref().expect("line")[0] & 1) << offset;
        }
    }
    result
}
#[test]
fn explicit_chain_window_rebases_bits_and_validates_total_width() {
    let mut h = build(config(), GraphConfig::default());
    h.run_all(&wave());
    let e: Vec<_> = h.events().into_iter().map(|e| e.payload).collect();
    assert_eq!(value(&e, 1, false), 0xa);
    assert_eq!(value(&e, 1, true), 1);
    assert_eq!(value(&e, 2, false), 4);
    assert_eq!(value(&e, 3, true), (0x11223344 << 3) | 2);
    assert!(e.iter().any(|e| matches!(
        e,
        JtagEvent::ScanEnd {
            scan: 1,
            bits: 4,
            reason: EndReason::Complete
        }
    )));
    assert!(e.iter().any(|e| matches!(
        e,
        JtagEvent::ScanEnd {
            scan: 3,
            bits: 35,
            reason: EndReason::Complete
        }
    )));
}
#[test]
fn selected_tap_drives_existing_dp_pipeline_with_scan_identity() {
    let mut h = Harness::build(
        5,
        TimeBase::new(1_000_000),
        GraphConfig::default(),
        |graph, source| {
            let tap = graph.add(Jtag::new(JtagConfig::default()).expect("TAP"));
            graph.connect(source.samples, Jtag::input(tap))?;
            let chain = graph.add(JtagChain::new(config()).expect("chain"));
            graph.connect(Jtag::events(tap), JtagChain::input(chain))?;
            let dp = graph.add(JtagDp::new(JtagDpConfig {}));
            graph.connect(JtagChain::events(chain), JtagDp::input(dp))?;
            Ok(JtagDp::events(dp))
        },
    )
    .expect("pipeline");
    h.run_all(&wave());
    let e: Vec<_> = h.events().into_iter().map(|e| e.payload).collect();
    assert!(
        e.iter()
            .any(|e| matches!(e, JtagDpEvent::Instruction { value: 10, .. }))
    );
    assert!(e.iter().any(|e| matches!(
        e,
        JtagDpEvent::Access {
            scan: 3,
            previous_scan: Some(2),
            response_data: 0x11223344,
            ..
        }
    )));
}
#[test]
fn wrong_width_partial_scans_and_missing_lines_never_commit() {
    let mut c = config();
    c.ir_after += 1;
    let mut h = build(c, GraphConfig::default());
    h.run_all(&wave());
    assert!(h.events().iter().any(|e| matches!(
        e.payload,
        JtagEvent::ScanEnd {
            scan: 1,
            reason: EndReason::Aborted,
            ..
        }
    )));
    let mut w = JtagWaveform::new(JtagConfig::default(), 5, 2, Tick::ZERO);
    w.idle(8);
    w.reset();
    w.go_to(TapState::ShiftDr);
    w.clock(false, true);
    w.clock(false, false);
    let mut h = build(config(), GraphConfig::default());
    h.run_all(&w.finish());
    assert!(h.events().iter().any(|e| matches!(
        e.payload,
        JtagEvent::ScanEnd {
            reason: EndReason::Aborted,
            ..
        }
    )));
    let missing = Harness::<JtagEvent>::build(
        5,
        TimeBase::new(1_000_000),
        GraphConfig::default(),
        |graph, source| {
            let tap = graph.add(
                Jtag::new(JtagConfig {
                    tdi: None,
                    ..JtagConfig::default()
                })
                .expect("TAP"),
            );
            graph.connect(source.samples, Jtag::input(tap))?;
            let chain = graph.add(JtagChain::new(config()).expect("chain"));
            graph.connect(Jtag::events(tap), JtagChain::input(chain))?;
            Ok(JtagChain::events(chain))
        },
    );
    assert!(matches!(missing,Err(GraphError::Requirement{key,..})if key=="tdi"));
    for c in [
        JtagChainConfig {
            ir_bits: 0,
            ..config()
        },
        JtagChainConfig {
            dr_bits: 4097,
            ..config()
        },
        JtagChainConfig {
            dr_before: 4097,
            ..config()
        },
    ] {
        assert!(JtagChain::new(c).is_err());
    }
}
#[test]
fn selected_bits_are_invariant_across_boundaries_backpressure_and_gaps() {
    let block = wave();
    assert_block_invariant(|g| build(config(), g), &block, &[]);
    assert_block_invariant(
        |g| build(config(), g),
        &block,
        &[TickRange::new(Tick(190), Tick(230))],
    );
}
