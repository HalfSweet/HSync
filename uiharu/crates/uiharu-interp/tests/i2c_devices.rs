// SPDX-License-Identifier: Apache-2.0

//! Manufacturer-derived register vectors through both generated and interpreted DSL nodes.
mod common;

use common::program;
use serde_json::{Value, json};
use uiharu_core::testing::{Harness, assert_block_invariant, without_progress};
use uiharu_core::{
    GraphConfig, InputRef, Item, LogicBlock, Node, OutputRef, Tick, TickRange, TimeBase,
};
use uiharu_interp::{DynEvent, Interpreter, ToDyn};
use uiharu_protocols::i2c::synth::I2cWaveform;
use uiharu_protocols::i2c::{I2c, I2cConfig, I2cEvent};
use uiharu_protocols::{
    adxl345, atsha204a, ds1307, edid, eeprom24xx_memory, lm75, ltc26x7, mcp230xx, mlx90614,
    mxc6225xu, nunchuk, pca9571, rtc8564, tca6408a, xfp,
};

fn harness(
    name: &str,
    config: &Value,
    interpreted: bool,
    limits: GraphConfig,
) -> Harness<DynEvent> {
    Harness::build(2, TimeBase::new(1_000_000), limits, |graph, source| {
        let i2c = graph.add(I2c::new(I2cConfig::default()).expect("I2C wiring"));
        graph.connect(source.samples, I2c::input(i2c))?;
        if interpreted {
            let bridge = graph.add(ToDyn::<I2cEvent>::new(I2cConfig::default().props()));
            graph.connect(I2c::events(i2c), InputRef::of_node(bridge, 0))?;
            let node = graph.add(Interpreter::new(program(name), config).expect("device config"));
            graph.connect(
                OutputRef::<DynEvent>::of_node(bridge, 0),
                Interpreter::bus_input(node),
            )?;
            return Ok(Interpreter::events(node));
        }
        macro_rules! compiled {
            ($module:ident, $node:ident) => {{
                let config: $module::DeviceConfig =
                    serde_json::from_value(config.clone()).expect("config");
                let decoder = $module::$node::new(config).expect("valid config");
                let props = decoder.spec().outputs[0].props.clone();
                let node = graph.add(decoder);
                graph.connect(I2c::events(i2c), $module::$node::input(node))?;
                let bridge = graph.add(ToDyn::<$module::DeviceEvent>::new(props));
                graph.connect($module::$node::events(node), InputRef::of_node(bridge, 0))?;
                Ok(OutputRef::of_node(bridge, 0))
            }};
        }
        match name {
            "ds1307" => compiled!(ds1307, Ds1307),
            "rtc8564" => compiled!(rtc8564, Rtc8564),
            "lm75" => compiled!(lm75, Lm75),
            "mlx90614" => compiled!(mlx90614, Mlx90614),
            "adxl345" => compiled!(adxl345, Adxl345),
            "mcp230xx" => compiled!(mcp230xx, Mcp230xx),
            "pca9571" => compiled!(pca9571, Pca9571),
            "tca6408a" => compiled!(tca6408a, Tca6408a),
            "eeprom24xx-memory" => {
                let config: eeprom24xx_memory::Config =
                    serde_json::from_value(config.clone()).expect("config");
                let decoder =
                    eeprom24xx_memory::Eeprom24xxMemory::new(config).expect("memory config");
                let props = decoder.spec().outputs[0].props.clone();
                let node = graph.add(decoder);
                graph.connect(
                    I2c::events(i2c),
                    eeprom24xx_memory::Eeprom24xxMemory::input(node),
                )?;
                let bridge = graph.add(ToDyn::<eeprom24xx_memory::Event>::new(props));
                graph.connect(
                    eeprom24xx_memory::Eeprom24xxMemory::events(node),
                    InputRef::of_node(bridge, 0),
                )?;
                Ok(OutputRef::of_node(bridge, 0))
            }
            "edid" => compiled!(edid, Edid),
            "ltc26x7" => compiled!(ltc26x7, Ltc26x7),
            "mxc6225xu" => compiled!(mxc6225xu, Mxc6225xu),
            "nunchuk" => compiled!(nunchuk, Nunchuk),
            "atsha204a" => compiled!(atsha204a, Atsha204a),
            "xfp" => compiled!(xfp, Xfp),
            _ => panic!("unknown test device"),
        }
    })
    .expect("device graph")
}
fn wave() -> I2cWaveform {
    let mut wave = I2cWaveform::new(I2cConfig::default(), 2, 2, Tick::ZERO);
    wave.idle(8);
    wave
}
fn write(w: &mut I2cWaveform, address: u8, bytes: &[u8]) {
    w.start();
    w.byte(address * 2, true);
    for &byte in bytes {
        w.byte(byte, true);
    }
    w.stop();
    w.idle(8);
}
fn read(w: &mut I2cWaveform, address: u8, pointer: u8, bytes: &[u8]) {
    w.start();
    w.byte(address * 2, true);
    w.byte(pointer, true);
    w.restart();
    w.byte(address * 2 + 1, true);
    for (i, &byte) in bytes.iter().enumerate() {
        w.byte(byte, i + 1 < bytes.len());
    }
    w.stop();
    w.idle(8);
}
fn events(h: &Harness<DynEvent>) -> Vec<Value> {
    h.events().into_iter().map(|e| e.payload.0).collect()
}
fn decode(name: &str, config: &Value, b: &LogicBlock, gaps: &[TickRange]) -> Harness<DynEvent> {
    let mut compiled = harness(name, config, false, GraphConfig::default());
    compiled.feed_with_gaps(b, gaps, &[u64::MAX]);
    compiled.end();
    compiled.assert_progress_promise();
    let mut interpreted = harness(name, config, true, GraphConfig::default());
    interpreted.feed_with_gaps(b, gaps, &[1, 7, 31]);
    interpreted.end();
    interpreted.assert_progress_promise();
    assert_eq!(
        without_progress(&compiled.items),
        without_progress(&interpreted.items),
        "{name} generated/interpreted"
    );
    compiled
}
fn field(events: &[Value], name: &str, value: u32, negative: bool, valid: bool) {
    assert!(
        events.iter().any(|e| e["kind"] == "field"
            && e["field"] == name
            && e["value"] == value
            && e["negative"] == negative
            && e["valid"] == valid),
        "{name}: {events:?}"
    );
}

