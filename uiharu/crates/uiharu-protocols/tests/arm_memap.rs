// SPDX-License-Identifier: Apache-2.0

//! ADIv5 plans retain scan identities and never assert successful memory operations.
use uiharu_core::testing::{Harness, assert_block_invariant};
use uiharu_core::{GraphConfig, LogicBlock, Tick, TickRange, TimeBase};
use uiharu_protocols::arm_memap::{ArmMemap, ArmMemapConfig, ArmMemapEvent, RegisterName};
use uiharu_protocols::jtag::{Jtag, JtagConfig, synth::JtagWaveform};
use uiharu_protocols::jtag_dp::{JtagDp, JtagDpConfig, synth};

fn build(config: ArmMemapConfig, graph: GraphConfig) -> Harness<ArmMemapEvent> {
    Harness::build(5, TimeBase::new(1_000_000), graph, |graph, source| {
        let tap = graph.add(Jtag::new(JtagConfig::default()).expect("TAP"));
        graph.connect(source.samples, Jtag::input(tap))?;
        let dp = graph.add(JtagDp::new(JtagDpConfig {}));
        graph.connect(Jtag::events(tap), JtagDp::input(dp))?;
        let ap = graph.add(ArmMemap::new(config).expect("MEMAP"));
        graph.connect(JtagDp::events(dp), ArmMemap::input(ap))?;
        Ok(ArmMemap::events(ap))
    })
    .expect("pipeline")
}
fn request(w: &mut JtagWaveform, read: bool, addr: u8, value: u32, response: u32, wait: bool) {
    // IHI0031A DPACC/APACC: RnW,A[3:2],32-bitDATAIN; ACK thenprior32-bitresult.
    let input = (u64::from(value) << 3) | (u64::from(addr >> 2) << 1) | u64::from(read);
    let output = (u64::from(response) << 3) | if wait { 1 } else { 2 };
    synth::data_register(w, input, output);
}
fn trace(csw: u32, ap: u8) -> LogicBlock {
    let mut w = JtagWaveform::new(JtagConfig::default(), 5, 2, Tick::ZERO);
    w.idle(8);
    w.reset();
    synth::instruction(&mut w, 10);
    request(&mut w, false, 8, u32::from(ap) << 24, 0, false);
    synth::instruction(&mut w, 11);
    request(&mut w, false, 0, csw, 0, false);
    request(&mut w, false, 4, 0x200003fc, 0, false);
    request(&mut w, false, 12, 0x11223344, 0, false);
    request(&mut w, true, 12, 0, 0, true); // discarded request, no TARincrement
    request(&mut w, true, 12, 0, 0, false); // acknowledges priorwrite, emitsnextplan
    synth::instruction(&mut w, 10);
    request(&mut w, true, 4, 0, 0x55667788, false);
    request(&mut w, true, 12, 0, 0x20, false); // captured CTRL/STAT stickyerrorinvalidatescandidateAPstate
    synth::instruction(&mut w, 11);
    request(&mut w, false, 12, 0xa5, 0, false);
    w.idle(8);
    w.finish()
}
fn config() -> ArmMemapConfig {
    ArmMemapConfig {
        ap: 0,
        tar_wrap_bits: 10,
    }
}
fn plans(events: &[ArmMemapEvent]) -> Vec<Option<u32>> {
    events
        .iter()
        .filter_map(|e| match e {
            ArmMemapEvent::MemoryRequest {
                planned_address, ..
            } => Some(*planned_address),
            _ => None,
        })
        .collect()
}
#[test]
fn plans_use_posted_register_context_waits_wrap_and_fault_invalidation() {
    let block = trace(0x12, 0);
    let mut h = build(config(), GraphConfig::default());
    h.run_all(&block);
    let events: Vec<_> = h.events().into_iter().map(|event| event.payload).collect();
    assert_eq!(
        plans(&events),
        vec![Some(0x200003fc), Some(0x20000000), None]
    );
    assert_eq!(
        events
            .iter()
            .filter(|e| matches!(e, ArmMemapEvent::Wait { .. }))
            .count(),
        1
    );
    assert!(events.iter().any(|e| matches!(
        e,
        ArmMemapEvent::Status {
            sticky_fault: true,
            ..
        }
    )));
    for event in events {
        if let ArmMemapEvent::Response {
            scan,
            response_scan,
            ok_or_fault,
            ..
        }
        | ArmMemapEvent::MemoryResponse {
            scan,
            response_scan,
            ok_or_fault,
            ..
        } = event
        {
            assert!(scan < response_scan);
            assert!(ok_or_fault)
        }
    }
}
#[test]
fn configured_ap_and_wrap_width_are_applied_without_fabricated_state() {
    let block = trace(0x12, 1);
    let mut other = build(config(), GraphConfig::default());
    other.run_all(&block);
    assert!(
        other
            .events()
            .iter()
            .all(|e| !matches!(e.payload, ArmMemapEvent::MemoryRequest { .. }))
    );
    let mut chosen = build(
        ArmMemapConfig {
            ap: 1,
            tar_wrap_bits: 12,
        },
        GraphConfig::default(),
    );
    chosen.run_all(&block);
    let events: Vec<_> = chosen.events().into_iter().map(|e| e.payload).collect();
    assert_eq!(
        plans(&events),
        vec![Some(0x200003fc), Some(0x20000400), None]
    );
    assert!(
        ArmMemap::new(ArmMemapConfig {
            ap: 0,
            tar_wrap_bits: 9
        })
        .is_err()
    );
    assert!(
        ArmMemap::new(ArmMemapConfig {
            ap: 0,
            tar_wrap_bits: 32
        })
        .is_err()
    );
}
#[test]
fn sizes_and_packed_increment_use_raw_words_without_inferred_endianness() {
    for (csw, stride) in [
        (0x10, 1),
        (0x11, 2),
        (0x12, 4),
        (0x20, 4),
        (0x21, 4),
        (2, 0),
    ] {
        let mut h = build(
            ArmMemapConfig {
                ap: 0,
                tar_wrap_bits: 12,
            },
            GraphConfig::default(),
        );
        h.run_all(&trace(csw, 0));
        let events: Vec<_> = h.events().into_iter().map(|e| e.payload).collect();
        assert_eq!(plans(&events)[1], Some(0x200003fc + stride));
    }
}
#[test]
fn unknown_select_does_not_become_ap_zero_and_reset_loses_context() {
    let mut w = JtagWaveform::new(JtagConfig::default(), 5, 2, Tick::ZERO);
    w.idle(8);
    w.reset();
    synth::instruction(&mut w, 11);
    request(&mut w, false, 12, 1, 0, false);
    w.idle(8);
    let mut h = build(config(), GraphConfig::default());
    h.run_all(&w.finish());
    assert!(h.events().iter().any(|e| matches!(
        e.payload,
        ArmMemapEvent::Request {
            register: RegisterName::ApRegister,
            ap_index: None,
            address: None,
            ..
        }
    )));
    assert!(
        h.events()
            .iter()
            .all(|e| !matches!(e.payload, ArmMemapEvent::MemoryRequest { .. }))
    );
}
#[test]
fn planned_accesses_are_invariant_under_blocks_gaps_and_backpressure() {
    let block = trace(0x12, 0);
    assert_block_invariant(|g| build(config(), g), &block, &[]);
    assert_block_invariant(
        |g| build(config(), g),
        &block,
        &[TickRange::new(Tick(700), Tick(800))],
    );
}

