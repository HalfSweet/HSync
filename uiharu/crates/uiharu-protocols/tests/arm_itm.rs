// SPDX-License-Identifier: Apache-2.0

//! Byte vectors use Arm DDI0553B.y Part F1 packet tables, independent of the decoder.
use uiharu_core::testing::{Harness, assert_block_invariant};
use uiharu_core::{GraphConfig, LogicBlock, Tick, TickRange, TimeBase};
use uiharu_protocols::arm_itm::{ArmItm, ArmItmConfig, ArmItmEvent, ErrorReason};
use uiharu_protocols::arm_trace_transport::{
    ArmTraceTransport, ArmTraceTransportConfig, TraceFormat,
};
use uiharu_protocols::uart::{Uart, UartConfig, synth::UartWaveform};
fn build(aligned: bool, config: GraphConfig) -> Harness<ArmItmEvent> {
    Harness::build(1, TimeBase::new(96000), config, |graph, source| {
        let uart = graph.add(
            Uart::new(UartConfig {
                baud: 9600,
                ..UartConfig::default()
            })
            .expect("UART"),
        );
        graph.connect(source.samples, Uart::input(uart))?;
        let transport = graph.add(
            ArmTraceTransport::new(ArmTraceTransportConfig {
                format: TraceFormat::Raw,
                trace_id: 1,
                start_aligned: false,
            })
            .expect("trace adapter"),
        );
        graph.connect(Uart::events(uart), ArmTraceTransport::input(transport))?;
        let itm = graph.add(
            ArmItm::new(ArmItmConfig {
                start_aligned: aligned,
            })
            .expect("ITM"),
        );
        graph.connect(ArmTraceTransport::events(transport), ArmItm::input(itm))?;
        Ok(ArmItm::events(itm))
    })
    .expect("pipeline")
}
fn waveform(bytes: &[u8]) -> LogicBlock {
    let mut w = UartWaveform::new(
        UartConfig {
            baud: 9600,
            ..UartConfig::default()
        },
        1,
        10,
        Tick::ZERO,
    );
    w.idle(20);
    w.bytes(bytes.iter().copied());
    w.idle(20);
    w.finish()
}
fn decode(aligned: bool, bytes: &[u8]) -> Vec<ArmItmEvent> {
    let mut h = build(aligned, GraphConfig::default());
    h.run_all(&waveform(bytes));
    h.events().into_iter().map(|e| e.payload).collect()
}
#[test]
fn specified_packet_headers_payload_order_and_dwt_fields() {
    let e = decode(
        false,
        &[
            0, 0, 0, 0, 0, 0x80, 0x28, 0x09, 0x41, 0x13, 0x78, 0x56, 0x34, 0x12, 0x05, 0x22, 0x0e,
            0x03, 0x10, 0x17, 0x00, 0x10, 0x00, 0x08, 0x15, 0, 0x30, 0xd0, 0x81, 0x01, 0x70,
        ],
    );
    assert!(e.contains(&ArmItmEvent::Sync));
    assert!(e.contains(&ArmItmEvent::Extension {
        hardware: false,
        value: 2
    }));
    assert!(e.contains(&ArmItmEvent::Software {
        port: 1,
        page: Some(2),
        bytes: 1,
        value: 0x41
    }));
    assert!(e.contains(&ArmItmEvent::Software {
        port: 2,
        page: Some(2),
        bytes: 4,
        value: 0x12345678
    }));
    assert!(e.contains(&ArmItmEvent::EventCounter { flags: 0x22 }));
    assert!(e.contains(&ArmItmEvent::Exception {
        exception: 3,
        action: 1
    }));
    assert!(e.contains(&ArmItmEvent::Pc { value: 0x08001000 }));
    assert!(e.contains(&ArmItmEvent::Sleep));
    assert!(e.contains(&ArmItmEvent::Timestamp {
        delta: 3,
        control: 0
    }));
    assert!(e.contains(&ArmItmEvent::Timestamp {
        delta: 129,
        control: 1
    }));
    assert!(e.contains(&ArmItmEvent::Overflow));
}
#[test]
fn start_alignment_and_unknown_page_are_explicit() {
    assert!(decode(false, &[1, 0x41]).is_empty());
    assert_eq!(
        decode(true, &[1, 0x41]),
        vec![ArmItmEvent::Software {
            port: 0,
            page: None,
            bytes: 1,
            value: 0x41
        }]
    );
}
#[test]
fn unsupported_headers_overlong_packets_and_truncation_report_errors() {
    assert!(decode(true, &[0x04, 1, 0x41]).iter().any(|e| matches!(
        e,
        ArmItmEvent::Error {
            reason: ErrorReason::UnsupportedHeader,
            ..
        }
    )));
    assert!(
        decode(true, &[0xc0, 0x80, 0x80, 0x80, 0x80])
            .iter()
            .any(|e| matches!(
                e,
                ArmItmEvent::Error {
                    reason: ErrorReason::Overlong,
                    ..
                }
            ))
    );
    assert!(decode(true, &[3, 1, 2]).iter().any(|e| matches!(
        e,
        ArmItmEvent::Error {
            reason: ErrorReason::Aborted,
            ..
        }
    )));
    assert!(decode(true, &[0, 0, 0x80]).iter().any(|e| matches!(
        e,
        ArmItmEvent::Error {
            reason: ErrorReason::BadSync,
            ..
        }
    )));
    let e = decode(true, &[0x04, 1, 0x41, 0, 0, 0, 0, 0, 0x80, 1, 0x42]);
    assert_eq!(
        e.iter()
            .filter(|e| matches!(e, ArmItmEvent::Software { .. }))
            .count(),
        1
    );
}
#[test]
fn packet_state_is_independent_of_block_budget_and_gaps() {
    let block = waveform(&[
        0, 0, 0, 0, 0, 0x80, 3, 0x78, 0x56, 0x34, 0x12, 0xd0, 0x81, 1, 0x70,
    ]);
    assert_block_invariant(|config| build(false, config), &block, &[]);
    let gap = TickRange::new(Tick(775), Tick(930));
    assert_block_invariant(|config| build(false, config), &block, &[gap]);
}

