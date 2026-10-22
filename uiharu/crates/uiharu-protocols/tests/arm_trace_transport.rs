// SPDX-License-Identifier: Apache-2.0

//! Independent CoreSight IHI0029D D4.2.1 example and synchronization packet vectors.
use uiharu_core::testing::{Harness, assert_block_invariant};
use uiharu_core::{GraphConfig, LogicBlock, Tick, TickRange, TimeBase};
use uiharu_protocols::arm_trace_transport::{
    ArmTraceTransport, ArmTraceTransportConfig, TraceByteError, TraceByteEvent, TraceFormat,
};
use uiharu_protocols::uart::{
    Uart, UartConfig,
    synth::{Faults, UartWaveform},
};
const FRAME: [u8; 16] = [
    0x07, 0xaa, 0xa6, 0xa7, 0x2b, 0xa8, 0x54, 0x52, 0x52, 0x54, 0x07, 0xca, 0xc6, 0xc7, 0xc8, 0x1c,
];
fn config(id: u8, aligned: bool) -> ArmTraceTransportConfig {
    ArmTraceTransportConfig {
        format: TraceFormat::Tpiu,
        trace_id: id,
        start_aligned: aligned,
    }
}
fn build(c: ArmTraceTransportConfig, g: GraphConfig) -> Harness<TraceByteEvent> {
    Harness::build(1, TimeBase::new(96000), g, |graph, source| {
        let uart = graph.add(
            Uart::new(UartConfig {
                baud: 9600,
                ..UartConfig::default()
            })
            .expect("UART"),
        );
        graph.connect(source.samples, Uart::input(uart))?;
        let transport = graph.add(ArmTraceTransport::new(c).expect("transport"));
        graph.connect(Uart::events(uart), ArmTraceTransport::input(transport))?;
        Ok(ArmTraceTransport::events(transport))
    })
    .expect("pipeline")
}
fn wave(bytes: &[u8]) -> LogicBlock {
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
fn decode(c: ArmTraceTransportConfig, bytes: &[u8]) -> Vec<TraceByteEvent> {
    let mut h = build(c, GraphConfig::default());
    h.run_all(&wave(bytes));
    h.events().into_iter().map(|e| e.payload).collect()
}
fn data(e: &[TraceByteEvent]) -> Vec<u8> {
    e.iter()
        .filter_map(|e| {
            if let TraceByteEvent::Byte { value } = e {
                Some(*value)
            } else {
                None
            }
        })
        .collect()
}
#[test]
fn official_formatter_example_reconstructs_auxiliary_bits_and_delayed_id() {
    assert_eq!(
        data(&decode(config(3, true), &FRAME)),
        [0xaa, 0xa6, 0xa7, 0xa8, 0xca, 0xc6, 0xc7, 0xc8]
    );
    assert_eq!(
        data(&decode(config(0x15, true), &FRAME)),
        [0x55, 0x52, 0x53, 0x54]
    );
    assert!(data(&decode(config(1, true), &FRAME)).is_empty());
}
#[test]
fn frame_and_halfword_sync_are_removed_without_losing_frame_position() {
    let mut stream = vec![0x12, 0x34, 0xff, 0xff, 0xff, 0x7f];
    stream.extend_from_slice(&FRAME[..4]);
    stream.extend([0xff, 0x7f]);
    stream.extend_from_slice(&FRAME[4..]);
    assert_eq!(
        data(&decode(config(3, false), &stream)),
        [0xaa, 0xa6, 0xa7, 0xa8, 0xca, 0xc6, 0xc7, 0xc8]
    );
    assert!(data(&decode(config(3, false), &FRAME)).is_empty());
    let mut partial = vec![0x07, 0xaa, 0xff, 0xff, 0xff, 0x7f];
    partial.extend(FRAME);
    let e = decode(config(3, true), &partial);
    assert!(e.iter().any(|e| matches!(
        e,
        TraceByteEvent::Error {
            reason: TraceByteError::Alignment
        }
    )));
    assert_eq!(data(&e), [0xaa, 0xa6, 0xa7, 0xa8, 0xca, 0xc6, 0xc7, 0xc8]);
}
#[test]
fn final_id_switch_applies_to_next_frame_and_partial_frame_never_leaks() {
    let mut first = FRAME;
    first[14] = 0x2b;
    first[15] |= 0x80;
    let mut second = [0u8; 16];
    second[0] = 0x42;
    second[1] = 0x44;
    let mut bytes = first.to_vec();
    bytes.extend(second);
    let e = decode(config(0x15, true), &bytes);
    let d = data(&e);
    assert_eq!(&d[..4], [0x55, 0x52, 0x53, 0x54]);
    assert_eq!(&d[4..6], [0x42, 0x44]);
    let e = decode(config(3, true), &FRAME[..10]);
    assert!(data(&e).is_empty());
    assert!(e.iter().any(|e| matches!(
        e,
        TraceByteEvent::Error {
            reason: TraceByteError::Truncated
        }
    )));
}
#[test]
fn raw_transport_preserves_uart_errors_and_configuration_validation() {
    let c = ArmTraceTransportConfig {
        format: TraceFormat::Raw,
        ..config(1, false)
    };
    assert_eq!(data(&decode(c, &[0xff, 0, 0x80])), [0xff, 0, 0x80]);
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
    w.frame(
        0x41,
        Faults {
            framing: true,
            parity: false,
        },
    );
    w.idle(20);
    w.break_(200);
    w.idle(20);
    w.byte(0x42);
    w.idle(20);
    let mut h = build(c, GraphConfig::default());
    h.run_all(&w.finish());
    let e: Vec<_> = h.events().into_iter().map(|e| e.payload).collect();
    assert!(e.iter().any(|e| matches!(
        e,
        TraceByteEvent::Error {
            reason: TraceByteError::Framing
        }
    )));
    assert!(e.iter().any(|e| matches!(
        e,
        TraceByteEvent::Error {
            reason: TraceByteError::Break
        }
    )));
    assert_eq!(data(&e), [0x42]);
    for id in [0, 112, 255] {
        assert!(ArmTraceTransport::new(config(id, false)).is_err());
    }
}
#[test]
fn formatter_state_is_invariant_under_blocks_gaps_and_backpressure() {
    let mut bytes = vec![0xff, 0xff, 0xff, 0x7f];
    bytes.extend(FRAME);
    bytes.extend(FRAME);
    let block = wave(&bytes);
    assert_block_invariant(|g| build(config(3, false), g), &block, &[]);
    assert_block_invariant(
        |g| build(config(3, false), g),
        &block,
        &[TickRange::new(Tick(1150), Tick(1280))],
    );
}
