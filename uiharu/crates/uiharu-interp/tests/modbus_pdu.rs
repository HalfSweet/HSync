// SPDX-License-Identifier: Apache-2.0

//! A new stream decoder has the same observable behavior compiled or interpreted.

mod common;

use common::{as_dyn, program};
use uiharu_core::testing::Harness;
use uiharu_core::{GraphConfig, InputRef, OutputRef, Props, Tick, TickRange, TimeBase};
use uiharu_interp::{DynEvent, Interpreter, ToDyn};
use uiharu_protocols::modbus_pdu::{Direction, ModbusPdu, ModbusPduConfig, ModbusPduEvent};
use uiharu_protocols::modbus_rtu::{ModbusEvent, ModbusRtu, synth::rtu_frame};
use uiharu_protocols::uart::{Parity, Uart, UartConfig, synth::UartWaveform};

fn config() -> UartConfig {
    UartConfig {
        baud: 9600,
        parity: Parity::Even,
        ..UartConfig::default()
    }
}
fn build(interpreted: bool, direction: Direction, graph: GraphConfig) -> Harness<DynEvent> {
    Harness::build(1, TimeBase::new(96000), graph, |graph, source| {
        let uart = graph.add(Uart::new(config()).expect("UART"));
        graph.connect(source.samples, Uart::input(uart))?;
        let rtu = graph.add(ModbusRtu::new());
        graph.connect(Uart::events(uart), ModbusRtu::input(rtu))?;
        if interpreted {
            let bridge = graph.add(ToDyn::<ModbusEvent>::new(Props::new()));
            graph.connect(ModbusRtu::events(rtu), InputRef::of_node(bridge, 0))?;
            let node = graph.add(
                Interpreter::new(
                    program("modbus-pdu"),
                    &serde_json::json!({"direction":direction}),
                )
                .expect("interpreter"),
            );
            graph.connect(OutputRef::of_node(bridge, 0), Interpreter::bus_input(node))?;
            Ok(Interpreter::events(node))
        } else {
            let pdu = graph.add(ModbusPdu::new(ModbusPduConfig { direction }).expect("PDU"));
            graph.connect(ModbusRtu::events(rtu), ModbusPdu::input(pdu))?;
            let bridge = graph.add(ToDyn::<ModbusPduEvent>::new(Props::new()));
            graph.connect(ModbusPdu::events(pdu), InputRef::of_node(bridge, 0))?;
            Ok(OutputRef::of_node(bridge, 0))
        }
    })
    .expect("pipeline")
}

#[test]
fn pdu_streams_match_for_roles_errors_batches_gaps_and_budgets() {
    for direction in [Direction::Request, Direction::Response] {
        let mut wave = UartWaveform::new(config(), 1, 10, Tick::ZERO);
        wave.idle(500);
        let mut large = vec![3, 100];
        for value in 0u16..50 {
            large.extend_from_slice(&value.to_be_bytes());
        }
        for pdu in [
            vec![3, 0, 0x6b, 0, 3],
            large,
            vec![0x83, 2],
            vec![0x41, 0xff],
            vec![20, 12, 5, 6, 0x0d, 0xfe, 0, 0x20, 5, 6, 0x33, 0xcd, 0, 0x40],
            vec![21, 9, 6, 0, 4, 0, 7, 0, 1, 0x12, 0x34],
            vec![24, 0, 6, 0, 2, 0x12, 0x34, 0xab, 0xcd],
            vec![43, 14, 1, 1, 0, 0, 1, 0, 3, 65, 66, 67],
        ] {
            wave.bytes(rtu_frame(0x11, &pdu));
            wave.idle(500);
        }
        let block = wave.finish();
        for gaps in [vec![], vec![TickRange::new(Tick(5000), Tick(5300))]] {
            for graph in [
                GraphConfig::default(),
                GraphConfig {
                    queue_capacity: 1,
                    budget: 3,
                },
            ] {
                let mut compiled = build(false, direction, graph);
                let mut interpreted = build(true, direction, graph);
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
