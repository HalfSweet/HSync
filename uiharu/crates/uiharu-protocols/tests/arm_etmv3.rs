// SPDX-License-Identifier: Apache-2.0

//! Byte vectors use Arm IHI0014Q Chapter7 packet tables, independent of the decoder.
use uiharu_core::testing::{Harness, assert_block_invariant};
use uiharu_core::{GraphConfig, LogicBlock, Tick, TickRange, TimeBase};
use uiharu_protocols::arm_etmv3::{
    ArmEtmv3, ArmEtmv3Config, ArmEtmv3Event, Atom, BranchEncoding, ErrorReason, Isa,
};
use uiharu_protocols::arm_trace_transport::{
    ArmTraceTransport, ArmTraceTransportConfig, TraceFormat,
};
use uiharu_protocols::uart::{Uart, UartConfig, synth::UartWaveform};
fn build(options: ArmEtmv3Config, config: GraphConfig) -> Harness<ArmEtmv3Event> {
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
        let itm = graph.add(ArmEtmv3::new(options).expect("ETM"));
        graph.connect(ArmTraceTransport::events(transport), ArmEtmv3::input(itm))?;
        Ok(ArmEtmv3::events(itm))
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
fn decode(options: ArmEtmv3Config, bytes: &[u8]) -> Vec<ArmEtmv3Event> {
    let mut h = build(options, GraphConfig::default());
    h.run_all(&waveform(bytes));
    h.events().into_iter().map(|e| e.payload).collect()
}
fn options() -> ArmEtmv3Config {
    ArmEtmv3Config {
        start_aligned: true,
        context_bytes: 0,
        cycle_accurate: false,
        minor_version: 5,
        branch_encoding: BranchEncoding::Alternative,
        data_addresses: false,
        timestamp_bits: 48,
    }
}
fn atoms(events: &[ArmEtmv3Event]) -> Vec<Atom> {
    events
        .iter()
        .filter_map(|event| {
            if let ArmEtmv3Event::Atom { atom, .. } = event {
                Some(*atom)
            } else {
                None
            }
        })
        .collect()
}
#[test]
fn official_atom_examples_and_cycle_formats() {
    // IHI0014Q Examples7-1/7-2, Tables7-2/7-3. No instruction PC is fabricated.
    assert_eq!(
        atoms(&decode(options(), &[0xc8, 0x8a])),
        vec![Atom::E, Atom::E, Atom::N, Atom::N, Atom::E]
    );
    let mut config = options();
    config.cycle_accurate = true;
    assert_eq!(
        atoms(&decode(config, &[0xc8, 0x8a, 0xe8, 0x92])),
        vec![
            Atom::W,
            Atom::E,
            Atom::W,
            Atom::E,
            Atom::W,
            Atom::N,
            Atom::W,
            Atom::N,
            Atom::E,
            Atom::W,
            Atom::W,
            Atom::W,
            Atom::E,
            Atom::E
        ]
    );
    assert!(decode(config, &[0x80]).iter().any(|event| matches!(
        event,
        ArmEtmv3Event::Error {
            reason: ErrorReason::Malformed,
            ..
        }
    )));
    config.minor_version = 0;
    assert_eq!(atoms(&decode(config, &[0x80])), vec![Atom::W]);
}
#[test]
fn isync_context_and_original_alternative_branch_geometry() {
    for encoding in [BranchEncoding::Original, BranchEncoding::Alternative] {
        let mut config = options();
        config.branch_encoding = encoding;
        config.context_bytes = 2;
        let events = decode(
            config,
            &[
                8, 0x34, 0x12, 0x09, 0x01, 0x10, 0, 0x08, 0x25, 0x6e, 0x78, 0x56,
            ],
        );
        assert!(events.iter().any(|event| matches!(
            event,
            ArmEtmv3Event::Isync {
                address: 0x08001000,
                context: Some(0x1234),
                isa: Isa::Thumb,
                nonsecure: true,
                ..
            }
        )));
        assert!(events.iter().any(|event| matches!(
            event,
            ArmEtmv3Event::Branch {
                address: Some(0x08001024),
                significant_bits: 7,
                ..
            }
        )));
        assert!(
            events
                .iter()
                .any(|event| matches!(event, ArmEtmv3Event::Context { value: 0x5678 }))
        );
    }
    // Figure7-19 compressed Thumb exception branch: two address bytes + exception byte.
    let events = decode(options(), &[8, 1, 1, 0x10, 0, 8, 0x81, 0x61, 0x13]);
    assert!(events.iter().any(|event| matches!(
        event,
        ArmEtmv3Event::Branch {
            address: Some(0x08001080),
            exception: 9,
            nonsecure: Some(true),
            ..
        }
    )));
    // Figure7-11 full Thumb address gives context even without prior I-sync.
    let events = decode(options(), &[0x81, 0x80, 0x80, 0x80, 0x10]);
    assert!(events.iter().any(|event| matches!(
        event,
        ArmEtmv3Event::Branch {
            address: Some(0),
            isa: Some(Isa::Thumb),
            significant_bits: 32,
            ..
        }
    )));
}
#[test]
fn data_addresses_tags_timestamp_components_and_marker_fields() {
    let mut config = options();
    config.data_addresses = true;
    let events = decode(
        config,
        &[
            0x2e, 0x80, 0x80, 0x80, 0x80, 0x11, 0x78, 0x56, 0x34, 0x12, 0x54, 0x24, 0xaa, 0x42,
            0x81, 1, 0x3c, 7, 0x0c, 0x50, 0x62, 0x66, 0x76, 0x7e,
        ],
    );
    assert!(events.iter().any(|event| matches!(
        event,
        ArmEtmv3Event::Data {
            value: Some(0x12345678),
            address: Some(0x10000000),
            big_endian: Some(true),
            bytes: 4,
            ..
        }
    )));
    assert!(events.iter().any(|event| matches!(
        event,
        ArmEtmv3Event::Data {
            tag: 1,
            placeholder: true,
            value: None,
            ..
        }
    )));
    assert!(events.iter().any(|event| matches!(
        event,
        ArmEtmv3Event::Data {
            tag: 1,
            placeholder: false,
            value: Some(0xaa),
            ..
        }
    )));
    assert!(events.iter().any(|event| matches!(
        event,
        ArmEtmv3Event::Timestamp {
            fragment: 129,
            significant_bits: 14,
            clock_change: false
        }
    )));
    assert!(
        events
            .iter()
            .any(|event| matches!(event, ArmEtmv3Event::Vmid { value: 7 }))
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, ArmEtmv3Event::Marker { .. }))
            .count(),
        6
    );
}
#[test]
fn loss_alignment_truncation_configuration_and_long_continuations() {
    assert!(
        decode(
            ArmEtmv3Config {
                start_aligned: false,
                ..options()
            },
            &[0xc8]
        )
        .is_empty()
    );
    assert_eq!(
        atoms(&decode(
            ArmEtmv3Config {
                start_aligned: false,
                ..options()
            },
            &[0, 0, 0, 0, 0, 0x80, 0xc8]
        )),
        vec![Atom::E, Atom::E, Atom::N]
    );
    for bytes in [
        &[4, 0x80, 0x80, 0x80, 0x80, 0x10][..],
        &[8, 1, 1, 2][..],
        &[0x72][..],
    ] {
        assert!(
            decode(options(), bytes)
                .iter()
                .any(|event| matches!(event, ArmEtmv3Event::Error { .. }))
        );
    }
    for bad in [
        ArmEtmv3Config {
            context_bytes: 3,
            ..options()
        },
        ArmEtmv3Config {
            minor_version: 6,
            ..options()
        },
        ArmEtmv3Config {
            timestamp_bits: 32,
            ..options()
        },
    ] {
        assert!(ArmEtmv3::new(bad).is_err());
    }
    let block = waveform(&[
        0, 0, 0, 0, 0, 0x80, 8, 1, 1, 0x10, 0, 8, 0x25, 0xc8, 4, 0x81, 1, 0x42, 0x81, 1,
    ]);
    assert_block_invariant(|graph| build(options(), graph), &block, &[]);
    assert_block_invariant(
        |graph| build(options(), graph),
        &block,
        &[TickRange::new(Tick(1000), Tick(1100))],
    );
}
