// SPDX-License-Identifier: Apache-2.0
//! SD SDR wire vectors follow Physical Layer Simplified Specification v2.00.
mod dsl_spi_support;
use serde_json::{Value, json};
use uiharu_core::testing::{Harness, assert_block_invariant};
use uiharu_core::{
    GraphConfig, Item, Levels, LogicBlock, LogicBlockBuilder, Tick, TickRange, TimeBase,
};
use uiharu_dsl_protocols::sd_native::{Config, Event, SdNative};
struct SdWire {
    b: LogicBlockBuilder,
    levels: u64,
}
impl SdWire {
    fn new() -> Self {
        Self {
            b: LogicBlockBuilder::new(Tick::ZERO, 8),
            levels: 6,
        }
    }
    fn bit(&mut self, cmd: bool, dat: bool) {
        self.levels = if cmd {
            self.levels | 2
        } else {
            self.levels & !2
        };
        self.levels = if dat {
            self.levels | 4
        } else {
            self.levels & !4
        };
        self.b.push_repeated(Levels(self.levels), 2);
        self.b.push_repeated(Levels(self.levels | 1), 2)
    }
    fn idle(&mut self, n: usize) {
        for _ in 0..n {
            self.bit(true, true)
        }
    }
    fn command(&mut self, host: bool, index: u8, arg: u32) {
        let header = if host { 0x40 | index } else { index };
        let value = ((header as u64) << 32) | arg as u64;
        let mut crc = 0u8;
        for i in (0..40).rev() {
            let bit = ((value >> i) & 1) as u8;
            let feedback = (crc >> 6) ^ bit;
            crc = (crc << 1) & 127;
            if feedback & 1 != 0 {
                crc ^= 9
            }
            self.bit(bit != 0, true)
        }
        for i in (0..7).rev() {
            self.bit(crc & (1 << i) != 0, true)
        }
        self.bit(true, true)
    }
    fn data(&mut self, value: u8, crc: u16, write: bool) {
        self.bit(true, false);
        for i in (0..8).rev() {
            self.bit(true, value & (1 << i) != 0)
        }
        for i in (0..16).rev() {
            self.bit(true, crc & (1 << i) != 0)
        }
        self.bit(true, true);
        if write {
            for bit in [false, false, true, false, true, false, false, false, true] {
                self.bit(true, bit)
            }
        }
    }
    fn finish(self) -> LogicBlock {
        self.b.finish().expect("valid test vector")
    }
}
fn harness(g: GraphConfig) -> Harness<Event> {
    let config: Config =
        serde_json::from_value(json!({"clk":0,"cmd":1,"dat0":2,"width":"one","block_bytes":1}))
            .expect("valid test vector");
    Harness::build(8, TimeBase::new(1_000_000), g, |graph, source| {
        let n = graph.add(SdNative::new(config).expect("valid test vector"));
        graph.connect(source.samples, SdNative::input(n))?;
        Ok(SdNative::events(n))
    })
    .expect("valid test vector")
}
fn output(h: &Harness<Event>) -> Vec<Value> {
    h.items
        .iter()
        .filter_map(|item| {
            if let Item::Event(e) = item {
                Some(serde_json::to_value(e.payload).expect("valid test vector"))
            } else {
                None
            }
        })
        .collect()
}
fn has(out: &[Value], expected: Value) {
    assert!(
        out.iter().any(|v| expected
            .as_object()
            .expect("valid test vector")
            .iter()
            .all(|(k, value)| v.get(k) == Some(value))),
        "missing {expected}: {out:?}"
    )
}
fn transfer(write: bool, crc: u16) -> LogicBlock {
    let mut w = SdWire::new();
    w.idle(10);
    let command = if write { 24 } else { 17 };
    w.command(true, command, 0x100);
    w.idle(2);
    w.command(false, command, 0);
    w.idle(2);
    w.data(0xa5, crc, write);
    w.idle(3);
    w.finish()
}
#[test]
fn write_data_crc_status_and_busy_are_separate_events() {
    // CRC-16/XMODEM of single byte A5 is E54F; card status token 0 010 1.
    let block = transfer(true, 0xe54f);
    let mut h = harness(GraphConfig {
        queue_capacity: 1,
        budget: 1,
    });
    h.feed(&block, &[1, 7, 11]);
    h.end();
    dsl_spi_support::logic_parity(
        "sd-native",
        json!({"clk":0,"cmd":1,"dat0":2,"width":"one","block_bytes":1}),
        GraphConfig {
            queue_capacity: 1,
            budget: 1,
        },
        &block,
        &[],
        &[1, 7, 11],
        &h.items,
    );
    let out = output(&h);
    has(&out, json!({"kind":"data-begin","read":false,"command":24}));
    has(&out, json!({"kind":"data","value":0xa5}));
    has(&out, json!({"kind":"data-crc","value":0xe54f,"ok":true}));
    has(
        &out,
        json!({"kind":"data-status","status":2,"accepted":true,"endOk":true}),
    );
    has(&out, json!({"kind":"busy","ticks":12}));
    has(&out, json!({"kind":"data-end","reason":"complete"}));
}
#[test]
fn read_crc_error_is_reported_without_relabeling_payload() {
    let block = transfer(false, 0);
    let mut h = harness(GraphConfig::default());
    h.feed(&block, &[3]);
    h.end();
    dsl_spi_support::logic_parity(
        "sd-native",
        json!({"clk":0,"cmd":1,"dat0":2,"width":"one","block_bytes":1}),
        GraphConfig::default(),
        &block,
        &[],
        &[3],
        &h.items,
    );
    let out = output(&h);
    has(&out, json!({"kind":"data","value":0xa5}));
    has(
        &out,
        json!({"kind":"data-crc","ok":false,"expected":0xe54f}),
    );
    assert!(!out.iter().any(|v| v["kind"] == "busy"));
}
#[test]
fn native_read_write_are_invariant_across_blocks_budgets_and_gaps() {
    for write in [false, true] {
        let block = transfer(write, 0xe54f);
        assert_block_invariant(harness, &block, &[TickRange::new(Tick(459), Tick(467))]);
    }
}
#[test]
fn missing_four_bit_lanes_and_duplicate_channels_are_rejected() {
    assert!(
        SdNative::new(
            serde_json::from_value(json!({"width":"four","dat0":2})).expect("valid test vector")
        )
        .is_err()
    );
    assert!(
        SdNative::new(serde_json::from_value(json!({"clk":0,"cmd":0})).expect("valid test vector"))
            .is_err()
    );
}
#[test]
fn four_bit_sd_checks_each_lane_crc_independently() {
    let mut w = SdWire::new();
    w.levels = 62;
    w.idle(10);
    w.command(true, 0, 0);
    w.idle(2);
    w.command(true, 17, 0);
    w.idle(2);
    w.command(false, 17, 0);
    w.idle(2);
    let mut groups = vec![0, 0xa, 5];
    // Lane0/2 receive 01 =>1021; lane1/3 receive10 =>2042.
    let crcs = [0x1021u16, 0x2042, 0x1021, 0x2043]; // deliberately corrupt lane3 only.
    for bit in (0..16).rev() {
        let mut group = 0u64;
        for (lane, crc) in crcs.iter().enumerate() {
            group |= (((crc >> bit) & 1) as u64) << lane
        }
        groups.push(group)
    }
    groups.push(15);
    for group in groups {
        w.levels = (w.levels & !0x3c) | (group << 2);
        w.b.push_repeated(Levels(w.levels), 2);
        w.b.push_repeated(Levels(w.levels | 1), 2)
    }
    w.idle(3);
    let block = w.finish();
    let config: Config = serde_json::from_value(
        json!({"clk":0,"cmd":1,"dat0":2,"dat1":3,"dat2":4,"dat3":5,"width":"four","block_bytes":1}),
    )
    .expect("valid test vector");
    let mut h = Harness::<Event>::build(
        8,
        TimeBase::new(1_000_000),
        GraphConfig {
            queue_capacity: 1,
            budget: 1,
        },
        |graph, source| {
            let n = graph.add(SdNative::new(config).expect("valid test vector"));
            graph.connect(source.samples, SdNative::input(n))?;
            Ok(SdNative::events(n))
        },
    )
    .expect("valid test vector");
    h.feed(&block, &[1, 9]);
    h.end();
    let out = output(&h);
    has(&out, json!({"kind":"data","value":0xa5}));
    for lane in 0..4 {
        has(&out, json!({"kind":"data-crc","lane":lane,"ok":lane!=3}))
    }
    dsl_spi_support::logic_parity(
        "sd-native",
        serde_json::to_value(config).expect("valid test vector"),
        GraphConfig {
            queue_capacity: 1,
            budget: 1,
        },
        &block,
        &[],
        &[1, 9],
        &h.items,
    );
}
#[test]
fn auto_width_requires_accepted_cmd55_and_acmd6_response() {
    for (app, error, expected) in [
        (true, false, true),
        (true, true, false),
        (false, false, false),
    ] {
        let mut w = SdWire::new();
        w.levels = 62;
        w.idle(10);
        if app {
            w.command(true, 55, 0);
            w.idle(2);
            w.command(false, 55, 32);
            w.idle(2)
        }
        w.command(true, 6, 2);
        w.idle(2);
        w.command(false, 6, if error { 1 << 22 } else { 0 });
        w.idle(3);
        let block = w.finish();
        let config:Config=serde_json::from_value(json!({"clk":0,"cmd":1,"dat0":2,"dat1":3,"dat2":4,"dat3":5,"width":"auto","block_bytes":1})).expect("valid test vector");
        let mut h = Harness::<Event>::build(
            8,
            TimeBase::new(1_000_000),
            GraphConfig {
                queue_capacity: 1,
                budget: 1,
            },
            |graph, source| {
                let n = graph.add(SdNative::new(config).expect("valid test vector"));
                graph.connect(source.samples, SdNative::input(n))?;
                Ok(SdNative::events(n))
            },
        )
        .expect("valid test vector");
        h.feed(&block, &[1, 7]);
        h.end();
        let out = output(&h);
        assert_eq!(out.iter().any(|v| v["kind"] == "bus-width"), expected);
        if expected {
            has(&out, json!({"kind":"bus-width","lanes":4,"applied":true}))
        }
        dsl_spi_support::logic_parity(
            "sd-native",
            serde_json::to_value(config).expect("valid test vector"),
            GraphConfig {
                queue_capacity: 1,
                budget: 1,
            },
            &block,
            &[],
            &[1, 7],
            &h.items,
        );
    }
}
#[test]
fn scr_fixed_eight_bytes_and_fields_follow_accepted_cmd55_acmd51() {
    for (four, app, error, bad_crc) in [
        (false, true, false, false),
        (true, true, false, false),
        (true, true, false, true),
        (false, false, false, false),
        (false, true, true, false),
    ] {
        let mut w = SdWire::new();
        if four {
            w.levels = 62
        }
        w.idle(10);
        if app {
            w.command(true, 55, 0);
            w.idle(2);
            w.command(false, 55, if error { 32 | (1 << 22) } else { 32 });
            w.idle(2);
        }
        w.command(true, 51, 0);
        w.idle(2);
        w.command(false, 51, 0);
        w.idle(2);
        let values = [0x02u8, 0xb5, 0x84, 0x03, 0x12, 0x34, 0x56, 0x78];
        if four {
            let mut groups = vec![0u64];
            for value in values {
                groups.push((value >> 4) as u64);
                groups.push((value & 15) as u64)
            }
            let crcs = [
                0x2204u16,
                0x347b,
                0x3c48,
                if bad_crc { 0x9f6f } else { 0x9f6e },
            ];
            for bit in (0..16).rev() {
                let mut group = 0;
                for (lane, crc) in crcs.iter().enumerate() {
                    group |= (((crc >> bit) & 1) as u64) << lane
                }
                groups.push(group)
            }
            groups.push(15);
            for group in groups {
                w.levels = (w.levels & !0x3c) | (group << 2);
                w.b.push_repeated(Levels(w.levels), 2);
                w.b.push_repeated(Levels(w.levels | 1), 2)
            }
        } else {
            w.bit(true, false);
            for value in values {
                for bit in (0..8).rev() {
                    w.bit(true, value & (1 << bit) != 0)
                }
            }
            for bit in (0..16).rev() {
                w.bit(true, 0x8c0au16 & (1 << bit) != 0)
            }
            w.bit(true, true);
        }
        w.idle(3);
        let block = w.finish();
        let config:Config=serde_json::from_value(json!({"clk":0,"cmd":1,"dat0":2,"dat1":if four{Some(3)}else{None},"dat2":if four{Some(4)}else{None},"dat3":if four{Some(5)}else{None},"width":if four{"four"}else{"one"},"block_bytes":512})).expect("config");
        let g = GraphConfig {
            queue_capacity: 1,
            budget: 1,
        };
        let mut h = Harness::<Event>::build(8, TimeBase::new(1_000_000), g, |graph, source| {
            let node = graph.add(SdNative::new(config).expect("sd"));
            graph.connect(source.samples, SdNative::input(node))?;
            Ok(SdNative::events(node))
        })
        .expect("graph");
        h.feed(&block, &[1, 17]);
        h.end();
        let out = output(&h);
        if app && !error {
            has(&out, json!({"kind":"data-begin","command":51,"bytes":8}));
            has(
                &out,
                json!({"kind":"scr","sdSpec":2,"security":3,"busWidths":5,"spec3":true,"spec4":true,"commandSupport":3,"vendorData":0x12345678,"crcOk":!bad_crc}),
            );
        } else {
            assert!(!out.iter().any(|event| event["kind"] == "scr"));
        }
        dsl_spi_support::logic_parity(
            "sd-native",
            serde_json::to_value(config).expect("config"),
            g,
            &block,
            &[],
            &[1, 17],
            &h.items,
        );
    }
}

