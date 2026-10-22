// SPDX-License-Identifier: Apache-2.0

//! A new stream decoder has the same observable behavior compiled or interpreted.

mod common;

use common::{as_dyn, program};
use uiharu_core::testing::Harness;
use uiharu_core::{GraphConfig, InputRef, OutputRef, Props, Tick, TickRange, TimeBase};
use uiharu_interp::{DynEvent, Interpreter, ToDyn};
use uiharu_protocols::arm_trace_transport::{
    ArmTraceTransport, ArmTraceTransportConfig, TraceByteEvent, TraceFormat,
};
use uiharu_protocols::uart::{Uart, UartConfig, UartEvent, synth::UartWaveform};

fn build(interpreted: bool, format: TraceFormat, id: u8, config: GraphConfig) -> Harness<DynEvent> {
    Harness::build(1, TimeBase::new(96000), config, |graph, source| {
        let uart = graph.add(
            Uart::new(UartConfig {
                baud: 9600,
                ..UartConfig::default()
            })
            .expect("UART"),
        );
        graph.connect(source.samples, Uart::input(uart))?;
        if interpreted {
            let bridge = graph.add(ToDyn::<UartEvent>::new(
                Props::new()
                    .with("dataBits", 8i64)
                    .with("bitOrder", "lsb-first"),
            ));
            graph.connect(Uart::events(uart), InputRef::of_node(bridge, 0))?;
            let node = graph.add(
                Interpreter::new(
                    program("arm-trace-transport"),
                    &serde_json::json!({"format":format,"trace_id":id,"start_aligned":false}),
                )
                .expect("interpreter"),
            );
            graph.connect(OutputRef::of_node(bridge, 0), Interpreter::bus_input(node))?;
            Ok(Interpreter::events(node))
        } else {
            let node = graph.add(
                ArmTraceTransport::new(ArmTraceTransportConfig {
                    format,
                    trace_id: id,
                    start_aligned: false,
                })
                .expect("transport"),
            );
            graph.connect(Uart::events(uart), ArmTraceTransport::input(node))?;
            let bridge = graph.add(ToDyn::<TraceByteEvent>::new(Props::new()));
            graph.connect(
                ArmTraceTransport::events(node),
                InputRef::of_node(bridge, 0),
            )?;
            Ok(OutputRef::of_node(bridge, 0))
        }
    })
    .expect("pipeline")
}
#[test]
fn both_transport_profiles_sync_and_gaps_match() {
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
    w.bytes([
        0xff, 0xff, 0xff, 0x7f, 0x07, 0xaa, 0xa6, 0xa7, 0x2b, 0xa8, 0xff, 0x7f, 0x54, 0x52, 0x52,
        0x54, 0x07, 0xca, 0xc6, 0xc7, 0xc8, 0x1c, 0x07, 0xaa,
    ]);
    w.idle(20);
    let block = w.finish();
    for format in [TraceFormat::Raw, TraceFormat::Tpiu] {
        for id in [3, 21] {
            for gaps in [vec![], vec![TickRange::new(Tick(1000), Tick(1130))]] {
                for config in [
                    GraphConfig::default(),
                    GraphConfig {
                        queue_capacity: 1,
                        budget: 3,
                    },
                ] {
                    let mut compiled = build(false, format, id, config);
                    let mut interpreted = build(true, format, id, config);
                    compiled.feed_cut(&block, &gaps, &[13, 211, 1], &[0, 1, 7]);
                    compiled.end();
                    interpreted.feed_cut(&block, &gaps, &[13, 211, 1], &[0, 1, 7]);
                    interpreted.end();
                    assert_eq!(
                        as_dyn(&compiled.without_progress()),
                        as_dyn(&interpreted.without_progress())
                    );
                }
            }
        }
    }
}