#[test]
fn global_timestamp_fragments_follow_arm_packet_figures() {
    // Arm DDI0403E.e D4.2.5: GTS1 7/14/21/26 low bits, GTS2 48/64 widths.
    let events = decode(
        true,
        &[
            0x94, 0x25, 0x94, 0x81, 0x02, 0x94, 0xff, 0xff, 0xff, 0x7f, 0xb4, 0x81, 0x82, 0x83,
            0x01, 0xb4, 0x81, 0x82, 0x83, 0x84, 0x85, 0x07,
        ],
    );
    assert_eq!(
        events,
        vec![
            ArmItmEvent::GlobalLow {
                fragment: 0x25,
                significant_bits: 7,
                wrap: false,
                clock_change: false
            },
            ArmItmEvent::GlobalLow {
                fragment: 257,
                significant_bits: 14,
                wrap: false,
                clock_change: false
            },
            ArmItmEvent::GlobalLow {
                fragment: 0x3ff_ffff,
                significant_bits: 26,
                wrap: true,
                clock_change: true
            },
            ArmItmEvent::GlobalHigh {
                fragment: 1 | (2 << 7) | (3 << 14) | (1 << 21),
                timestamp_bits: 48
            },
            ArmItmEvent::GlobalHigh {
                fragment: 1 | (2 << 7) | (3 << 14) | (4 << 21) | (5 << 28) | (7 << 35),
                timestamp_bits: 64
            },
        ]
    );
    for malformed in [
        &[0xb4, 0][..],
        &[0xb4, 0x80, 0x80, 0x80, 2],
        &[0xb4, 0x80, 0x80, 0x80, 0x80, 0],
        &[0xb4, 0x80, 0x80, 0x80, 0x80, 0x80, 8],
    ] {
        assert!(decode(true, malformed).iter().any(|event| matches!(
            event,
            ArmItmEvent::Error {
                reason: ErrorReason::Malformed,
                ..
            }
        )));
    }
    assert!(
        decode(true, &[0x94, 0x80, 0x80, 0x80, 0x80])
            .iter()
            .any(|event| matches!(
                event,
                ArmItmEvent::Error {
                    reason: ErrorReason::Overlong,
                    ..
                }
            ))
    );
    assert!(decode(true, &[0xb4, 0x80]).iter().any(|event| matches!(
        event,
        ArmItmEvent::Error {
            reason: ErrorReason::Aborted,
            ..
        }
    )));
    let block = waveform(&[0x94, 0xff, 0xff, 0xff, 0x7f, 0xb4, 0x81, 0x82, 0x83, 1]);
    assert_block_invariant(|config| build(true, config), &block, &[]);
    assert_block_invariant(
        |config| build(true, config),
        &block,
        &[TickRange::new(Tick(260), Tick(400))],
    );
}
