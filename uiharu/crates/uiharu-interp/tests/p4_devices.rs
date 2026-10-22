// SPDX-License-Identifier: Apache-2.0
//! Manufacturer-derived P4 vectors through the native buses, generated consumers and IR.
mod common;
use common::program;
use serde_json::{Value, json};
use uiharu_core::testing::{Harness, assert_block_invariant, without_progress};
use uiharu_core::{GraphConfig, InputRef, LogicBlock, OutputRef, Props, Tick, TickRange, TimeBase};
use uiharu_interp::{DynEvent, Interpreter, ToDyn};
use uiharu_protocols::onewire::synth::OneWireWaveform;
use uiharu_protocols::onewire::{LinkConfig, OneWireEvent, OneWireLink, OneWireNetwork};
fn wire_build(
    name: &str,
    config: &Value,
    interpreted: bool,
    limits: GraphConfig,
) -> Harness<DynEvent> {
    Harness::build(1, TimeBase::new(1_000_000), limits, |g, s| {
        let link = g.add(OneWireLink::new(LinkConfig::default()).expect("link"));
        g.connect(s.samples, OneWireLink::input(link))?;
        let net = g.add(OneWireNetwork::new());
        g.connect(OneWireLink::events(link), OneWireNetwork::input(net))?;
        if interpreted {
            let bridge = g.add(ToDyn::<OneWireEvent>::new(OneWireNetwork::props()));
            g.connect(OneWireNetwork::events(net), InputRef::of_node(bridge, 0))?;
            let node = g.add(Interpreter::new(program(name), config).expect("IR consumer"));
            g.connect(OutputRef::of_node(bridge, 0), Interpreter::bus_input(node))?;
            return Ok(Interpreter::events(node));
        }
        match name {
            "ds2408" => {
                use uiharu_protocols::ds2408::{Config, Ds2408, Event};
                let cfg: Config = serde_json::from_value(config.clone()).expect("config");
                let node = g.add(Ds2408::new(cfg).expect("consumer"));
                g.connect(OneWireNetwork::events(net), Ds2408::input(node))?;
                let bridge = g.add(ToDyn::<Event>::new(Props::new()));
                g.connect(Ds2408::events(node), InputRef::of_node(bridge, 0))?;
                Ok(OutputRef::of_node(bridge, 0))
            }
            "ds243x" => {
                use uiharu_protocols::ds243x::{Config, Ds243x, Event};
                let cfg: Config = serde_json::from_value(config.clone()).expect("config");
                let node = g.add(Ds243x::new(cfg).expect("consumer"));
                g.connect(OneWireNetwork::events(net), Ds243x::input(node))?;
                let bridge = g.add(ToDyn::<Event>::new(Props::new()));
                g.connect(Ds243x::events(node), InputRef::of_node(bridge, 0))?;
                Ok(OutputRef::of_node(bridge, 0))
            }
            "ds28ea00" => {
                use uiharu_protocols::ds28ea00::{Config, Ds28ea00, Event};
                let cfg: Config = serde_json::from_value(config.clone()).expect("config");
                let node = g.add(Ds28ea00::new(cfg).expect("consumer"));
                g.connect(OneWireNetwork::events(net), Ds28ea00::input(node))?;
                let bridge = g.add(ToDyn::<Event>::new(Props::new()));
                g.connect(Ds28ea00::events(node), InputRef::of_node(bridge, 0))?;
                Ok(OutputRef::of_node(bridge, 0))
            }
            _ => panic!("unknown test consumer"),
        }
    })
    .expect("graph")
}
fn wave() -> OneWireWaveform {
    let mut w = OneWireWaveform::new(
        LinkConfig::default(),
        1,
        TimeBase::new(1_000_000),
        Tick::ZERO,
    );
    w.idle(100);
    w
}
fn transaction(w: &mut OneWireWaveform, bytes: &[u8]) {
    w.reset(true);
    w.byte(0xcc);
    for &b in bytes {
        w.byte(b)
    }
}
fn crc16(bytes: &[u8]) -> u16 {
    let mut crc = 0u16;
    for &b in bytes {
        crc ^= u16::from(b);
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xa001
            } else {
                crc >> 1
            }
        }
    }
    crc ^ 0xffff
}
fn crc8(bytes: &[u8]) -> u8 {
    let mut crc = 0u8;
    for &b in bytes {
        crc ^= b;
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0x8c
            } else {
                crc >> 1
            }
        }
    }
    crc
}
fn decode(name: &str, config: Value, block: &LogicBlock) -> Vec<Value> {
    let mut reference = wire_build(name, &config, false, GraphConfig::default());
    reference.run_all(block);
    reference.assert_progress_promise();
    for interpreted in [false, true] {
        for limits in [
            GraphConfig::default(),
            GraphConfig {
                queue_capacity: 1,
                budget: 3,
            },
        ] {
            let mut cut = wire_build(name, &config, interpreted, limits);
            cut.feed_cut(block, &[], &[1, 211, 13], &[0, 1, 7]);
            cut.end();
            cut.assert_progress_promise();
            assert_eq!(
                without_progress(&reference.items),
                without_progress(&cut.items),
                "{name}"
            )
        }
    }
    reference
        .events()
        .iter()
        .map(|e| e.payload.0.clone())
        .collect()
}
#[test]
fn ds2408_channel_crc_cycles_address_and_complement_checks() {
    let mut w = wave();
    transaction(&mut w, &[0x5a, 0x12, 0xed, 0xaa, 0x56, 0x34, 0, 0xff, 0x78]);
    let mut first = vec![0xf5];
    first.extend(0u8..32);
    let crc = crc16(&first);
    first.extend(crc.to_le_bytes());
    let second: Vec<u8> = (32..64).collect();
    first.extend(&second);
    first.extend(crc16(&second).to_le_bytes());
    transaction(&mut w, &first);
    let mut regs = vec![0xf0, 0x8d, 0, 0xa5, 0xff, 0xff];
    regs.extend(crc16(&regs).to_le_bytes());
    transaction(&mut w, &regs);
    transaction(&mut w, &[0xcc, 0x8b, 0, 0x11, 0x22, 0x33]);
    transaction(&mut w, &[0xc3, 0xaa, 0xff]);
    let events = decode("ds2408", json!({}), &w.finish());
    assert_eq!(
        events
            .iter()
            .filter(|e| e["kind"] == "data" && e["field"] == "pio-sample")
            .count(),
        66
    );
    assert_eq!(
        events
            .iter()
            .filter(|e| e["kind"] == "check" && e["check"] == "crc16" && e["ok"] == true)
            .count(),
        3
    );
    assert!(
        events
            .iter()
            .any(|e| e["check"] == "inverse" && e["ok"] == false)
    );
    assert!(
        events
            .iter()
            .any(|e| e["kind"] == "address" && e["address"] == 0x8b)
    );
}
#[test]
fn ds2432_scratchpad_authentication_transport_and_crc_boundaries() {
    let mut w = wave();
    let mut write = vec![0x0f, 3, 0, 1, 2, 3, 4, 5, 6, 7, 8];
    write.extend(crc16(&write).to_le_bytes());
    transaction(&mut w, &write);
    let mut read = vec![0xaa, 0, 0, 7, 1, 2, 3, 4, 5, 6, 7, 8];
    read.extend(crc16(&read).to_le_bytes());
    transaction(&mut w, &read);
    let mut auth = vec![0xa5, 0x1e, 0, 0x12, 0x34, 0xff];
    auth.extend(crc16(&auth).to_le_bytes());
    let mac: Vec<u8> = (0..20).collect();
    auth.extend(&mac);
    auth.extend(crc16(&mac).to_le_bytes());
    auth.push(0xaa);
    transaction(&mut w, &auth);
    let mut copy = vec![0x55, 0, 0, 7];
    copy.extend(&mac);
    copy.push(0x55);
    transaction(&mut w, &copy);
    transaction(&mut w, &[0x5a, 0x80, 0, 7, 0xaa]);
    transaction(&mut w, &[0x33, 0, 0, 0x55]);
    let events = decode("ds243x", json!({"model":"ds2432"}), &w.finish());
    assert_eq!(
        events
            .iter()
            .filter(|e| e["kind"] == "crc16" && e["ok"] == true)
            .count(),
        4
    );
    assert_eq!(events.iter().filter(|e| e["field"] == "mac").count(), 40);
    assert_eq!(
        events
            .iter()
            .filter(|e| e["kind"] == "status" && e["success"] == true)
            .count(),
        4
    );
    assert!(!events.iter().any(|e| e["kind"] == "error"));
}
#[test]
fn ds2433_offset_scratchpad_has_no_read_crc_and_allows_memory_streams() {
    let mut w = wave();
    let mut write = vec![0x0f, 0x1e, 1, 0x12, 0x34];
    write.extend(crc16(&write).to_le_bytes());
    transaction(&mut w, &write);
    transaction(&mut w, &[0xaa, 0x1e, 1, 0x1f, 0x12, 0x34]);
    transaction(&mut w, &[0xf0, 0xfe, 1, 0x56, 0x78]);
    transaction(&mut w, &[0x55, 0x1e, 1, 0x1f, 0xaa]);
    let events = decode("ds243x", json!({"model":"ds2433"}), &w.finish());
    assert_eq!(events.iter().filter(|e| e["kind"] == "crc16").count(), 1);
    assert!(
        events
            .iter()
            .any(|e| e["kind"] == "crc16" && e["ok"] == true)
    );
    assert_eq!(
        events.iter().filter(|e| e["field"] == "scratchpad").count(),
        4
    );
    assert_eq!(events.iter().filter(|e| e["field"] == "memory").count(), 2);
}
#[test]
fn ds28ea00_scratchpad_negative_temperature_pio_chain_and_conditional_rom() {
    let mut w = wave();
    let mut scratch = vec![0xbe, 0x5e, 0xff, 0x4b, 0x46, 0x7f, 0xff, 0, 0x10];
    scratch.push(crc8(&scratch[1..]));
    transaction(&mut w, &scratch);
    transaction(&mut w, &[0xf5, 0xa5, 0x55]);
    transaction(&mut w, &[0xa5, 0xfe, 1, 0xaa, 0xc3]);
    transaction(&mut w, &[0x99, 0x5a, 0xa5, 0xaa]);
    w.reset(true);
    w.byte(0xcc);
    w.byte(0xb4);
    w.read_bit(false);
    let mut rom = [0x42, 1, 2, 3, 4, 5, 6, 0];
    rom[7] = crc8(&rom[..7]);
    w.reset(true);
    w.byte(0x0f);
    for b in rom {
        w.read_byte(b)
    }
    w.byte(0x99);
    w.byte(0x96);
    w.byte(0x69);
    w.read_byte(0xaa);
    let events = decode("ds28ea00", json!({}), &w.finish());
    assert!(events.iter().any(|e| e["kind"] == "temperature"
        && e["microCelsius"] == 10125000
        && e["negative"] == true
        && e["crcOk"] == true));
    assert!(
        events
            .iter()
            .any(|e| e["kind"] == "pio" && e["valid"] == true)
    );
    assert!(
        events
            .iter()
            .any(|e| e["kind"] == "pio" && e["valid"] == false)
    );
    assert_eq!(
        events
            .iter()
            .filter(|e| e["kind"] == "chain" && e["inverseOk"] == true)
            .count(),
        2
    );
    assert!(
        events
            .iter()
            .any(|e| e["kind"] == "status" && e["count"] == 1 && e["value"] == 0)
    );
    assert!(
        events
            .iter()
            .any(|e| e["kind"] == "rom" && e["crcOk"] == true && e["id"].as_str().is_some())
    );
}
#[test]
fn gaps_partial_fields_and_missing_reset_are_bounded_and_equivalent() {
    for (name, config, bytes) in [
        ("ds2408", json!({}), vec![0x5a, 0x12]),
        ("ds243x", json!({"model":"ds2432"}), vec![0x0f, 0]),
        ("ds28ea00", json!({}), vec![0xbe, 0x12]),
    ] {
        let mut w = wave();
        w.byte(0xcc);
        w.byte(0xf0);
        transaction(&mut w, &bytes);
        w.idle(100);
        let gap = w.position();
        w.idle(100);
        transaction(&mut w, &[0xfe, 0x44]);
        let block = w.finish();
        let ranges = [TickRange::new(gap, Tick(gap.0 + 100))];
        let mut compiled = wire_build(name, &config, false, GraphConfig::default());
        compiled.feed_with_gaps(&block, &ranges, &[u64::MAX]);
        compiled.end();
        for interpreted in [false, true] {
            let build = |limits| wire_build(name, &config, interpreted, limits);
            assert_block_invariant(build, &block, &ranges);
            let mut h = build(GraphConfig {
                queue_capacity: 1,
                budget: 2,
            });
            h.feed_cut(&block, &ranges, &[17, 211], &[0, 1, 7]);
            h.end();
            assert_eq!(
                without_progress(&compiled.items),
                without_progress(&h.items)
            );
        }
        assert!(
            compiled
                .events()
                .iter()
                .any(|e| e.payload.0["reason"] == "truncated")
        );
    }
}