#[test]
fn rtc_bcd_modes_flags_ranges_and_pointer_wrap() {
    let mut w = wave();
    read(
        &mut w,
        0x68,
        0,
        &[0xd9, 0x58, 0x71, 7, 0x31, 0x12, 0x24, 0x10],
    );
    read(&mut w, 0x68, 0, &[0x6a]);
    write(&mut w, 0x68, &[0x3f, 0xab, 0x12]);
    let e = events(&decode("ds1307", &json!({}), &w.finish(), &[]));
    field(&e, "second", 59, false, true);
    field(&e, "clock-halted", 1, false, true);
    field(&e, "hour-12", 11, false, true);
    field(&e, "pm", 1, false, true);
    field(&e, "weekday", 7, false, true);
    field(&e, "day", 31, false, true);
    field(&e, "month", 12, false, true);
    field(&e, "year", 24, false, true);
    field(&e, "second", 70, false, false);
    field(&e, "ram", 0xab, false, true);
    field(&e, "second", 12, false, true);
    let mut w = wave();
    read(
        &mut w,
        0x51,
        2,
        &[
            0xd9, 0x59, 0x23, 0x31, 6, 0x92, 0x24, 0x80, 0x23, 0x31, 0x86,
        ],
    );
    let e = events(&decode("rtc8564", &json!({}), &w.finish(), &[]));
    field(&e, "voltage-low", 1, false, true);
    field(&e, "hour", 23, false, true);
    field(&e, "century", 1, false, true);
    field(&e, "alarm-disabled", 1, false, true);
    field(&e, "weekday", 6, false, true);
}
#[test]
fn lm75_positive_negative_thresholds_and_incomplete_word() {
    let mut w = wave();
    read(&mut w, 0x48, 0, &[0x19, 0]);
    read(&mut w, 0x48, 0, &[0xff, 0x80]);
    write(&mut w, 0x48, &[2, 0x50, 0]);
    write(&mut w, 0x48, &[1, 0x1f]);
    read(&mut w, 0x48, 0, &[0x19]);
    let e = events(&decode("lm75", &json!({}), &w.finish(), &[]));
    field(&e, "temperature-microcelsius", 25000000, false, true);
    field(&e, "temperature-microcelsius", 500000, true, true);
    field(&e, "hysteresis-microcelsius", 80000000, false, true);
    field(&e, "configuration", 31, false, true);
    assert!(
        e.iter()
            .any(|e| e["kind"] == "end" && e["reason"] == "incomplete")
    );
}
#[test]
fn mlx90614_published_pec_vector_errors_and_negative_temperatures() {
    // Melexis Rev20 fig6 literal B4 07 B5 D2 3A 30: word3AD2, PEC30.
    let mut w = wave();
    read(&mut w, 0x5a, 7, &[0xd2, 0x3a, 0x30]);
    read(&mut w, 0x5a, 6, &[0, 0, 0]);
    read(&mut w, 0x5a, 7, &[0xd2, 0xba, 0x30]);
    let e = events(&decode("mlx90614", &json!({}), &w.finish(), &[]));
    field(&e, "word", 0x3ad2, false, true);
    field(&e, "pec", 0x30, false, true);
    field(&e, "object-1-centicelsius", 2801, false, true);
    field(&e, "ambient-centicelsius", 27315, true, true);
    field(&e, "object-1-centicelsius", 2801, false, false);
    assert!(e.iter().any(|e| e["field"] == "pec" && e["valid"] == false));
}
#[test]
fn adxl345_little_endian_axes_keep_sign_and_registers() {
    let mut w = wave();
    read(&mut w, 0x53, 0, &[0xe5]);
    read(&mut w, 0x53, 0x32, &[0, 1, 0xff, 0xff, 0, 0x80]);
    write(&mut w, 0x53, &[0x31, 0x0b]);
    let e = events(&decode("adxl345", &json!({}), &w.finish(), &[]));
    field(&e, "device-id", 0xe5, false, true);
    field(&e, "x-raw", 256, false, true);
    field(&e, "y-raw", 1, true, true);
    field(&e, "z-raw", 32768, true, true);
    field(&e, "data-format", 11, false, true);
}
#[test]
fn gpio_registers_pointerless_latch_banks_and_pair_toggle() {
    let mut w = wave();
    write(&mut w, 0x25, &[0xa5, 0x3c]);
    w.start();
    w.byte(0x4b, true);
    w.byte(0x3c, false);
    w.stop();
    w.idle(8);
    let e = events(&decode("pca9571", &json!({}), &w.finish(), &[]));
    field(&e, "output-mask", 0xa5, false, true);
    field(&e, "output-mask", 0x3c, false, true);
    assert!(!e.iter().any(|e| e["kind"] == "pointer"));
    let mut w = wave();
    write(&mut w, 0x20, &[3, 0xf0, 0x0f]);
    read(&mut w, 0x20, 0, &[0xa5]);
    let e = events(&decode("tca6408a", &json!({}), &w.finish(), &[]));
    field(&e, "input-direction-mask", 0xf0, false, true);
    field(&e, "input-direction-mask", 0x0f, false, true);
    field(&e, "input-mask", 0xa5, false, true);
    let mut w = wave();
    write(&mut w, 0x20, &[0x0a, 0x20]);
    read(&mut w, 0x20, 0x12, &[0xa5, 0x3c, 0x55]);
    write(&mut w, 0x20, &[0x0a, 0x80]);
    write(&mut w, 0x20, &[0x19, 0xc3]);
    let e = events(&decode("mcp230xx", &json!({}), &w.finish(), &[]));
    field(&e, "gpio-mask", 0xa5, false, true);
    field(&e, "gpio-mask", 0x3c, false, true);
    field(&e, "gpio-mask", 0x55, false, true);
    field(&e, "gpio-mask", 0xc3, false, true);
    assert!(e.iter().any(|e| e["field"] == "port" && e["value"] == 1));
    let mut w = wave();
    write(&mut w, 0x20, &[5, 0x20]);
    write(&mut w, 0x20, &[9, 0x11, 0x22]);
    let e = events(&decode(
        "mcp230xx",
        &json!({"model":"mcp23008"}),
        &w.finish(),
        &[],
    ));
    field(&e, "gpio-mask", 0x11, false, true);
    field(&e, "gpio-mask", 0x22, false, true);
}
#[test]
fn wrong_targets_nacks_unknown_pointers_and_partial_frames_do_not_invent_values() {
    for (name, address) in [
        ("ds1307", 0x68),
        ("rtc8564", 0x51),
        ("lm75", 0x48),
        ("mlx90614", 0x5a),
        ("adxl345", 0x53),
        ("mcp230xx", 0x20),
        ("tca6408a", 0x20),
    ] {
        let mut w = wave();
        read(&mut w, address ^ 1, 0, &[0x12, 0x34]);
        w.start();
        w.byte(address * 2, true);
        w.byte(0, false);
        w.restart();
        w.byte(address * 2 + 1, true);
        w.byte(0xab, false);
        w.stop();
        w.idle(8);
        let e = events(&decode(name, &json!({}), &w.finish(), &[]));
        assert!(!e.iter().any(|e| e["kind"] == "field"), "{name}: {e:?}");
    }
}
#[test]
fn each_device_is_bounded_and_block_budget_pause_gap_invariant() {
    for (name, address, pointer, bytes) in [
        (
            "ds1307",
            0x68,
            0,
            vec![0x59, 0x58, 0x23, 7, 0x31, 0x12, 0x24],
        ),
        (
            "rtc8564",
            0x51,
            2,
            vec![0x59, 0x58, 0x23, 0x31, 6, 0x12, 0x24],
        ),
        ("lm75", 0x48, 0, vec![0x19, 0]),
        ("mlx90614", 0x5a, 7, vec![0xd2, 0x3a, 0x30]),
        ("adxl345", 0x53, 0x32, vec![0, 1, 0xff, 0xff]),
        ("mcp230xx", 0x20, 0x12, vec![0xa5, 0x3c]),
        ("pca9571", 0x25, 0, vec![0xa5, 0x3c]),
        ("tca6408a", 0x20, 0, vec![0xa5, 0x3c]),
        ("edid", 0x50, 8, vec![0x4c, 0x2d, 0x34, 0x12]),
        ("ltc26x7", 0x10, 0x31, vec![0xab, 0xc0]),
        ("mxc6225xu", 0x15, 0, vec![0x80, 0x7f, 0xff]),
        ("nunchuk", 0x52, 0, vec![128, 64, 0x80, 0x40, 0x20, 0xd9]),
        ("atsha204a", 0x64, 3, vec![4, 0x11, 0x33, 0x43]),
        ("xfp", 0x50, 96, vec![0xff, 0x80]),
        ("eeprom24xx-memory", 0x50, 0, vec![0, 1, 2, 3]),
    ] {
        let mut w = wave();
        read(&mut w, address, pointer, &bytes);
        let gap_start = w.position();
        read(&mut w, address, pointer, &bytes);
        read(&mut w, address, pointer, &bytes);
        let b = w.finish();
        let gaps = [TickRange::new(
            Tick(gap_start.0 + 100),
            Tick(gap_start.0 + 116),
        )];
        decode(name, &json!({}), &b, &gaps);
        for interpreted in [false, true] {
            assert_block_invariant(|g| harness(name, &json!({}), interpreted, g), &b, &gaps);
        }
    }
}
#[test]
fn aborted_pair_and_mid_byte_never_become_complete_temperature() {
    let mut w = wave();
    w.start();
    w.byte(0x90, true);
    w.byte(0, true);
    w.restart();
    w.byte(0x91, true);
    w.byte(0x19, true);
    w.bits(0, 3);
    let b = w.finish();
    let h = decode("lm75", &json!({}), &b, &[]);
    let e = events(&h);
    assert!(!e.iter().any(|e| e["kind"] == "field"));
    assert!(
        e.iter()
            .any(|e| e["kind"] == "end" && e["reason"] == "aborted")
    );
    assert!(matches!(h.items.last(), Some(Item::End(_))));
}

