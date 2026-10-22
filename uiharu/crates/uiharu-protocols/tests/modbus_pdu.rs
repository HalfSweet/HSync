// SPDX-License-Identifier: Apache-2.0

//! Application vectors follow MODBUS Application Protocol V1.1b3, §6; RTU and PDU
//! assertions are independent of the PDU decoder. The same lower RTU contract is generated.

use uiharu_core::testing::{Harness, assert_block_invariant};
use uiharu_core::{GraphConfig, LogicBlock, Tick, TickRange, TimeBase};
use uiharu_dsl_protocols::modbus_pdu::{
    Direction, FieldName, ModbusPdu, ModbusPduConfig, ModbusPduEvent,
};
use uiharu_protocols::modbus_rtu::{ModbusRtu, synth::rtu_frame};
use uiharu_protocols::uart::{Parity, Uart, UartConfig, synth::UartWaveform};

fn line() -> UartConfig {
    UartConfig {
        baud: 9600,
        parity: Parity::Even,
        ..UartConfig::default()
    }
}
fn build(direction: Direction, config: GraphConfig) -> Harness<ModbusPduEvent> {
    Harness::build(1, TimeBase::new(96000), config, |graph, source| {
        let uart = graph.add(Uart::new(line()).expect("UART config"));
        graph.connect(source.samples, Uart::input(uart))?;
        let rtu = graph.add(ModbusRtu::new());
        graph.connect(Uart::events(uart), ModbusRtu::input(rtu))?;
        let pdu = graph.add(ModbusPdu::new(ModbusPduConfig { direction }).expect("PDU config"));
        graph.connect(ModbusRtu::events(rtu), ModbusPdu::input(pdu))?;
        Ok(ModbusPdu::events(pdu))
    })
    .expect("pipeline")
}
fn waveform(pdus: &[&[u8]]) -> LogicBlock {
    let mut wave = UartWaveform::new(line(), 1, 10, Tick::ZERO);
    wave.idle(500);
    for pdu in pdus {
        wave.bytes(rtu_frame(0x11, pdu));
        wave.idle(500);
    }
    wave.finish()
}
fn events(direction: Direction, pdu: &[u8]) -> Vec<ModbusPduEvent> {
    let mut harness = build(direction, GraphConfig::default());
    harness.run_all(&waveform(&[pdu]));
    harness
        .events()
        .into_iter()
        .map(|event| event.payload)
        .collect()
}

#[test]
fn official_read_holding_register_example_has_exact_fields() {
    // §6.3: 11 03 006B 0003; register zero is 0xCD6B in the response example.
    let request = events(Direction::Request, &[3, 0, 0x6b, 0, 3]);
    assert!(request.contains(&ModbusPduEvent::Field {
        frame: 1,
        field: FieldName::Address,
        value: 107
    }));
    assert!(request.contains(&ModbusPduEvent::Field {
        frame: 1,
        field: FieldName::Quantity,
        value: 3
    }));
    let response = events(Direction::Response, &[3, 6, 0xcd, 0x6b, 0, 0, 0, 0x75]);
    assert!(response.contains(&ModbusPduEvent::Register {
        frame: 1,
        index: 0,
        value: 0xcd6b
    }));
    assert!(response.contains(&ModbusPduEvent::Register {
        frame: 1,
        index: 2,
        value: 0x75
    }));
    assert!(matches!(
        response.last(),
        Some(ModbusPduEvent::End {
            valid: true,
            length_ok: true,
            ..
        })
    ));
}

#[test]
fn every_declared_function_has_request_and_response_shapes() {
    let vectors: &[(&[u8], &[u8])] = &[
        (
            &[1, 0, 0x13, 0, 0x25],
            &[1, 5, 0xcd, 0x6b, 0xb2, 0x0e, 0x1b],
        ),
        (&[2, 0, 0xc4, 0, 0x16], &[2, 3, 0xac, 0xdb, 0x35]),
        (
            &[3, 0, 0x6b, 0, 3],
            &[3, 6, 0xae, 0x41, 0x56, 0x52, 0x43, 0x40],
        ),
        (&[4, 0, 8, 0, 1], &[4, 2, 0, 10]),
        (&[5, 0, 0xac, 0xff, 0], &[5, 0, 0xac, 0xff, 0]),
        (&[6, 0, 1, 0, 3], &[6, 0, 1, 0, 3]),
        (&[7], &[7, 0x6d]),
        (&[8, 0, 0, 0, 0xa5], &[8, 0, 0, 0, 0xa5]),
        (&[11], &[11, 0, 0, 0, 10]),
        (&[12], &[12, 8, 0, 0, 0, 10, 0, 11, 0x12, 0x13]),
        (&[15, 0, 0x13, 0, 10, 2, 0xcd, 1], &[15, 0, 0x13, 0, 10]),
        (&[16, 0, 1, 0, 2, 4, 0, 10, 1, 2], &[16, 0, 1, 0, 2]),
        (&[17], &[17, 3, 1, 0xff, 0x42]),
        (
            &[22, 0, 4, 0xf2, 0xff, 0, 0x25],
            &[22, 0, 4, 0xf2, 0xff, 0, 0x25],
        ),
        (
            &[
                23, 0, 3, 0, 6, 0, 0x0e, 0, 3, 6, 0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc,
            ],
            &[23, 12, 0, 1, 0, 2, 0, 3, 0, 4, 0, 5, 0, 6],
        ),
    ];
    for (request, response) in vectors {
        for (direction, pdu) in [
            (Direction::Request, *request),
            (Direction::Response, *response),
        ] {
            let decoded = events(direction, pdu);
            assert!(
                matches!(
                    decoded.last(),
                    Some(ModbusPduEvent::End {
                        valid: true,
                        supported: true,
                        length_ok: true,
                        ..
                    })
                ),
                "{pdu:02x?}: {decoded:?}"
            );
        }
    }
}