#[test]
fn wrong_index_r1_never_accepts_application_data_or_bus_width() {
    for command in [55, 6, 17] {
        let mut w = SdWire::new();
        w.levels = 62;
        w.idle(10);
        if command == 6 {
            w.command(true, 55, 0);
            w.idle(2);
            w.command(false, 55, 32);
            w.idle(2);
        }
        w.command(true, command, if command == 6 { 2 } else { 0 });
        w.idle(2);
        w.command(false, 13, if command == 55 { 32 } else { 0 });
        w.idle(2);
        if command == 55 {
            w.command(true, 51, 0);
            w.idle(2);
            w.command(false, 51, 0);
            w.idle(2);
        }
        w.data(0xa5, 0xe54f, false);
        w.idle(3);
        let block = w.finish();
        let config:Config=serde_json::from_value(json!({"clk":0,"cmd":1,"dat0":2,"dat1":3,"dat2":4,"dat3":5,"width":"auto","block_bytes":1})).expect("config");
        let g = GraphConfig {
            queue_capacity: 1,
            budget: 1,
        };
        let mut h = Harness::<Event>::build(8, TimeBase::new(1_000_000), g, |graph, source| {
            let node = graph.add(SdNative::new(config).expect("sd"));
            graph.connect(source.samples, SdNative::input(node))?;
            Ok(SdNative::events(node))
        })
        .expect("graph");
        h.feed(&block, &[1, 19]);
        h.end();
        assert!(!output(&h).iter().any(|event| event["kind"] == "data-begin"
            || event["kind"] == "bus-width"
            || event["kind"] == "scr"));
        dsl_spi_support::logic_parity(
            "sd-native",
            serde_json::to_value(config).expect("config"),
            g,
            &block,
            &[],
            &[1, 19],
            &h.items,
        );
    }
}

