// SPDX-License-Identifier: Apache-2.0

//! Register vectors follow MIPS MD00249 02.03 Tables8.27–8.30 and Figures8.2/8.3.
use uiharu_core::testing::{Harness, assert_block_invariant};
use uiharu_core::{GraphConfig, LogicBlock, Tick, TickRange, TimeBase};
use uiharu_protocols::jtag::{Jtag, JtagConfig, Register, TapState, synth::JtagWaveform};
use uiharu_protocols::jtag_ejtag::{
    ErrorReason, JtagEjtag, JtagEjtagConfig, JtagEjtagEvent, RegisterName,
};
fn build(width: u8, g: GraphConfig) -> Harness<JtagEjtagEvent> {
    Harness::build(5, TimeBase::new(1_000_000), g, |graph, source| {
        let tap = graph.add(Jtag::new(JtagConfig::default()).expect("TAP"));
        graph.connect(source.samples, Jtag::input(tap))?;
        let ejtag = graph.add(JtagEjtag::new(JtagEjtagConfig { ir_bits: width }).expect("EJTAG"));
        graph.connect(Jtag::events(tap), JtagEjtag::input(ejtag))?;
        Ok(JtagEjtag::events(ejtag))
    })
    .expect("pipeline")
}
fn start() -> JtagWaveform {
    let mut w = JtagWaveform::new(JtagConfig::default(), 5, 2, Tick::ZERO);
    w.idle(8);
    w.reset();
    w
}
fn ir(w: &mut JtagWaveform, value: u8, width: u32) {
    w.scan(Register::Ir, &[value], &[1], width)
}
fn dr(w: &mut JtagWaveform, input: u64, output: u64, bits: u32) {
    w.scan(
        Register::Dr,
        &input.to_le_bytes(),
        &output.to_le_bytes(),
        bits,
    )
}
fn all(w: &mut JtagWaveform, input: [u32; 3], output: [u32; 3]) {
    let mut a = Vec::new();
    let mut b = Vec::new();
    for word in input {
        a.extend(word.to_le_bytes())
    }
    for word in output {
        b.extend(word.to_le_bytes())
    }
    w.scan(Register::Dr, &a, &b, 96)
}
fn decode(width: u8, w: JtagWaveform) -> Vec<JtagEjtagEvent> {
    let mut h = build(width, GraphConfig::default());
    h.run_all(&w.finish());
    h.events().into_iter().map(|e| e.payload).collect()
}
#[test]
fn reset_idcode_and_implementation_version_are_observed_fields() {
    let mut w = start();
    dr(&mut w, 0, 0x04a07053, 32);
    ir(&mut w, 3, 5);
    dr(&mut w, 0, 0x40004000, 32);
    let e = decode(5, w);
    assert!(e.contains(&JtagEjtagEvent::Idcode {
        scan: 1,
        value: 0x04a07053,
        version: 0,
        part: 0x4a07,
        manufacturer: 0x29,
        lsb_ok: true
    }));
    assert!(e.iter().any(|e| matches!(
        e,
        JtagEjtagEvent::Implementation {
            value: 0x40004000,
            version: 2,
            ..
        }
    )));
}
#[test]
fn processor_completion_requests_require_observed_pending_context() {
    let mut w = start();
    ir(&mut w, 10, 5);
    dr(&mut w, 0x40000, 0xc0008, 32);
    ir(&mut w, 8, 5);
    dr(&mut w, 0, 0xff200200, 32);
    ir(&mut w, 9, 5);
    dr(&mut w, 0x3c1a1234, 0xaabbccdd, 32);
    ir(&mut w, 10, 5);
    dr(&mut w, 0, 0xc0008, 32);
    dr(&mut w, 0, 0, 32);
    dr(&mut w, 0, 0x40000, 32);
    let e = decode(5, w);
    let requests: Vec<_> = e
        .iter()
        .filter(|e| matches!(e, JtagEjtagEvent::ProcessorCompletionRequest { .. }))
        .collect();
    assert_eq!(requests.len(), 2);
    assert!(matches!(
        requests[0],
        JtagEjtagEvent::ProcessorCompletionRequest {
            address: Some(0xff200200),
            data_in: Some(0x3c1a1234),
            data_out: Some(0xaabbccdd),
            prnw: true,
            ..
        }
    ));
    assert!(matches!(
        requests[1],
        JtagEjtagEvent::ProcessorCompletionRequest {
            address: None,
            data_in: None,
            data_out: None,
            ..
        }
    ));
}
#[test]
fn concatenated_registers_and_fastdata_use_documented_bit_order() {
    let mut w = start();
    ir(&mut w, 11, 5);
    all(
        &mut w,
        [0, 0x12345678, 0],
        [0xc0008, 0xaabbccdd, 0xff200204],
    );
    ir(&mut w, 14, 5);
    dr(&mut w, 0x11223344u64 << 1, (0x8899aabbu64 << 1) | 1, 33);
    dr(&mut w, 0, 0, 33);
    dr(&mut w, 1, 1, 33);
    let e = decode(5, w);
    assert!(e.iter().any(|e| matches!(
        e,
        JtagEjtagEvent::ProcessorCompletionRequest {
            address: Some(0xff200204),
            data_in: Some(0x12345678),
            data_out: Some(0xaabbccdd),
            ..
        }
    )));
    assert!(e.iter().any(|e| matches!(
        e,
        JtagEjtagEvent::Fastdata {
            completion_requested: true,
            success_flag: true,
            completed: true,
            data_in: 0x11223344,
            data_out: 0x8899aabb,
            ..
        }
    )));
    assert!(e.iter().any(|e| matches!(
        e,
        JtagEjtagEvent::Fastdata {
            completion_requested: true,
            success_flag: false,
            completed: false,
            ..
        }
    )));
    assert!(e.iter().any(|e| matches!(
        e,
        JtagEjtagEvent::Fastdata {
            completion_requested: false,
            success_flag: true,
            completed: false,
            ..
        }
    )));
}
#[test]
fn boot_bypass_and_configured_instruction_width_are_explicit() {
    let mut w = start();
    for code in [12, 13, 255] {
        ir(&mut w, code, 8);
        dr(&mut w, 1, 0, 1)
    }
    let e = decode(8, w);
    for register in [
        RegisterName::Ejtagboot,
        RegisterName::Normalboot,
        RegisterName::Bypass,
    ] {
        assert!(e.iter().any(|e|matches!(e,JtagEjtagEvent::Instruction{register:r,capture_ok:true,..}if *r==register)));
    }
    assert_eq!(
        e.iter()
            .filter(|e| matches!(
                e,
                JtagEjtagEvent::Bypass {
                    data_in: true,
                    data_out: false,
                    ..
                }
            ))
            .count(),
        3
    );
    for width in [0, 4, 9] {
        assert!(JtagEjtag::new(JtagEjtagConfig { ir_bits: width }).is_err());
    }
}
#[test]
fn malformed_unknown_partial_and_bad_ir_capture_never_commit_register_values() {
    let mut w = start();
    ir(&mut w, 8, 4);
    dr(&mut w, 0, 0, 32);
    ir(&mut w, 0x12, 5);
    dr(&mut w, 0, 0, 32);
    ir(&mut w, 10, 5);
    dr(&mut w, 0, 0, 31);
    w.scan(Register::Ir, &[8], &[0], 5);
    dr(&mut w, 0, 0, 32);
    w.go_to(TapState::ShiftIr);
    w.clock(false, true);
    let e = decode(5, w);
    for reason in [
        ErrorReason::InvalidLength,
        ErrorReason::UnknownInstruction,
        ErrorReason::InvalidCapture,
        ErrorReason::Aborted,
    ] {
        assert!(
            e.iter()
                .any(|e| matches!(e,JtagEjtagEvent::Error{reason:r,..}if *r==reason))
        );
    }
    assert!(
        !e.iter()
            .any(|e| matches!(e, JtagEjtagEvent::Register { .. }))
    );
}
fn trace() -> LogicBlock {
    let mut w = start();
    ir(&mut w, 11, 5);
    all(
        &mut w,
        [0, 0x12345678, 0],
        [0xc0008, 0x8899aabb, 0xff200200],
    );
    ir(&mut w, 14, 5);
    dr(&mut w, 0x11223344u64 << 1, (0x8899aabbu64 << 1) | 1, 33);
    w.idle(8);
    w.finish()
}
#[test]
fn register_context_is_invariant_under_block_gap_and_backpressure() {
    let block = trace();
    assert_block_invariant(|g| build(5, g), &block, &[]);
    assert_block_invariant(
        |g| build(5, g),
        &block,
        &[TickRange::new(Tick(370), Tick(420))],
    );
}