#[test]
fn lm75_all_resolution_profiles() {
    for resolution in 9..=12 {
        let mut w = wave();
        let raw = 1u16 << (16 - resolution);
        read(&mut w, 0x48, 0, &raw.to_be_bytes());
        read(&mut w, 0x48, 0, &(0u16.wrapping_sub(raw)).to_be_bytes());
        let e = events(&decode(
            "lm75",
            &json!({"resolution":resolution}),
            &w.finish(),
            &[],
        ));
        field(
            &e,
            "temperature-microcelsius",
            1_000_000u32 >> (resolution - 8),
            false,
            true,
        );
        field(
            &e,
            "temperature-microcelsius",
            1_000_000u32 >> (resolution - 8),
            true,
            true,
        );
    }
}
#[test]
fn edid_checksum_identity_and_split_multibyte_recovery() {
    let mut block = [0u8; 128];
    block[..8].copy_from_slice(&[0, 255, 255, 255, 255, 255, 255, 0]);
    block[8] = 0x4c;
    block[9] = 0x2d;
    block[10] = 0x34;
    block[11] = 0x12;
    block[12..16].copy_from_slice(&[0x78, 0x56, 0x34, 0xf2]);
    block[127] = 0u8.wrapping_sub(block[..127].iter().fold(0u8, |s, &b| s.wrapping_add(b)));
    let mut w = wave();
    read(&mut w, 0x50, 0, &block);
    let e = events(&decode("edid", &json!({}), &w.finish(), &[]));
    field(&e, "manufacturer-code", 0x4c2d, false, true);
    field(&e, "product-code", 0x1234, false, true);
    field(&e, "serial-number", 0xf2345678, false, true);
    field(&e, "checksum", 0, false, true);
    // Every split within a multi-byte identity or timing field must discard its pending provenance.
    for split in [9, 11, 13, 14, 15, 55, 57, 58, 60, 61] {
        let mut w = wave();
        read(&mut w, 0x50, 0, &block[..split]);
        w.start();
        w.byte(0xa1, true);
        for (i, &b) in block[split..64].iter().enumerate() {
            w.byte(b, i + 1 < 64 - split);
        }
        w.stop();
        w.idle(8);
        let e = events(&decode("edid", &json!({}), &w.finish(), &[]));
        assert!(!e.iter().any(|v| v["field"] == "checksum"));
    }
}
#[test]
fn ltc_model_widths_and_incomplete_command() {
    for (model, code) in [("ltc2607", 0xabc0), ("ltc2617", 0x2af0), ("ltc2627", 0xabc)] {
        let mut w = wave();
        write(&mut w, 0x10, &[0x31, 0xab, 0xc0]);
        write(&mut w, 0x73, &[0x30, 0]);
        let e = events(&decode(
            "ltc26x7",
            &json!({"model":model}),
            &w.finish(),
            &[],
        ));
        field(&e, "dac-code", code, false, true);
        field(&e, "channel", 1, false, true);
        assert!(e.iter().any(|v| v["reason"] == "incomplete"));
    }
}
#[test]
fn nunchuk_raw_report_and_legacy_transform() {
    for encoding in ["plain", "legacy-xor"] {
        let report = [128u8, 64, 0x80, 0x40, 0x20, 0b11011001];
        let wire: Vec<u8> = report
            .iter()
            .map(|&b| {
                if encoding == "plain" {
                    b
                } else {
                    b.wrapping_sub(0x17) ^ 0x17
                }
            })
            .collect();
        let mut w = wave();
        read(&mut w, 0x52, 0, &wire);
        let e = events(&decode(
            "nunchuk",
            &json!({"encoding":encoding}),
            &w.finish(),
            &[],
        ));
        field(&e, "stick-x", 128, false, true);
        field(&e, "accel-x", 514, false, true);
        field(&e, "accel-y", 257, false, true);
        field(&e, "accel-z", 131, false, true);
    }
}
#[test]
fn mxc_signed_axes_and_xfp_signed_diagnostics() {
    let mut w = wave();
    read(&mut w, 0x15, 0, &[0x80, 0x7f, 0xff, 0xff]);
    let e = events(&decode("mxc6225xu", &json!({}), &w.finish(), &[]));
    field(&e, "x-raw", 128, true, true);
    field(&e, "y-raw", 127, false, true);
    let mut w = wave();
    read(&mut w, 0x50, 96, &[0xff, 0x80]);
    read(&mut w, 0x50, 148, b"ACME");
    let e = events(&decode("xfp", &json!({}), &w.finish(), &[]));
    field(&e, "temperature-256ths-celsius", 128, true, true);
    field(&e, "vendor-name-byte", 65, false, true);
}
#[test]
fn atsha_published_wake_crc_and_excess_bytes() {
    // Manufacturer's wake status packet is count=4, status=0x11, CRC=0x4333.
    for (tail, complete) in [
        (vec![4, 0x11, 0x33, 0x43], true),
        (vec![4, 0x11, 0x33, 0x43, 0], false),
        (vec![4, 0x11, 0x33], false),
    ] {
        let mut w = wave();
        w.start();
        w.byte(0xc9, true);
        for (i, &b) in tail.iter().enumerate() {
            w.byte(b, i + 1 < tail.len());
        }
        w.stop();
        w.idle(8);
        let e = events(&decode("atsha204a", &json!({}), &w.finish(), &[]));
        if tail.len() >= 4 {
            field(&e, "crc", 0x4333, false, true);
        }
        assert!(
            e.iter().any(|v| v["kind"] == "end"
                && v["reason"] == if complete { "complete" } else { "incomplete" }),
            "{e:?}"
        );
    }
}

