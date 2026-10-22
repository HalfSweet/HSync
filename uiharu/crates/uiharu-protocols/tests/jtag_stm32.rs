// SPDX-License-Identifier: Apache-2.0

//! ST RM0008 Rev21 §31.6 ID table and §31.5 chain geometry; Arm IHI0031A scan fields.
use uiharu_core::testing::{Harness, assert_block_invariant};
use uiharu_core::{GraphConfig, Tick, TickRange, TimeBase};
use uiharu_protocols::jtag::{Jtag, JtagConfig, Register, synth::JtagWaveform};
use uiharu_protocols::jtag_stm32::{
    Density, ErrorReason, JtagStm32, JtagStm32Config, JtagStm32Event,
};
fn build(config: GraphConfig) -> Harness<JtagStm32Event> {
    Harness::build(5, TimeBase::new(1_000_000), config, |graph, source| {
        let tap = graph.add(Jtag::new(JtagConfig::default()).expect("TAP"));
        graph.connect(source.samples, Jtag::input(tap))?;
        let stm = graph.add(JtagStm32::new(JtagStm32Config {}).expect("STM32"));
        graph.connect(Jtag::events(tap), JtagStm32::input(stm))?;
        Ok(JtagStm32::events(stm))
    })
    .expect("pipeline")
}
fn start() -> JtagWaveform {
    let mut wave = JtagWaveform::new(JtagConfig::default(), 5, 2, Tick::ZERO);
    wave.idle(8);
    wave.reset();
    wave
}
fn ir(wave: &mut JtagWaveform, mcu: u8, bsc: u8) {
    let value = u16::from(mcu) | (u16::from(bsc) << 4);
    wave.scan(Register::Ir, &value.to_le_bytes(), &[0x11, 0], 9);
}
fn dr(wave: &mut JtagWaveform, input: u64, output: u64, bits: u32) {
    wave.scan(
        Register::Dr,
        &input.to_le_bytes(),
        &output.to_le_bytes(),
        bits,
    );
}
fn decode(wave: JtagWaveform) -> Vec<JtagStm32Event> {
    let mut harness = build(GraphConfig::default());
    harness.run_all(&wave.finish());
    harness
        .events()
        .into_iter()
        .map(|event| event.payload)
        .collect()
}
#[test]
fn reset_selects_both_idcodes_in_documented_chain_order() {
    let mut wave = start();
    dr(&mut wave, 0, (0x16410041_u64 << 32) | 0x3ba00477, 64);
    let events = decode(wave);
    assert!(events.iter().any(|event| matches!(
        event,
        JtagStm32Event::Idcode {
            boundary_scan: false,
            value: 0x3ba00477,
            manufacturer: 0x23b,
            density: Density::Unknown,
            ..
        }
    )));
    assert!(events.iter().any(|event| matches!(
        event,
        JtagStm32Event::Idcode {
            boundary_scan: true,
            value: 0x16410041,
            version: 1,
            manufacturer: 0x20,
            density: Density::Medium,
            ..
        }
    )));
}
#[test]
fn boundary_scan_id_table_and_bypass_windows_are_exact() {
    for (value, density) in [
        (0x06412041, Density::Low),
        (0x06410041, Density::Medium),
        (0x06414041, Density::High),
        (0x06430041, Density::Xl),
        (0x06418041, Density::Connectivity),
        (0x12345041, Density::Unknown),
    ] {
        let mut wave = start();
        ir(&mut wave, 15, 1);
        dr(&mut wave, 0, (u64::from(value as u32) << 1) | 1, 33);
        assert!(decode(wave).iter().any(|event| matches!(event,JtagStm32Event::Idcode{boundary_scan:true,value:observed,density:observed_density,..} if *observed==value as u32&&*observed_density==density)));
    }
}
#[test]
fn dynamic_dp_width_and_wait_preserve_previous_accepted_scan() {
    let mut wave = start();
    ir(&mut wave, 10, 31);
    dr(
        &mut wave,
        (0xaabbccdd_u64 << 3) | 2,
        (0x12345678_u64 << 3) | 2 | (1 << 35),
        36,
    );
    dr(&mut wave, 7, 1, 36);
    dr(&mut wave, 7, (0x55667788_u64 << 3) | 2, 36);
    ir(&mut wave, 8, 31);
    dr(&mut wave, 8, 0, 36);
    ir(&mut wave, 14, 31);
    dr(&mut wave, 0, 0x3ba00477, 33);
    let events = decode(wave);
    assert!(events.iter().any(|event| matches!(
        event,
        JtagStm32Event::Access {
            scan: 2,
            addr: 4,
            data_in: 0xaabbccdd,
            response_data: 0x12345678,
            previous_scan: None,
            accepted: Some(true),
            ..
        }
    )));
    assert!(events.iter().any(|event| matches!(
        event,
        JtagStm32Event::Access {
            scan: 3,
            previous_scan: Some(2),
            accepted: Some(false),
            ..
        }
    )));
    assert!(events.iter().any(|event| matches!(
        event,
        JtagStm32Event::Access {
            scan: 4,
            previous_scan: Some(2),
            response_data: 0x55667788,
            ..
        }
    )));
    assert!(events.iter().any(|event| matches!(
        event,
        JtagStm32Event::Abort {
            value: 1,
            low_bits: 0,
            ..
        }
    )));
    assert!(events.iter().any(|event| matches!(
        event,
        JtagStm32Event::Idcode {
            value: 0x3ba00477,
            ..
        }
    )));
}
#[test]
fn invalid_profiles_capture_lengths_and_loss_clear_context() {
    let mut wave = start();
    wave.scan(Register::Ir, &[0xfa, 1], &[0, 0], 9);
    dr(&mut wave, 0, 0, 36);
    ir(&mut wave, 14, 0);
    dr(&mut wave, 0, 0, 33);
    ir(&mut wave, 14, 31);
    dr(&mut wave, 0, 0, 32);
    let events = decode(wave);
    for reason in [
        ErrorReason::InvalidCapture,
        ErrorReason::UnknownInstruction,
        ErrorReason::InvalidLength,
    ] {
        assert!(events.iter().any(
            |event| matches!(event,JtagStm32Event::Error{reason:observed,..} if *observed==reason)
        ));
    }
    let mut wave = start();
    ir(&mut wave, 10, 31);
    dr(&mut wave, 7, 2, 36);
    let block = wave.finish();
    assert_block_invariant(build, &block, &[]);
    assert_block_invariant(build, &block, &[TickRange::new(Tick(300), Tick(390))]);
}