fn cfp_build(interpreted: bool, limits: GraphConfig) -> Harness<DynEvent> {
    use uiharu_protocols::mdio::{Mdio, MdioConfig, MdioEvent};
    Harness::build(2, TimeBase::new(1_000_000), limits, |g, s| {
        let bus = g.add(Mdio::new(MdioConfig::default()).expect("MDIO"));
        g.connect(s.samples, Mdio::input(bus))?;
        if interpreted {
            let b = g.add(ToDyn::<MdioEvent>::new(Props::new()));
            g.connect(Mdio::events(bus), InputRef::of_node(b, 0))?;
            let n = g.add(Interpreter::new(program("cfp"), &json!({"phy":3})).expect("CFP IR"));
            g.connect(OutputRef::of_node(b, 0), Interpreter::bus_input(n))?;
            return Ok(Interpreter::events(n));
        }
        use uiharu_protocols::cfp::{Cfp, Config, Event};
        let n = g.add(Cfp::new(Config { phy: 3 }).expect("CFP"));
        g.connect(Mdio::events(bus), Cfp::input(n))?;
        let b = g.add(ToDyn::<Event>::new(Props::new()));
        g.connect(Cfp::events(n), InputRef::of_node(b, 0))?;
        Ok(OutputRef::of_node(b, 0))
    })
    .expect("graph")
}
#[test]
fn cfp_clause45_address_pairing_regions_increment_and_unrelated_ports() {
    use uiharu_protocols::mdio::{
        MdioConfig,
        synth::{Frame, MdioWaveform},
    };
    let mut w = MdioWaveform::new(MdioConfig::default(), 2, 2, Tick::ZERO);
    w.idle(10);
    for frame in [
        Frame::c45_read(3, 1, 14),
        Frame::c45_address(3, 1, 0x8000),
        Frame::c45_address(4, 1, 0xa000),
        Frame::c45_address(3, 2, 0x8800),
        Frame::c22_write(3, 1, 0x9000),
        Frame::c45_read_increment(3, 1, 14),
        Frame::c45_read(3, 1, 0x1234),
        Frame::c45_address(3, 1, 0xa080),
        Frame::c45_write(3, 1, 0x96a5),
        Frame::c45_address(3, 1, 0x8000),
        Frame::c45_read(3, 1, 18),
    ] {
        w.frame(32, &frame);
    }
    let block = w.finish();
    let mut reference = cfp_build(false, GraphConfig::default());
    reference.run_all(&block);
    let e: Vec<_> = reference
        .events()
        .iter()
        .map(|e| e.payload.0.clone())
        .collect();
    assert!(e.iter().any(|e| e["reason"] == "missing-address"));
    assert!(
        e.iter()
            .any(|e| e["kind"] == "identifier" && e["module"] == "cfp")
    );
    assert!(
        e.iter()
            .any(|e| e["kind"] == "identifier" && e["module"] == "cfp4")
    );
    assert!(
        e.iter()
            .any(|e| e["kind"] == "register" && e["address"] == 0x8001 && e["value"] == 0x1234)
    );
    assert!(
        e.iter()
            .any(|e| e["kind"] == "register" && e["region"] == "mlg" && e["read"] == false)
    );
    for interpreted in [false, true] {
        let build = |limits| cfp_build(interpreted, limits);
        assert_block_invariant(build, &block, &[]);
        let mut cut = build(GraphConfig {
            queue_capacity: 1,
            budget: 2,
        });
        cut.feed_cut(&block, &[], &[1, 7, 29], &[0, 1, 7]);
        cut.end();
        assert_eq!(
            without_progress(&reference.items),
            without_progress(&cut.items)
        );
    }
}
#[test]
fn cfp_gap_and_bad_turnaround_invalidate_the_observed_address() {
    use uiharu_protocols::mdio::{
        MdioConfig,
        synth::{Frame, MdioWaveform},
    };
    let mut w = MdioWaveform::new(MdioConfig::default(), 2, 2, Tick::ZERO);
    w.idle(10);
    w.frame(32, &Frame::c45_address(3, 1, 0x8000));
    w.idle(20);
    let gap = w.position();
    w.idle(20);
    w.frame(32, &Frame::c45_read(3, 1, 14));
    w.frame(32, &Frame::c45_address(3, 1, 0x8000));
    w.frame(32, &Frame::c45_read(3, 1, 14).with_turnaround(3));
    w.frame(32, &Frame::c45_read(3, 1, 14));
    let block = w.finish();
    let gaps = [TickRange::new(gap, Tick(gap.0 + 20))];
    let mut reference = cfp_build(false, GraphConfig::default());
    reference.feed_with_gaps(&block, &gaps, &[u64::MAX]);
    reference.end();
    assert!(
        !reference
            .events()
            .iter()
            .any(|e| e.payload.0["kind"] == "identifier")
    );
    for interpreted in [false, true] {
        let mut cut = cfp_build(
            interpreted,
            GraphConfig {
                queue_capacity: 1,
                budget: 2,
            },
        );
        cut.feed_cut(&block, &gaps, &[13, 211], &[0, 1, 7]);
        cut.end();
        cut.assert_progress_promise();
        assert_eq!(
            without_progress(&reference.items),
            without_progress(&cut.items)
        );
    }
}
fn am_build(model: &str, interpreted: bool, limits: GraphConfig) -> Harness<DynEvent> {
    use uiharu_protocols::am230x::{Am230x, Am230xConfig, Am230xEvent, Model};
    Harness::build(1, TimeBase::new(1_000_000), limits, |g, s| {
        let cfg = Am230xConfig {
            data: 0,
            model: if model == "dht11" {
                Model::Dht11
            } else {
                Model::Am2302
            },
            ..Am230xConfig::default()
        };
        let bus = g.add(Am230x::new(cfg).expect("sensor"));
        g.connect(s.samples, Am230x::input(bus))?;
        if interpreted {
            let b = g.add(ToDyn::<Am230xEvent>::new(Props::new()));
            g.connect(Am230x::events(bus), InputRef::of_node(b, 0))?;
            let n = g.add(
                Interpreter::new(program("am230x-values"), &json!({"model":model}))
                    .expect("sensor IR"),
            );
            g.connect(OutputRef::of_node(b, 0), Interpreter::bus_input(n))?;
            return Ok(Interpreter::events(n));
        }
        use uiharu_protocols::am230x_values::{Am230xValues, Config, Event};
        let cfg: Config = serde_json::from_value(json!({"model":model})).expect("config");
        let n = g.add(Am230xValues::new(cfg).expect("engineering consumer"));
        g.connect(Am230x::events(bus), Am230xValues::input(n))?;
        let b = g.add(ToDyn::<Event>::new(Props::new()));
        g.connect(Am230xValues::events(n), InputRef::of_node(b, 0))?;
        Ok(OutputRef::of_node(b, 0))
    })
    .expect("graph")
}
#[test]
fn am230x_exact_five_bytes_engineering_values_sign_and_checksum() {
    use uiharu_protocols::am230x::{
        Am230xConfig, Model,
        synth::{Am230xWaveform, with_checksum},
    };
    for (model, raw, humidity, temperature, negative) in [
        ("am2302", [2, 0x8c, 0x80, 0x7b], 65200, 12300, true),
        ("dht11", [55, 0, 24, 0], 55000, 24000, false),
    ] {
        let cfg = Am230xConfig {
            data: 0,
            model: if model == "dht11" {
                Model::Dht11
            } else {
                Model::Am2302
            },
            ..Am230xConfig::default()
        };
        let mut w = Am230xWaveform::new(cfg, 1, TimeBase::new(1_000_000), Tick::ZERO);
        w.idle_us(2000);
        w.reading(with_checksum(raw));
        w.idle_us(2000);
        let mut bad = with_checksum(raw);
        bad[4] ^= 1;
        w.reading(bad);
        if model == "dht11" {
            w.idle_us(2000);
            w.reading(with_checksum([55, 1, 24, 0]))
        }
        w.idle_us(2000);
        let block = w.finish();
        let mut reference = am_build(model, false, GraphConfig::default());
        reference.run_all(&block);
        let e: Vec<_> = reference
            .events()
            .iter()
            .map(|e| e.payload.0.clone())
            .collect();
        assert!(e.iter().any(|e| e["kind"] == "reading"
            && e["humidityMilliPercent"] == humidity
            && e["temperatureMilliCelsius"] == temperature
            && e["negative"] == negative
            && e["valid"] == true));
        assert!(e.iter().any(|e| e["reason"] == "checksum"));
        if model == "dht11" {
            assert!(e.iter().any(|e| e["reason"] == "decimal"));
        }
        for interpreted in [false, true] {
            let mut cut = am_build(
                model,
                interpreted,
                GraphConfig {
                    queue_capacity: 1,
                    budget: 3,
                },
            );
            cut.feed_cut(&block, &[], &[1, 211, 13], &[0, 1, 7]);
            cut.end();
            cut.assert_progress_promise();
            assert_eq!(
                without_progress(&reference.items),
                without_progress(&cut.items)
            );
        }
    }
}
#[test]
fn canonical_contract_validation_preserves_wire_shapes() {
    use uiharu_protocols::{am230x::Am230xEvent, mdio::MdioEvent, onewire::OneWireEvent};
    for bytes in [json!([1, 2, 3, 4]), json!([1, 2, 3, 4, 5, 6])] {
        assert!(serde_json::from_value::<Am230xEvent>(json!({"kind":"reading","bytes":bytes,"checksumOk":true,"humidityRaw":1,"temperatureRaw":2})).is_err());
    }
    assert!(serde_json::from_value::<MdioEvent>(json!({"kind":"frame","clause":23,"op":"read","phy":0,"reg":1,"turnaround":2,"data":0,"preambleBits":32})).is_err());
    let event = OneWireEvent::Search {
        tx: 1,
        id: u64::MAX,
        crc_ok: true,
        conflicts: 0,
        unanswered: 0,
    };
    let encoded = serde_json::to_value(event).expect("wire");
    assert_eq!(encoded["id"], "0xffffffffffffffff");
    assert_eq!(
        serde_json::from_value::<OneWireEvent>(encoded).expect("round trip"),
        event
    );
}