#[test]
fn eeprom_memory_page_capacity_block_and_ack_profiles() {
    let mut w = wave();
    write(&mut w, 0x50, &[0, 0x3f, 0xaa, 0xbb]);
    read(&mut w, 0x50, 0xff, &[]); // Separate one-byte word address is incomplete for this model.
    w.start();
    w.byte(0xa0, true);
    w.byte(0x7f, true);
    w.byte(0xff, true);
    w.restart();
    w.byte(0xa1, true);
    w.byte(1, true);
    w.byte(2, false);
    w.stop();
    w.idle(8);
    let e = events(&decode("eeprom24xx-memory", &json!({}), &w.finish(), &[]));
    let data: Vec<_> = e.iter().filter(|v| v["kind"] == "data").collect();
    assert_eq!(
        data.iter()
            .map(|v| v["address"].clone())
            .collect::<Vec<_>>(),
        vec![json!(63), json!(0), json!(32767), json!(0)]
    );
    let mut w = wave();
    read(&mut w, 0x51, 0xfe, &[1, 2, 3]);
    let e = events(&decode(
        "eeprom24xx-memory",
        &json!({"chip":"24c04"}),
        &w.finish(),
        &[],
    ));
    assert_eq!(
        e.iter()
            .filter(|v| v["kind"] == "data")
            .map(|v| v["address"].clone())
            .collect::<Vec<_>>(),
        vec![json!(510), json!(511), json!(0)]
    );
    let mut w = wave();
    w.start();
    w.byte(0xa0, true);
    w.byte(7, true);
    w.byte(9, false);
    w.stop();
    w.idle(8);
    let e = events(&decode(
        "eeprom24xx-memory",
        &json!({"chip":"24c02"}),
        &w.finish(),
        &[],
    ));
    assert!(e.iter().any(|v| v["kind"] == "data" && v["valid"] == false));
    assert!(!e.iter().any(|v| v["writeRequested"] == true));
}
#[test]
fn eeprom_34aa04_bank_dummy_nacks_and_readback() {
    let mut w = wave();
    w.start();
    w.byte(0x6e, true);
    w.byte(0, false);
    w.byte(0, false);
    w.stop();
    w.idle(8);
    read(&mut w, 0x50, 0xff, &[1, 2]);
    w.start();
    w.byte(0x6d, false);
    w.byte(0, false);
    w.stop();
    w.idle(8);
    let e = events(&decode(
        "eeprom24xx-memory",
        &json!({"chip":"34aa04"}),
        &w.finish(),
        &[],
    ));
    assert!(
        e.iter()
            .any(|v| v["kind"] == "bank" && v["bank"] == 1 && v["valid"] == true)
    );
    assert_eq!(
        e.iter()
            .filter(|v| v["kind"] == "data")
            .map(|v| v["address"].clone())
            .collect::<Vec<_>>(),
        vec![json!(511), json!(256)]
    );
}
#[test]
fn edid_standard_timings_and_streamed_monitor_name() {
    let mut w = wave();
    read(&mut w, 0x50, 19, &[3]);
    read(&mut w, 0x50, 38, &[0x81, 0x80, 1, 1]);
    read(&mut w, 0x50, 54, &[0, 0, 0, 0xfc, 0, b'A', b'B', 10]);
    read(&mut w, 0x50, 72, &[0, 0, 0, 0xfb, 0, 2]);
    read(&mut w, 0x50, 90, &[0, 0, 0, 0xfa, 0, 3]);
    let e = events(&decode("edid", &json!({}), &w.finish(), &[]));
    field(&e, "standard-horizontal", 1280, false, true);
    field(&e, "standard-vertical", 1024, false, true);
    field(&e, "standard-refresh-hz", 60, false, true);
    field(&e, "monitor-name-byte", 65, false, true);
    field(&e, "additional-color-point-byte", 2, false, true);
    field(&e, "additional-standard-timing-byte", 3, false, true);
}

