// SPDX-License-Identifier: Apache-2.0

use uiharu_core::testing::{Harness, assert_block_invariant, assert_progress_promise};
use uiharu_core::{GraphConfig, Item, LogicBlock, Tick, TickRange, TimeBase};
use uiharu_protocols::jtag::synth::JtagWaveform;
use uiharu_protocols::jtag::{Jtag, JtagConfig, Register, TapState};
use uiharu_protocols::jtag_dp::{Ack, JtagDp, JtagDpConfig, JtagDpEvent, ScanStatus, synth};

fn harness(config: JtagConfig, g: GraphConfig) -> Harness<JtagDpEvent> {
    Harness::build(5, TimeBase::new(1_000_000), g, |graph, source| {
        let j = graph.add(Jtag::new(config).expect("valid JTAG"));
        let dp = graph.add(JtagDp::new(JtagDpConfig {}));
        graph.connect(source.samples, Jtag::input(j))?;
        graph.connect(Jtag::events(j), JtagDp::input(dp))?;
        Ok(JtagDp::events(dp))
    })
    .expect("build")
}
fn waveform() -> JtagWaveform {
    let mut w = JtagWaveform::new(JtagConfig::default(), 5, 2, Tick::ZERO);
    w.idle(5);
    w.reset();
    w
}
fn decode(block: &LogicBlock, gaps: &[TickRange]) -> Vec<Item<JtagDpEvent>> {
    let mut h = harness(JtagConfig::default(), GraphConfig::default());
    h.feed_with_gaps(block, gaps, &[u64::MAX]);
    h.end();
    assert_progress_promise(&h.items);
    h.items
}
fn payloads(items: &[Item<JtagDpEvent>]) -> Vec<JtagDpEvent> {
    items
        .iter()
        .filter_map(|i| match i {
            Item::Event(e) => Some(e.payload),
            _ => None,
        })
        .collect()
}