#[test]
fn exceptions_lengths_invalid_coil_values_and_reserved_functions_are_honest() {
    let exception = events(Direction::Response, &[0x83, 2]);
    assert!(exception.contains(&ModbusPduEvent::Field {
        frame: 1,
        field: FieldName::Exception,
        value: 2
    }));
    assert!(matches!(
        exception.last(),
        Some(ModbusPduEvent::End { valid: true, .. })
    ));
    for (direction, pdu) in [
        (Direction::Request, &[3, 0, 1, 0][..]),
        (Direction::Response, &[3, 4, 0, 1][..]),
        (Direction::Request, &[5, 0, 1, 0x12, 0x34][..]),
        (Direction::Request, &[3, 0, 0, 0, 0][..]),
    ] {
        assert!(matches!(
            events(direction, pdu).last(),
            Some(ModbusPduEvent::End {
                valid: false,
                length_ok: false,
                ..
            })
        ));
    }
    assert!(matches!(
        events(Direction::Response, &[0x41, 1, 2]).last(),
        Some(ModbusPduEvent::End {
            supported: false,
            valid: false,
            ..
        })
    ));
}

#[test]
fn bad_crc_truncated_frames_and_gaps_never_become_valid_pdus() {
    let mut wave = UartWaveform::new(line(), 1, 10, Tick::ZERO);
    wave.idle(500);
    let mut frame = rtu_frame(0x11, &[3, 0, 0x6b, 0, 3]);
    frame[7] ^= 1;
    wave.bytes(frame);
    wave.idle(500);
    let mut harness = build(Direction::Request, GraphConfig::default());
    harness.run_all(&wave.finish());
    assert!(matches!(
        harness.events().last().map(|event| event.payload),
        Some(ModbusPduEvent::End { valid: false, .. })
    ));
    let block = waveform(&[&[3, 0, 0x6b, 0, 3]]);
    let cut = block
        .slice(TickRange::new(Tick::ZERO, Tick(1000)))
        .expect("slice");
    let mut harness = build(Direction::Request, GraphConfig::default());
    harness.run_all(&cut);
    assert!(
        harness
            .events()
            .iter()
            .all(|event| !matches!(event.payload, ModbusPduEvent::End { valid: true, .. }))
    );
    let gap = TickRange::new(Tick(800), Tick(1000));
    let mut harness = build(Direction::Request, GraphConfig::default());
    harness.feed_with_gaps(&block, &[gap], &[17]);
    harness.end();
    assert!(
        harness
            .events()
            .iter()
            .all(|event| !matches!(event.payload, ModbusPduEvent::End { valid: true, .. }))
    );
}

#[test]
fn word_pairs_crossing_transport_batches_remain_incremental_and_invariant() {
    let mut pdu = vec![3, 100];
    for value in 0u16..50 {
        pdu.extend_from_slice(&value.to_be_bytes());
    }
    let block = waveform(&[&pdu, &[0x83, 2]]);
    let mut harness = build(Direction::Response, GraphConfig::default());
    harness.run_all(&block);
    let registers: Vec<_> = harness
        .events()
        .into_iter()
        .filter_map(|event| match event.payload {
            ModbusPduEvent::Register { value, .. } => Some(value),
            _ => None,
        })
        .collect();
    assert_eq!(registers, (0u16..50).collect::<Vec<_>>());
    let prefix = block
        .slice(TickRange::new(Tick::ZERO, Tick(7000)))
        .expect("prefix");
    let mut live = build(Direction::Response, GraphConfig::default());
    live.feed(&prefix, &[29]);
    live.collect();
    assert!(
        live.events()
            .iter()
            .any(|event| matches!(event.payload, ModbusPduEvent::Register { .. }))
    );
    assert!(
        live.events()
            .iter()
            .all(|event| !matches!(event.payload, ModbusPduEvent::End { .. }))
    );
    assert_block_invariant(|config| build(Direction::Response, config), &block, &[]);
    assert_block_invariant(
        |config| build(Direction::Response, config),
        &block,
        &[TickRange::new(Tick(4000), Tick(4300))],
    );
}

