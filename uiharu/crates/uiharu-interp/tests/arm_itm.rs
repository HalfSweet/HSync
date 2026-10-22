// SPDX-License-Identifier: Apache-2.0

//! A new stream decoder has the same observable behavior compiled or interpreted.

mod common;

use common::{as_dyn, program};
use uiharu_core::testing::Harness;
use uiharu_core::{GraphConfig, InputRef, OutputRef, Props, Tick, TickRange, TimeBase};
use uiharu_interp::{DynEvent, Interpreter, ToDyn};
use uiharu_protocols::arm_itm::{ArmItm, ArmItmConfig, ArmItmEvent};
use uiharu_protocols::arm_trace_transport::{
    ArmTraceTransport, ArmTraceTransportConfig, TraceByteEvent, TraceFormat,
};
use uiharu_protocols::uart::{Uart, UartConfig, synth::UartWaveform};

fn config() -> UartConfig {
    UartConfig {
        baud: 9600,
        ..UartConfig::default()
    }
}
fn build(interpreted: bool, aligned: bool, graph: GraphConfig) -> Harness<DynEvent> {
    Harness::build(1, TimeBase::new(96000), graph, |graph, source| {
        let uart = graph.add(Uart::new(config()).expect("UART"));
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
        if interpreted {
            let bridge = graph.add(ToDyn::<TraceByteEvent>::new(Props::new()));
            graph.connect(
                ArmTraceTransport::events(transport),
                InputRef::of_node(bridge, 0),
            )?;
            let node = graph.add(
                Interpreter::new(
                    program("arm-itm"),
                    &serde_json::json!({"start_aligned":aligned}),
                )
                .expect("interpreter"),
            );
            graph.connect(OutputRef::of_node(bridge, 0), Interpreter::bus_input(node))?;
            Ok(Interpreter::events(node))
        } else {
            let pdu = graph.add(
                ArmItm::new(ArmItmConfig {
                    start_aligned: aligned,
                })
                .expect("ITM"),
            );
            graph.connect(ArmTraceTransport::events(transport), ArmItm::input(pdu))?;
            let bridge = graph.add(ToDyn::<ArmItmEvent>::new(Props::new()));
            graph.connect(ArmItm::events(pdu), InputRef::of_node(bridge, 0))?;
            Ok(OutputRef::of_node(bridge, 0))
        }
    })
    .expect("pipeline")
}

#[test]
fn itm_packets_errors_gaps_and_budgets_match() {
    for aligned in [false, true] {
        let mut wave = UartWaveform::new(config(), 1, 10, Tick::ZERO);
        wave.idle(20);
        wave.bytes([
            0, 0, 0, 0, 0, 0x80, 0x28, 3, 0x78, 0x56, 0x34, 0x12, 0xd0, 0x81, 1, 0x70, 0x94, 0xff,
            0xff, 0xff, 0x7f, 0xb4, 0x81, 0x82, 0x83, 1, 0xb4, 0x81, 0x82, 0x83, 0x84, 0x85, 7,
            0x04, 0, 0, 0, 0, 0, 0x80, 1, 0x42, 3, 1,
        ]);
        wave.idle(20);
        let block = wave.finish();
        for gaps in [vec![], vec![TickRange::new(Tick(800), Tick(950))]] {
            for graph in [
                GraphConfig::default(),
                GraphConfig {
                    queue_capacity: 1,
                    budget: 3,
                },
            ] {
                let mut compiled = build(false, aligned, graph);
                let mut interpreted = build(true, aligned, graph);
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