#[test]
fn ap_register_reads_complete_on_next_accepted_scan_and_rdbuff_drains_last_result() {
    // IHI0031A §4.5/Tables4-5,4-6: JTAG has one scan of pipelining, unlike
    // the SW-DP AP read posting path. RDBUFF shifts the preceding AP result.
    let mut w = JtagWaveform::new(JtagConfig::default(), 5, 2, Tick::ZERO);
    w.idle(8);
    w.reset();
    synth::instruction(&mut w, 10);
    request(&mut w, false, 8, 0, 0, false); // SELECT
    synth::instruction(&mut w, 11);
    request(&mut w, true, 0, 0, 0, false); // CSW read; previous SELECT completes
    request(&mut w, true, 4, 0, 0x12, false); // TAR read; CSW result
    request(&mut w, true, 12, 0, 0, true); // WAIT discards this DRW request
    synth::instruction(&mut w, 10);
    request(&mut w, true, 12, 0, 0x20000040, false); // RDBUFF shifts prior TAR result
    synth::instruction(&mut w, 11);
    request(&mut w, true, 12, 0, 0, false); // DRW, RDBUFF own result zero
    synth::instruction(&mut w, 10);
    request(&mut w, true, 12, 0, 0x11223344, false); // RDBUFF drains DRW result
    request(&mut w, true, 12, 0, 0, false); // RDBUFF own result zero
    w.idle(8);
    let mut h = build(config(), GraphConfig::default());
    h.run_all(&w.finish());
    let e: Vec<_> = h.events().into_iter().map(|e| e.payload).collect();
    assert_eq!(plans(&e), vec![Some(0x20000040)]);
    assert!(e.iter().any(|e| matches!(
        e,
        ArmMemapEvent::Response {
            register: RegisterName::Csw,
            value: 0x12,
            ..
        }
    )));
    assert!(e.iter().any(|e| matches!(
        e,
        ArmMemapEvent::Response {
            register: RegisterName::Tar,
            value: 0x20000040,
            ..
        }
    )));
    assert!(e.iter().any(|e| matches!(
        e,
        ArmMemapEvent::MemoryResponse {
            raw_data: 0x11223344,
            planned_address: Some(0x20000040),
            ..
        }
    )));
}