#[test]
fn file_fifo_and_device_identification_vectors_stream_all_subrecords() {
    let cases: &[(Direction, &[u8], &[u16])] = &[
        // §6.14 example: files 4 and 3, starting records 1 and 9, lengths 2 and 2.
        (
            Direction::Request,
            &[20, 14, 6, 0, 4, 0, 1, 0, 2, 6, 0, 3, 0, 9, 0, 2],
            &[],
        ),
        (
            Direction::Response,
            &[20, 12, 5, 6, 0x0d, 0xfe, 0, 0x20, 5, 6, 0x33, 0xcd, 0, 0x40],
            &[0x0dfe, 0x20, 0x33cd, 0x40],
        ),
        // §6.15 write file 4, record 7, three registers; response echoes the request.
        (
            Direction::Request,
            &[21, 13, 6, 0, 4, 0, 7, 0, 3, 6, 0xaf, 4, 0xbe, 0x10, 0x0d],
            &[0x06af, 0x04be, 0x100d],
        ),
        (
            Direction::Response,
            &[21, 13, 6, 0, 4, 0, 7, 0, 3, 6, 0xaf, 4, 0xbe, 0x10, 0x0d],
            &[0x06af, 0x04be, 0x100d],
        ),
        (Direction::Request, &[24, 0, 0x20], &[]),
        (
            Direction::Response,
            &[24, 0, 6, 0, 2, 0x12, 0x34, 0xab, 0xcd],
            &[0x1234, 0xabcd],
        ),
        (Direction::Request, &[43, 14, 1, 0], &[]),
        (
            Direction::Response,
            &[
                43, 14, 1, 1, 0, 0, 2, 0, 3, b'A', b'B', b'C', 1, 2, b'X', b'Y',
            ],
            &[],
        ),
    ];
    for &(direction, pdu, expected) in cases {
        let actual = events(direction, pdu);
        assert!(
            matches!(
                actual.last(),
                Some(ModbusPduEvent::End {
                    valid: true,
                    supported: true,
                    ..
                })
            ),
            "{direction:?} {pdu:?}: {actual:?}"
        );
        let words: Vec<_> = actual
            .iter()
            .filter_map(|e| match e {
                ModbusPduEvent::Register { value, .. } => Some(*value),
                _ => None,
            })
            .collect();
        assert_eq!(words, expected, "{pdu:?}");
    }
    let e = events(
        Direction::Response,
        &[43, 14, 1, 1, 0, 0, 1, 0, 3, b'A', b'B', b'C'],
    );
    assert!(e.contains(&ModbusPduEvent::Field {
        frame: 1,
        field: FieldName::ObjectCount,
        value: 1
    }));
    assert!(e.contains(&ModbusPduEvent::Field {
        frame: 1,
        field: FieldName::ObjectLength,
        value: 3
    }));
}

#[test]
fn malformed_record_lengths_fifo_counts_and_mei_objects_are_not_valid() {
    for (direction, pdu) in [
        (Direction::Request, vec![20, 7, 5, 0, 1, 0, 0, 0, 1]),
        (Direction::Request, vec![20, 7, 6, 0, 1, 0, 0, 0, 0]),
        (Direction::Response, vec![20, 4, 4, 6, 0, 1]),
        (Direction::Response, vec![21, 7, 6, 0, 1, 0, 0, 0, 1]),
        (Direction::Response, vec![24, 0, 4, 0, 2, 0, 1]),
        (Direction::Response, vec![24, 0, 2, 0, 32]),
        (Direction::Response, vec![43, 14, 1, 1, 0, 0, 2, 0, 1, b'A']),
        (Direction::Response, vec![43, 14, 1, 1, 0, 0, 1, 0, 2, b'A']),
        (Direction::Request, vec![43, 14, 0, 0]),
    ] {
        assert!(
            matches!(
                events(direction, &pdu).last(),
                Some(ModbusPduEvent::End {
                    valid: false,
                    length_ok: false,
                    ..
                })
            ),
            "{pdu:?}"
        );
    }
    // CANopen MEI is observed as bytes; it is not falsely validated as MEI 14.
    assert!(matches!(
        events(Direction::Request, &[43, 13, 0, 0]).last(),
        Some(ModbusPduEvent::End {
            valid: false,
            supported: false,
            ..
        })
    ));
    for code in [20, 21, 24, 43] {
        assert!(matches!(
            events(Direction::Response, &[code | 0x80, 2]).last(),
            Some(ModbusPduEvent::End { valid: true, .. })
        ));
    }
}

#[test]
fn file_subrecords_and_mei_object_bytes_are_block_gap_and_backpressure_invariant() {
    let block = waveform(&[
        &[20, 12, 5, 6, 0x0d, 0xfe, 0, 0x20, 5, 6, 0x33, 0xcd, 0, 0x40],
        &[43, 14, 1, 1, 0, 0, 1, 0, 3, b'A', b'B', b'C'],
        &[24, 0, 6, 0, 2, 0x12, 0x34, 0xab, 0xcd],
    ]);
    assert_block_invariant(
        |g| build(Direction::Response, g),
        &block,
        &[TickRange::new(Tick(2000), Tick(2200))],
    );
}
