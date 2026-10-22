// SPDX-License-Identifier: Apache-2.0
//! SD v2.00 CID/CSD register fields and separate transport/register integrity checks.
mod dsl_spi_support;
use serde_json::{Value, json};
use uiharu_core::testing::{Harness, assert_block_invariant};
use uiharu_core::{GraphConfig, InputRef, LogicBlock, OutputRef, Tick, TimeBase};
use uiharu_dsl_protocols::sd_registers::{Config, Event, SdRegisters};
use uiharu_interp::{DynEvent, Interpreter, ToDyn};
use uiharu_protocols::spi::{Spi, SpiConfig, SpiEvent, synth::SpiWaveform};
fn compiled(limits: GraphConfig) -> Harness<Event> {
    Harness::build(8, TimeBase::new(1_000_000), limits, |g, s| {
        let bus = g.add(Spi::new(SpiConfig::default()).expect("SPI"));
        g.connect(s.samples, Spi::input(bus))?;
        let n = g.add(SdRegisters::new(Config {}).expect("registers"));
        g.connect(Spi::events(bus), SdRegisters::input(n))?;
        Ok(SdRegisters::events(n))
    })
    .expect("graph")
}
fn decode(block: &LogicBlock) -> Vec<Value> {
    let limits = GraphConfig {
        queue_capacity: 1,
        budget: 2,
    };
    let mut c = compiled(limits);
    c.feed(block, &[1, 7, 29]);
    c.end();
    let mut i = Harness::<DynEvent>::build(8, TimeBase::new(1_000_000), limits, |g, s| {
        let bus = g.add(Spi::new(SpiConfig::default()).expect("SPI"));
        g.connect(s.samples, Spi::input(bus))?;
        let b = g.add(ToDyn::<SpiEvent>::new(SpiConfig::default().props()));
        g.connect(Spi::events(bus), InputRef::of_node(b, 0))?;
        let n = g.add(
            Interpreter::new(dsl_spi_support::program("sd-registers"), &json!({})).expect("IR"),
        );
        g.connect(OutputRef::of_node(b, 0), Interpreter::bus_input(n))?;
        Ok(Interpreter::events(n))
    })
    .expect("graph");
    i.feed(block, &[1, 7, 29]);
    i.end();
    dsl_spi_support::assert_items(&c.items, &i.items, "sd-registers");
    assert_block_invariant(compiled, block, &[]);
    c.events()
        .iter()
        .map(|e| serde_json::to_value(e.payload).expect("JSON"))
        .collect()
}
fn reg_crc(mut bytes: [u8; 16]) -> [u8; 16] {
    let mut crc = 0u8;
    for byte in &bytes[..15] {
        for bit in (0..8).rev() {
            let f = (crc >> 6) ^ (byte >> bit);
            crc = (crc << 1) & 127;
            if f & 1 != 0 {
                crc ^= 9
            }
        }
    }
    bytes[15] = (crc << 1) | 1;
    bytes
}
fn transport_crc(bytes: &[u8]) -> u16 {
    let mut crc = 0u16;
    for &b in bytes {
        crc ^= u16::from(b) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 {
                (crc << 1) ^ 0x1021
            } else {
                crc << 1
            }
        }
    }
    crc
}
fn frame(command: u8, bytes: [u8; 16]) -> Vec<(u8, u8)> {
    let mut frame = vec![
        (0x40 | command, 0xff),
        (0, 0xff),
        (0, 0xff),
        (0, 0xff),
        (0, 0xff),
        (1, 0xff),
        (0xff, 0),
        (0xff, 0xfe),
    ];
    frame.extend(bytes.into_iter().map(|b| (0xff, b)));
    frame.extend(
        transport_crc(&bytes)
            .to_be_bytes()
            .into_iter()
            .map(|b| (0xff, b)),
    );
    frame
}
fn wave(frames: &[Vec<(u8, u8)>]) -> LogicBlock {
    let mut w = SpiWaveform::new(SpiConfig::default(), 8, 2, Tick::ZERO);
    w.idle(5);
    for frame in frames {
        w.transaction(frame.iter().copied());
        w.idle(5)
    }
    w.finish()
}
fn has(events: &[Value], expected: Value) {
    assert!(
        events.iter().any(|e| expected
            .as_object()
            .expect("expected object")
            .iter()
            .all(|(k, v)| e.get(k) == Some(v))),
        "missing {expected}: {events:?}"
    )
}
#[test]
fn cid_name_and_inner_crc_are_independent_of_data_block_crc() {
    let cid = reg_crc([
        3, b'S', b'D', b'A', b'B', b'C', b'D', b'E', 0x21, 0x12, 0x34, 0x56, 0x78, 1, 0x94, 0,
    ]);
    let mut bad = cid;
    bad[15] ^= 2;
    let e = decode(&wave(&[frame(10, cid), frame(10, bad)]));
    has(
        &e,
        json!({"kind":"cid","productName":[65,66,67,68,69],"manufacturer":3,"oem":0x5344,"year":2025,"month":4,"crcOk":true,"registerCrcOk":true,"registerEndOk":true}),
    );
    has(&e, json!({"kind":"cid","crcOk":true,"registerCrcOk":false}));
}
#[test]
fn csd_v1_fields_and_v2_capacity_do_not_reinterpret_reserved_current_bits() {
    let mut word = 0u128;
    for (low, width, value) in [
        (112, 8, 0x2e),
        (104, 8, 0x11),
        (96, 8, 0x32),
        (84, 12, 0x5b5),
        (80, 4, 9),
        (79, 1, 1),
        (78, 1, 1),
        (77, 1, 1),
        (76, 1, 1),
        (62, 12, 0xabc),
        (59, 3, 1),
        (56, 3, 2),
        (53, 3, 3),
        (50, 3, 4),
        (47, 3, 5),
        (46, 1, 1),
        (39, 7, 0x12),
        (32, 7, 0x23),
        (31, 1, 1),
        (26, 3, 3),
        (22, 4, 9),
        (21, 1, 1),
        (15, 1, 1),
        (14, 1, 1),
        (13, 1, 1),
        (12, 1, 1),
        (10, 2, 2),
    ] {
        word |= (value as u128 & ((1u128 << width) - 1)) << low
    }
    let csd1 = reg_crc(word.to_be_bytes());
    let csd2 = reg_crc(((1u128 << 126) | (9u128 << 80) | (0x1fffffu128 << 48)).to_be_bytes());
    let e = decode(&wave(&[frame(9, csd1), frame(9, csd2)]));
    has(
        &e,
        json!({"kind":"csd","structure":0,"blocks":351872,"taac":0x2e,"nsac":0x11,"transferSpeed":0x32,"commandClasses":0x5b5,"readCurrentMin":1,"readCurrentMax":2,"writeCurrentMin":3,"writeCurrentMax":4,"sizeMultiplier":5,"eraseSectorBlocks":19,"writeProtectGroupSectors":36,"writeSpeedFactor":3,"writeBlockBits":9,"writePartial":true,"copied":true,"permanentWriteProtect":true,"temporaryWriteProtect":true,"fileFormat":2,"registerCrcOk":true}),
    );
    has(
        &e,
        json!({"kind":"csd","structure":1,"blocks":2147483648u64,"readCurrentMin":null,"sizeMultiplier":null,"registerCrcOk":true}),
    );
}

#[test]
fn malformed_sd_start_or_transmission_bits_never_produce_register_semantics() {
    let register = reg_crc([0x40, 0, 0, 0, 0, 9, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    let mut frames = Vec::new();
    for header in [0x09, 0x89, 0xc9] {
        let mut malformed = frame(9, register);
        malformed[0].0 = header;
        frames.push(malformed);
    }
    frames.push(frame(9, register));
    let events = decode(&wave(&frames));
    assert_eq!(
        events
            .iter()
            .filter(|event| event["kind"] == "begin")
            .count(),
        1
    );
    assert_eq!(
        events.iter().filter(|event| event["kind"] == "csd").count(),
        1
    );
}