#[test]
fn unrelated_address_then_selected_restart_in_same_transaction() {
    for (name, address) in [
        ("ds1307", 0x68),
        ("rtc8564", 0x51),
        ("lm75", 0x48),
        ("mlx90614", 0x5a),
        ("adxl345", 0x53),
        ("mcp230xx", 0x20),
        ("pca9571", 0x25),
        ("tca6408a", 0x20),
        ("edid", 0x50),
        ("ltc26x7", 0x10),
        ("mxc6225xu", 0x15),
        ("nunchuk", 0x52),
        ("atsha204a", 0x64),
        ("xfp", 0x50),
        ("eeprom24xx-memory", 0x50),
    ] {
        let mut w = wave();
        w.start();
        w.byte(0x22, false);
        w.byte(0, true);
        w.restart();
        w.byte(address * 2, true);
        w.byte(0, true);
        w.byte(0, true);
        w.stop();
        w.idle(8);
        let e = events(&decode(name, &json!({}), &w.finish(), &[]));
        assert!(
            e.iter().any(|v| v["kind"] == "begin" && v["ack"] == true),
            "{name}: {e:?}"
        );
    }
}

#[test]
fn cat24m01_generic_17bit_address_and_page_wrap() {
    let config = json!({"chip":"generic","capacity":131072,"page_bytes":256,"address_bytes":2,"block_bits":1});
    let mut w = wave();
    write(&mut w, 0x51, &[0xff, 0xff, 0xa5, 0x5a]);
    w.start();
    w.byte(0xa2, true);
    w.byte(0xff, true);
    w.byte(0xff, true);
    w.restart();
    w.byte(0xa3, true);
    w.byte(1, true);
    w.byte(2, false);
    w.stop();
    w.idle(8);
    let e = events(&decode("eeprom24xx-memory", &config, &w.finish(), &[]));
    assert_eq!(
        e.iter()
            .filter(|v| v["kind"] == "data")
            .map(|v| v["address"].clone())
            .collect::<Vec<_>>(),
        vec![json!(131071), json!(130816), json!(131071), json!(0)]
    );
}