fn with_register_crc(mut bytes: [u8; 16]) -> [u8; 16] {
    let mut crc = 0u8;
    for byte in &bytes[..15] {
        for bit in (0..8).rev() {
            let feedback = (crc >> 6) ^ (byte >> bit);
            crc = (crc << 1) & 127;
            if feedback & 1 != 0 {
                crc ^= 9;
            }
        }
    }
    bytes[15] = (crc << 1) | 1;
    bytes
}
impl SdWire {
    fn register(&mut self, bytes: [u8; 16]) {
        // R2's leading byte is start/card/reserved, followed by the 128-bit register.
        for bit in (0..8).rev() {
            self.bit(0x3f & (1 << bit) != 0, true);
        }
        for byte in bytes {
            for bit in (0..8).rev() {
                self.bit(byte & (1 << bit) != 0, true);
            }
        }
    }
}
#[test]
fn response_status_rca_voltage_and_echo_are_named_observations() {
    let mut w = SdWire::new();
    w.idle(10);
    w.command(true, 13, 0x1234_0000);
    w.idle(2);
    w.command(false, 13, 0x8040_0920);
    w.idle(2);
    w.command(true, 3, 0);
    w.idle(2);
    w.command(false, 3, 0x1234_e900);
    w.idle(2);
    w.command(true, 8, 0x1aa);
    w.idle(2);
    w.command(false, 8, 0x1aa);
    w.idle(2);
    w.command(true, 8, 0x1aa);
    w.idle(2);
    w.command(false, 8, 0x255);
    w.idle(2);
    w.command(true, 7, 0x1234_0000);
    w.idle(2);
    w.command(false, 7, 0x900);
    w.idle(2);
    // Wrong response index remains raw; no false R6 association.
    w.command(true, 3, 0);
    w.idle(2);
    w.command(false, 8, 0xdead_beef);
    w.idle(2);
    let block = w.finish();
    let limits = GraphConfig {
        queue_capacity: 1,
        budget: 2,
    };
    let mut h = harness(limits);
    h.feed(&block, &[1, 7, 29]);
    h.end();
    let out = output(&h);
    has(
        &out,
        json!({"kind":"card-status","format":"r1","value":0x80400920u32,"currentState":4,"readyForData":true,"appCommand":true}),
    );
    has(
        &out,
        json!({"kind":"status-field","field":"out-of-range","value":1}),
    );
    has(
        &out,
        json!({"kind":"status-field","field":"illegal-command","value":1}),
    );
    has(
        &out,
        json!({"kind":"relative-address","address":0x1234,"status":0xe900,"crcOk":true}),
    );
    has(
        &out,
        json!({"kind":"card-status","format":"r6","value":0xc80900,"relativeAddress":0x1234}),
    );
    has(
        &out,
        json!({"kind":"interface-condition","voltage":1,"voltageRange":"v27-36","pattern":0xaa,"patternOk":true,"voltageOk":true}),
    );
    has(
        &out,
        json!({"kind":"interface-condition","voltage":2,"pattern":0x55,"patternOk":false,"voltageOk":false}),
    );
    has(&out, json!({"kind":"card-status","format":"r1b"}));
    assert_eq!(
        out.iter()
            .filter(|e| e["kind"] == "relative-address")
            .count(),
        1
    );
    dsl_spi_support::logic_parity(
        "sd-native",
        json!({"clk":0,"cmd":1,"dat0":2,"width":"one","block_bytes":1}),
        limits,
        &block,
        &[],
        &[1, 7, 29],
        &h.items,
    );
    assert_block_invariant(harness, &block, &[]);
}
#[test]
fn cid_product_name_and_all_csd_v1_fields_survive_r2_and_crc() {
    let cid = with_register_crc([
        3, b'S', b'D', b'A', b'B', b'C', b'D', b'E', 0x21, 0x12, 0x34, 0x56, 0x78, 1, 0x94, 0,
    ]);
    // Manufacturer table bit positions, independently packed into the 128-bit register.
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
        word |= (value as u128 & ((1u128 << width) - 1)) << low;
    }
    let csd = with_register_crc(word.to_be_bytes());
    let mut w = SdWire::new();
    w.idle(10);
    w.command(true, 2, 0);
    w.idle(2);
    w.register(cid);
    w.idle(2);
    w.command(true, 9, 0);
    w.idle(2);
    w.register(csd);
    w.idle(3);
    let block = w.finish();
    let limits = GraphConfig {
        queue_capacity: 1,
        budget: 3,
    };
    let mut h = harness(limits);
    h.feed(&block, &[1, 13, 29]);
    h.end();
    let out = output(&h);
    has(
        &out,
        json!({"kind":"cid","productName":[65,66,67,68,69],"manufacturer":3,"oem":0x5344,"revision":0x21,"serial":0x12345678,"year":2025,"month":4,"crcOk":true}),
    );
    has(
        &out,
        json!({"kind":"csd","structure":0,"size":0xabc,"blocks":351872,"taac":0x2e,"nsac":0x11,"transferSpeed":0x32,"commandClasses":0x5b5,"readPartial":true,"writeMisalign":true,"readMisalign":true,"dsrImplemented":true,"readCurrentMin":1,"readCurrentMax":2,"writeCurrentMin":3,"writeCurrentMax":4,"sizeMultiplier":5,"eraseSingleBlock":true,"eraseSectorBlocks":19,"writeProtectGroupSectors":36,"writeProtectGroupEnabled":true,"writeSpeedFactor":3,"writeBlockBits":9,"writePartial":true,"fileFormatGroup":true,"copied":true,"permanentWriteProtect":true,"temporaryWriteProtect":true,"fileFormat":2,"crcOk":true,"endOk":true}),
    );
    dsl_spi_support::logic_parity(
        "sd-native",
        json!({"clk":0,"cmd":1,"dat0":2,"width":"one","block_bytes":1}),
        limits,
        &block,
        &[],
        &[1, 13, 29],
        &h.items,
    );
    assert_block_invariant(harness, &block, &[]);
}