#[test]
fn published_wire_vectors_and_pipelined_wait_response() {
    let mut w = waveform();
    // ARM IHI0031C table 3-2: IR 1010 = DPACC; figure 3-7:
    // request bits [0]=RnW, [2:1]=A[3:2], [34:3]=DATAIN;
    // captured [2:0]=ACK, [34:3]=ReadResult.
    synth::instruction(&mut w, 0xa);
    synth::data_register(&mut w, 0b011, 0b010); // read DP 0x4, OK/FAULT
    synth::data_register(&mut w, 0x91a2_b3c0, 0b001); // WAIT discards current request
    synth::data_register(&mut w, 0b111, (0xdead_beefu64 << 3) | 2);
    w.idle(5);
    let events = payloads(&decode(&w.finish(), &[]));
    let accesses: Vec<_> = events
        .into_iter()
        .filter(|e| matches!(e, JtagDpEvent::Access { .. }))
        .collect();
    assert_eq!(
        accesses,
        vec![
            JtagDpEvent::Access {
                scan: 2,
                instruction: 0xa,
                read: true,
                addr: 4,
                data_in: 0,
                ack_bits: 2,
                ack: Ack::OkFault,
                response_data: 0,
                previous_scan: None,
                accepted: Some(true)
            },
            JtagDpEvent::Access {
                scan: 3,
                instruction: 0xa,
                read: false,
                addr: 0,
                data_in: 0x12345678,
                ack_bits: 1,
                ack: Ack::Wait,
                response_data: 0,
                previous_scan: Some(2),
                accepted: Some(false)
            },
            JtagDpEvent::Access {
                scan: 4,
                instruction: 0xa,
                read: true,
                addr: 12,
                data_in: 0,
                ack_bits: 2,
                ack: Ack::OkFault,
                response_data: 0xdeadbeef,
                previous_scan: Some(2),
                accepted: Some(true)
            },
        ]
    );
}
#[test]
fn idcode_abort_bypass_and_apacc_have_exact_register_lengths() {
    let mut w = waveform();
    // Reset selects IDCODE per IHI0031C §3.3.1. The illustrative value has
    // ARM designer 0x23b and bit zero set, as figure 3-6 specifies.
    w.scan(Register::Dr, &[0; 4], &0x1ba0_0477u32.to_le_bytes(), 32);
    synth::instruction(&mut w, 0xf);
    w.scan(Register::Dr, &[1], &[0], 1);
    synth::instruction(&mut w, 8);
    synth::data_register(&mut w, 8, 0); // §3.4.4 DPv0 DAPABORT
    synth::instruction(&mut w, 0xb);
    synth::data_register(
        &mut w,
        synth::request(false, 8, 0x87654321),
        synth::response(2, 0),
    );
    w.idle(5);
    let events = payloads(&decode(&w.finish(), &[]));
    assert!(events.contains(&JtagDpEvent::Idcode {
        scan: 1,
        value: 0x1ba00477
    }));
    assert!(events.contains(&JtagDpEvent::Bypass {
        scan: 3,
        tdi: true,
        tdo: false
    }));
    assert!(events.contains(&JtagDpEvent::Abort {
        scan: 5,
        value: 1,
        low_bits: 0
    }));
    assert!(events.iter().any(|e| matches!(
        e,
        JtagDpEvent::Access {
            instruction: 0xb,
            addr: 8,
            data_in: 0x87654321,
            previous_scan: None,
            ..
        }
    )));
}
#[test]
fn malformed_instructions_unknown_capture_and_reserved_ack_never_invent_success() {
    let mut w = waveform();
    w.scan(Register::Ir, &[0xa], &[1], 3);
    w.scan(Register::Dr, &[0; 5], &[2], 35);
    synth::instruction(&mut w, 0xa);
    w.scan(Register::Dr, &[0; 5], &[2], 34);
    synth::data_register(&mut w, 3, 7);
    synth::data_register(&mut w, 3, 2);
    synth::instruction(&mut w, 3);
    w.scan(Register::Dr, &[0], &[0], 8);
    w.idle(5);
    let events = payloads(&decode(&w.finish(), &[]));
    assert!(events.contains(&JtagDpEvent::Scan {
        scan: 1,
        ir: true,
        bits: 3,
        status: ScanStatus::InvalidLength
    }));
    assert!(events.contains(&JtagDpEvent::Scan {
        scan: 2,
        ir: false,
        bits: 35,
        status: ScanStatus::UnknownInstruction
    }));
    assert!(events.contains(&JtagDpEvent::Scan {
        scan: 4,
        ir: false,
        bits: 34,
        status: ScanStatus::InvalidLength
    }));
    assert!(events.iter().any(|e| matches!(
        e,
        JtagDpEvent::Access {
            scan: 5,
            ack: Ack::Invalid,
            ack_bits: 7,
            accepted: None,
            ..
        }
    )));
    assert!(events.iter().any(|e| matches!(
        e,
        JtagDpEvent::Access {
            scan: 6,
            previous_scan: None,
            ..
        }
    )));
    assert!(events.contains(&JtagDpEvent::Scan {
        scan: 8,
        ir: false,
        bits: 8,
        status: ScanStatus::UnknownInstruction
    }));
}
#[test]
fn partial_scan_never_commits_and_gaps_lose_instruction_and_pipeline_state() {
    let mut w = waveform();
    synth::instruction(&mut w, 0xa);
    synth::data_register(&mut w, 3, 2);
    let gap_start = w.position();
    w.idle(8);
    let gap_end = w.position();
    // Re-synchronize the TAP. A reset restores IDCODE, not the old DPACC.
    w.reset();
    w.scan(Register::Dr, &[0; 4], &0x12345679u32.to_le_bytes(), 32);
    synth::instruction(&mut w, 0xa);
    w.go_to(TapState::ShiftDr);
    w.shift(&[3], 10, false);
    w.idle(4);
    let block = w.finish();
    let gaps = [TickRange::new(gap_start, gap_end)];
    let items = decode(&block, &gaps);
    let events = payloads(&items);
    assert!(events.iter().any(|e| matches!(
        e,
        JtagDpEvent::Idcode {
            value: 0x12345679,
            ..
        }
    )));
    assert!(events.iter().any(|e| matches!(
        e,
        JtagDpEvent::Scan {
            ir: false,
            bits: 10,
            status: ScanStatus::Aborted,
            ..
        }
    )));
    assert_eq!(
        events
            .iter()
            .filter(|e| matches!(e, JtagDpEvent::Access { .. }))
            .count(),
        1
    );
    assert_block_invariant(|g| harness(JtagConfig::default(), g), &block, &gaps);
}
#[test]
fn bypass_preserves_pipeline_but_idcode_selection_discards_it() {
    let mut w = waveform();
    synth::instruction(&mut w, 0xa);
    synth::data_register(&mut w, 3, 2);
    synth::instruction(&mut w, 0xf);
    w.scan(Register::Dr, &[1], &[0], 1);
    synth::instruction(&mut w, 0xb);
    synth::data_register(&mut w, 1, 2);
    synth::instruction(&mut w, 0xe);
    w.scan(Register::Dr, &[0; 4], &[1, 0, 0, 0], 32);
    synth::instruction(&mut w, 0xa);
    synth::data_register(&mut w, 3, 2);
    w.idle(3);
    let events = payloads(&decode(&w.finish(), &[]));
    assert!(events.iter().any(|e| matches!(
        e,
        JtagDpEvent::Access {
            scan: 6,
            previous_scan: Some(2),
            ..
        }
    )));
    assert!(events.iter().any(|e| matches!(
        e,
        JtagDpEvent::Access {
            scan: 10,
            previous_scan: None,
            ..
        }
    )));
}
#[test]
fn missing_bus_lines_are_rejected_by_graph() {
    for config in [
        JtagConfig {
            tdi: None,
            ..Default::default()
        },
        JtagConfig {
            tdo: None,
            ..Default::default()
        },
    ] {
        let built = Harness::<JtagDpEvent>::build(
            5,
            TimeBase::new(1_000_000),
            GraphConfig::default(),
            |graph, source| {
                let j = graph.add(Jtag::new(config).expect("valid"));
                let dp = graph.add(JtagDp::new(JtagDpConfig {}));
                graph.connect(source.samples, Jtag::input(j))?;
                graph.connect(Jtag::events(j), JtagDp::input(dp))?;
                Ok(JtagDp::events(dp))
            },
        );
        assert!(built.is_err());
    }
}
#[test]
fn serde_contract_is_explicit_and_unknown_options_are_rejected() {
    assert_eq!(
        serde_json::to_value(JtagDpEvent::Abort {
            scan: 1,
            value: 1,
            low_bits: 0
        })
        .expect("serializes"),
        serde_json::json!({"kind":"abort","scan":1,"value":1,"lowBits":0})
    );
    assert!(serde_json::from_str::<JtagDpConfig>(r#"{"chain":2}"#).is_err());
}

#[test]
fn instruction_provenance_covers_shift_entry_through_committed_update() {
    let mut w = waveform();
    let start = w.position();
    synth::instruction(&mut w, 0xa);
    let end = w.position();
    w.idle(3);
    let items = decode(&w.finish(), &[]);
    let event = items
        .iter()
        .find_map(|item| match item {
            Item::Event(e) if matches!(e.payload, JtagDpEvent::Instruction { .. }) => Some(e),
            _ => None,
        })
        .expect("instruction");
    // From Test-Logic-Reset, five clocks enter Shift-IR; four shift and
    // one commits Update-IR. The final idle clock follows that update.
    assert_eq!(event.span.start, Tick(start.0 + 18));
    assert_eq!(event.settled, Tick(end.0 - 6));
    assert_eq!(event.span.end, event.settled.next());
}

#[test]
fn short_wait_scans_preserve_the_outstanding_request() {
    let mut w = waveform();
    synth::instruction(&mut w, 0xa);
    synth::data_register(&mut w, 3, 2);
    for bits in [3, 4, 34, 36, 300] {
        w.scan(Register::Dr, &[0; 5], &[1], bits);
    }
    synth::data_register(&mut w, 7, synth::response(2, 0x12345678));
    w.idle(3);
    let events = payloads(&decode(&w.finish(), &[]));
    for (scan, bits) in [(3, 3), (4, 4), (5, 34), (6, 36), (7, 300)] {
        assert!(events.contains(&JtagDpEvent::Scan {
            scan,
            ir: false,
            bits,
            status: ScanStatus::Wait
        }));
    }
    assert!(events.iter().any(|e| matches!(
        e,
        JtagDpEvent::Access {
            scan: 8,
            previous_scan: Some(2),
            response_data: 0x12345678,
            ..
        }
    )));
}
