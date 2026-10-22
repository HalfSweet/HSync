// SPDX-License-Identifier: Apache-2.0

//! A new stream decoder has the same observable behavior compiled or interpreted.

mod common;

use common::{as_dyn, program};
use uiharu_core::testing::Harness;
use uiharu_core::{GraphConfig, InputRef, OutputRef, Props, Tick, TickRange, TimeBase};
use uiharu_interp::{DynEvent, Interpreter, ToDyn};
use uiharu_protocols::arm_etmv3::{ArmEtmv3, ArmEtmv3Config, ArmEtmv3Event};
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
fn build(interpreted: bool, options: ArmEtmv3Config, graph: GraphConfig) -> Harness<DynEvent> {
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
                    program("arm-etmv3"),
                    &serde_json::to_value(options).expect("config"),
                )
                .expect("interpreter"),
            );
            graph.connect(OutputRef::of_node(bridge, 0), Interpreter::bus_input(node))?;
            Ok(Interpreter::events(node))
        } else {
            let pdu = graph.add(ArmEtmv3::new(options).expect("ETM"));
            graph.connect(ArmTraceTransport::events(transport), ArmEtmv3::input(pdu))?;
            let bridge = graph.add(ToDyn::<ArmEtmv3Event>::new(Props::new()));
            graph.connect(ArmEtmv3::events(pdu), InputRef::of_node(bridge, 0))?;
            Ok(OutputRef::of_node(bridge, 0))
        }
    })
    .expect("pipeline")
}

#[test]
fn etm_packets_context_gaps_and_budgets_match() {
    use uiharu_protocols::arm_etmv3::BranchEncoding;
    for encoding in [BranchEncoding::Original, BranchEncoding::Alternative] {
        for context_bytes in [0, 1, 2, 4] {
            let options = ArmEtmv3Config {
                start_aligned: false,
                context_bytes,
                cycle_accurate: true,
                minor_version: 5,
                branch_encoding: encoding,
                data_addresses: true,
                timestamp_bits: 64,
            };
            let mut wave = UartWaveform::new(config(), 1, 10, Tick::ZERO);
            wave.idle(20);
            wave.bytes([0, 0, 0, 0, 0, 0x80, 8]);
            wave.bytes(std::iter::repeat_n(0x12, usize::from(context_bytes)));
            wave.bytes([
                1, 1, 0x10, 0, 8, 0x25, 0xc8, 0x8a, 0xe8, 4, 0x81, 1, 0x2e, 0x80, 0x80, 0x80, 0x80,
                1, 0x78, 0x56, 0x34, 0x12, 0x42, 0x81, 1, 0x3c, 7, 0x72,
            ]);
            wave.idle(20);
            let block = wave.finish();
            for gaps in [vec![], vec![TickRange::new(Tick(1000), Tick(1100))]] {
                for graph in [
                    GraphConfig::default(),
                    GraphConfig {
                        queue_capacity: 1,
                        budget: 3,
                    },
                ] {
                    let mut compiled = build(false, options, graph);
                    let mut interpreted = build(true, options, graph);
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