#[test]
fn edid_chromaticity_established_full_dtd_and_range_limits() {
    let mut w = wave();
    read(&mut w, 0x50, 19, &[4]);
    read(
        &mut w,
        0x50,
        25,
        &[0x6c, 0x93, 0x80, 0x40, 0x60, 0x20, 0x30, 0x50, 0x70, 0x90],
    );
    read(&mut w, 0x50, 35, &[0x21, 0x81, 0x80]);
    read(
        &mut w,
        0x50,
        54,
        &[
            0x02, 0x3a, 0x80, 0x18, 0x71, 0x38, 0x2d, 0x40, 0x58, 0x2c, 0x45, 0, 0xfd, 0x1e, 0x11,
            0, 0, 0x1e,
        ],
    );
    read(
        &mut w,
        0x50,
        72,
        &[0, 0, 0, 0xfd, 0x0f, 50, 75, 30, 83, 17, 1],
    );
    let e = events(&decode("edid", &json!({}), &w.finish(), &[]));
    field(&e, "red-x-1024", 513, false, true);
    field(&e, "red-y-1024", 258, false, true);
    field(&e, "green-x-1024", 387, false, true);
    field(&e, "white-y-1024", 579, false, true);
    assert_eq!(e.iter().filter(|v| v["kind"] == "timing").count(), 5);
    assert!(e.iter().any(|v| v["kind"] == "timing"
        && v["width"] == 1280
        && v["height"] == 1024
        && v["refresh"] == 75));
    for (name, value) in [
        ("pixel-clock-hz", 148_500_000),
        ("horizontal-active", 1920),
        ("horizontal-blanking", 280),
        ("vertical-active", 1080),
        ("vertical-blanking", 45),
        ("horizontal-front-porch", 88),
        ("horizontal-sync-width", 44),
        ("vertical-front-porch", 4),
        ("vertical-sync-width", 5),
        ("horizontal-millimeters", 509),
        ("vertical-millimeters", 286),
        ("sync-format", 3),
        ("sync-flags", 3),
        ("minimum-vertical-hz", 305),
        ("maximum-vertical-hz", 330),
        ("minimum-horizontal-khz", 285),
        ("maximum-horizontal-khz", 338),
        ("maximum-pixel-clock-hz", 170_000_000),
    ] {
        field(&e, name, value, false, true)
    }
}

fn cta_block() -> [u8; 128] {
    let mut block = [0u8; 128];
    block[..22].copy_from_slice(&[
        2, 3, 22, 0xf1, 0x42, 0x90, 4, 0x23, 9, 7, 7, 0x83, 1, 0, 0, 0x63, 3, 12, 0, 0xe2, 5, 3,
    ]);
    block[22..40].copy_from_slice(&[
        0x02, 0x3a, 0x80, 0x18, 0x71, 0x38, 0x2d, 0x40, 0x58, 0x2c, 0x45, 0, 0xfd, 0x1e, 0x11, 0,
        0, 0x1e,
    ]);
    block[127] = 0u8.wrapping_sub(block[..127].iter().fold(0u8, |s, &b| s.wrapping_add(b)));
    block
}
fn segment_read(w: &mut I2cWaveform, segment: u8, data: &[u8]) {
    w.start();
    w.byte(0x60, true);
    w.byte(segment, true);
    w.restart();
    w.byte(0xa0, true);
    w.byte(0, true);
    w.restart();
    w.byte(0xa1, true);
    for (i, &b) in data.iter().enumerate() {
        w.byte(b, i + 1 < data.len())
    }
    w.stop();
    w.idle(8);
}
#[test]
fn eddc_segment_cta_streaming_descriptors_and_reset() {
    let mut w = wave();
    segment_read(&mut w, 1, &cta_block());
    read(&mut w, 0x50, 0, &[0, 255, 255, 255, 255, 255, 255, 0]);
    let b = w.finish();
    let e = events(&decode("edid", &json!({}), &b, &[]));
    assert_eq!(e.iter().filter(|v| v["kind"] == "cta-block").count(), 5);
    assert!(
        e.iter()
            .any(|v| v["kind"] == "cta-video" && v["code"] == 16 && v["native"] == true)
    );
    assert!(e.iter().any(|v| v["kind"] == "cta-audio"
        && v["format"] == 1
        && v["channels"] == 2
        && v["rates"] == 7
        && v["detail"] == 7
        && v["valid"] == true));
    field(&e, "cta-vendor-oui", 0xc03, false, true);
    field(&e, "cta-speaker-mask", 1, false, true);
    field(&e, "cta-extended-tag", 5, false, true);
    assert!(
        e.iter()
            .any(|v| v["kind"] == "byte" && v["absolute"] == 256)
    );
    assert!(
        e.iter()
            .any(|v| v["field"] == "checksum" && v["block"] == 2 && v["valid"] == true)
    );
    assert!(
        e.iter()
            .any(|v| v["field"] == "header-byte" && v["block"] == 0)
    );
    for interpreted in [false, true] {
        assert_block_invariant(
            |g| harness("edid", &json!({}), interpreted, g),
            &b,
            &[TickRange::new(Tick(500), Tick(530))],
        );
    }
}
#[test]
fn edid_cta_bounds_and_fragmented_fields_never_reuse_old_context() {
    let block = cta_block();
    for split in 1..40 {
        let mut w = wave();
        segment_read(&mut w, 1, &block[..split]);
        read(&mut w, 0x50, split as u8, &block[split..40]);
        let e = events(&decode("edid", &json!({}), &w.finish(), &[]));
        // The second read is in segment zero because the preceding read ended with NACK/STOP.
        assert!(
            !e.iter()
                .any(|v| v["kind"] == "field" && v["field"] == "checksum")
        );
        assert!(
            !e.iter()
                .any(|v| v["kind"] == "byte" && v["absolute"] == 256 + split)
        );
    }
    let mut bad = block;
    bad[2] = 7;
    bad[4] = 0x5f;
    let mut w = wave();
    segment_read(&mut w, 1, &bad);
    let e = events(&decode("edid", &json!({}), &w.finish(), &[]));
    assert!(
        e.iter()
            .any(|v| v["kind"] == "cta-block" && v["valid"] == false)
    );
    assert!(!e.iter().any(|v| v["kind"] == "cta-video"));
    let mut w = wave();
    read(&mut w, 0x50, 0, &[0, 255]);
    let e = events(&decode(
        "edid",
        &json!({"initial_segment":null}),
        &w.finish(),
        &[],
    ));
    assert!(!e.iter().any(|v| v["field"] == "header-byte"));
}

#[test]
fn xfp_thresholds_control_and_complete_page_one_named_profile() {
    let mut low = [0u8; 112];
    low[0] = 6;
    low[1] = 0x36;
    low[2..4].copy_from_slice(&[0xff, 0x80]);
    low[18..20].copy_from_slice(&[1, 0]);
    low[58] = 0x21;
    low[59] = 0x31;
    low[70] = 0x12;
    low[71] = 0x34;
    low[88] = 0x80;
    let mut high = [0u8; 95];
    high[1] = 0xd8;
    high[2] = 7;
    high[3] = 0xc0;
    high[11] = 0xc0;
    high[12] = 100;
    high[13] = 110;
    high[14] = 10;
    high[15] = 255;
    high[19] = 0x5f;
    high[36] = 0x81;
    high[58..62].copy_from_slice(&[0x77, 0xdd, 0, 47]);
    high[62] = 85;
    high[64..68].copy_from_slice(&[100, 10, 0x34, 0x56]);
    high[84..90].copy_from_slice(b"240229");
    high[92] = 0x18;
    high[93] = 0xe7;
    high[94] = 0x13;
    let mut w = wave();
    read(&mut w, 0x50, 0, &low);
    read(&mut w, 0x50, 128, &high);
    let e = events(&decode("xfp", &json!({}), &w.finish(), &[]));
    assert!(e.iter().any(|v| v["kind"] == "threshold"
        && v["channel"] == "temperature-256ths-celsius"
        && v["level"] == "high-alarm"
        && v["value"] == 128
        && v["negative"] == true));
    assert!(e.iter().any(|v| v["kind"] == "threshold"
        && v["channel"] == "tx-bias-2microamp"
        && v["value"] == 256));
    for (name, value) in [
        ("signal-data-rate-megabits", 10100),
        ("lineside-loopback", 1),
        ("xfi-loopback", 1),
        ("vps-lowest-millivolts", 1600),
        ("vps-supply-millivolts", 1700),
        ("vps-bypassed-millivolts", 1500),
        ("ber-byte", 0x12),
        ("interrupt-mask", 128),
        ("power-class", 4),
        ("cdr-present", 1),
        ("refclk-required", 0),
        ("clei-present", 1),
        ("minimum-bitrate-megabits", 10000),
        ("maximum-bitrate-megabits", 11000),
        ("wavelength-picometers", 1534250),
        ("wavelength-tolerance-picometers", 235),
        ("maximum-case-celsius", 85),
        ("maximum-power-milliwatts", 2000),
        ("power-down-milliwatts", 100),
        ("current-5v-milliamps", 150),
        ("current-33v-milliamps", 400),
        ("current-18v-milliamps", 500),
        ("current-negative-5v-milliamps", 300),
        ("manufacture-year", 2024),
        ("manufacture-month", 2),
        ("manufacture-day", 29),
        ("rx-power-average", 1),
    ] {
        field(&e, name, value, false, true)
    }
    assert!(
        e.iter()
            .any(|v| v["kind"] == "description" && v["description"] == "lc")
    );
    assert!(
        e.iter()
            .any(|v| v["kind"] == "description" && v["description"] == "1550nm-dfb")
    );
    for name in [
        "10gbase-sr",
        "10gbase-lr",
        "encoding-64b66b",
        "encoding-8b10b",
        "cdr-9950",
        "xfi-loopback-supported",
        "soft-tx-disable",
    ] {
        assert!(
            e.iter()
                .any(|v| v["kind"] == "capability" && v["name"] == name && v["enabled"] == true),
            "{name}"
        )
    }
    assert!(e.iter().any(|v| v["kind"] == "link"
        && v["medium"] == "single-mode"
        && v["meters"] == 10000
        && v["greaterThan"] == false));
    assert!(e.iter().any(|v| v["kind"] == "link"
        && v["medium"] == "extended-50um"
        && v["meters"] == 508
        && v["greaterThan"] == true));
    assert!(e.iter().any(|v| v["kind"] == "link"
        && v["medium"] == "50um"
        && v["specified"] == false
        && v["meters"].is_null()));
    assert!(e.iter().any(|v| v["kind"] == "auxiliary-type"
        && v["channel"] == 1
        && v["type"] == "apd-bias-voltage"));
}
#[test]
fn xfp_split_threshold_wavelength_date_and_unknown_table_are_conservative() {
    for (pointer, first, second) in [(2, 0xff, 0x80), (186, 0x77, 0xdd), (212, b'2', b'4')] {
        let mut w = wave();
        read(&mut w, 0x50, pointer, &[first]);
        w.start();
        w.byte(0xa1, true);
        w.byte(second, false);
        w.stop();
        w.idle(8);
        let e = events(&decode("xfp", &json!({}), &w.finish(), &[]));
        assert!(!e.iter().any(|v| v["kind"] == "threshold"
            || v["field"] == "wavelength-picometers"
            || v["field"] == "manufacture-year"));
    }
    let mut w = wave();
    write(&mut w, 0x50, &[127, 2]);
    read(&mut w, 0x50, 131, &[0xff]);
    let e = events(&decode("xfp", &json!({}), &w.finish(), &[]));
    assert!(!e.iter().any(|v| v["kind"] == "capability"));
    field(&e, "unknown-table", 255, false, true);
}
