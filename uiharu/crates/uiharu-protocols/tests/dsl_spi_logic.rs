// SPDX-License-Identifier: Apache-2.0
//! Independent lane groups from Winbond's SDR/DTR instruction timing diagrams.
mod dsl_spi_support;
use serde_json::{Value, json};
use uiharu_core::testing::Harness;
use uiharu_core::{
    GraphConfig, Item, Levels, LogicBlock, LogicBlockBuilder, Tick, TickRange, TimeBase,
};
use uiharu_dsl_protocols::qspi_flash::{FlashEvent, QspiFlash, QspiFlashConfig};

struct Wire {
    builder: LogicBlockBuilder,
    levels: u64,
}
impl Wire {
    fn new() -> Self {
        Self {
            builder: LogicBlockBuilder::new(Tick::ZERO, 8),
            levels: 1,
        }
    }
    fn hold(&mut self, n: u64) {
        self.builder.push_repeated(Levels(self.levels), n);
    }
    fn set(&mut self, channel: u8, high: bool) {
        if high {
            self.levels |= 1 << channel
        } else {
            self.levels &= !(1 << channel)
        }
    }
    fn select(&mut self) {
        self.hold(3);
        self.set(0, false);
        self.hold(2)
    }
    fn release(&mut self) {
        self.set(0, true);
        self.hold(3)
    }
    fn group(&mut self, value: u8, lanes: u8, read: bool) {
        for lane in 0..lanes {
            self.set(
                if lanes == 1 && read { 3 } else { 2 + lane },
                value & (1 << lane) != 0,
            )
        }
    }
    fn sdr(&mut self, value: u32, bits: u8, lanes: u8, read: bool) {
        for shift in (0..bits / lanes).rev() {
            self.group((value >> (shift * lanes)) as u8, lanes, read);
            self.hold(2);
            self.set(1, true);
            self.hold(2);
            self.set(1, false)
        }
        self.hold(2)
    }
    fn dtr(&mut self, value: u32, bits: u8, lanes: u8, read: bool) {
        for shift in (0..bits / lanes).rev() {
            self.group((value >> (shift * lanes)) as u8, lanes, read);
            self.hold(2);
            self.levels ^= 2;
            self.hold(2)
        }
    }
    fn dummy(&mut self, clocks: u8, dtr: bool) {
        if dtr {
            for _ in 0..clocks * 2 {
                self.hold(2);
                self.levels ^= 2;
                self.hold(2)
            }
        } else {
            for _ in 0..clocks {
                self.hold(2);
                self.set(1, true);
                self.hold(2);
                self.set(1, false)
            }
            self.hold(2)
        }
    }
    fn finish(self) -> LogicBlock {
        self.builder.finish().expect("samples")
    }
}
fn harness(config: QspiFlashConfig, g: GraphConfig) -> Harness<FlashEvent> {
    Harness::build(8, TimeBase::new(1_000_000), g, |graph, source| {
        let node = graph.add(QspiFlash::new(config).expect("valid config"));
        graph.connect(source.samples, QspiFlash::input(node))?;
        Ok(QspiFlash::events(node))
    })
    .expect("graph")
}
fn events(items: &[Item<FlashEvent>]) -> Vec<Value> {
    items
        .iter()
        .filter_map(|i| {
            if let Item::Event(e) = i {
                Some(serde_json::to_value(e.payload).expect("valid test vector"))
            } else {
                None
            }
        })
        .collect()
}
fn has(events: &[Value], expected: Value) {
    assert!(
        events.iter().any(|actual| expected
            .as_object()
            .expect("valid test vector")
            .iter()
            .all(|(key, value)| actual.get(key) == Some(value))),
        "missing {expected}: {events:?}"
    )
}
fn config(qpi: bool) -> QspiFlashConfig {
    serde_json::from_value(json!({"clk":1,"cs":0,"io0":2,"io1":3,"io2":4,"io3":5,"address_bytes":3,"quad_dummy_clocks":4,"mode":0,"command_mode":if qpi{"qpi"}else{"spi"},"qpi_dummy_clocks":2,"dtr_quad_dummy_clocks":7})).expect("valid test vector")
}
#[test]
fn externally_specified_sdr_dual_quad_phases() {
    for (opcode, address_lanes, data_lanes, mode, dummy) in [
        (0x03, 1, 1, false, 0),
        (0x3b, 1, 2, false, 8),
        (0x6b, 1, 4, false, 8),
        (0xbb, 2, 2, true, 0),
        (0xeb, 4, 4, true, 4),
    ] {
        let mut w = Wire::new();
        w.select();
        w.sdr(opcode, 8, 1, false);
        w.sdr(0x123456, 24, address_lanes, false);
        if mode {
            w.sdr(0xff, 8, address_lanes, false)
        }
        w.dummy(dummy, false);
        w.sdr(0xa5, 8, data_lanes, true);
        w.release();
        let block = w.finish();
        let mut h = harness(
            config(false),
            GraphConfig {
                queue_capacity: 1,
                budget: 1,
            },
        );
        h.feed(&block, &[1, 7]);
        h.end();
        dsl_spi_support::logic_parity(
            "qspi-flash",
            serde_json::to_value(config(false)).expect("valid test vector"),
            GraphConfig {
                queue_capacity: 1,
                budget: 1,
            },
            &block,
            &[],
            &[1, 7],
            &h.items,
        );
        let out = events(&h.items);
        has(
            &out,
            json!({"kind":"address","value":0x123456,"lanes":address_lanes}),
        );
        has(&out, json!({"kind":"data","value":0xa5,"lanes":data_lanes}));
        has(&out, json!({"kind":"end","reason":"complete"}));
    }
}
#[test]
fn qpi_opcode_is_four_lanes_and_mode_is_included_in_dummy_count() {
    for opcode in [0x0b, 0xeb] {
        let mut w = Wire::new();
        w.select();
        w.sdr(opcode, 8, 4, false);
        w.sdr(0xabcdef, 24, 4, false);
        if opcode == 0xeb {
            w.sdr(0xff, 8, 4, false)
        } else {
            w.dummy(2, false)
        }
        w.sdr(0x5a, 8, 4, true);
        w.release();
        let block = w.finish();
        let mut h = harness(config(true), GraphConfig::default());
        h.feed(&block, &[1]);
        h.end();
        dsl_spi_support::logic_parity(
            "qspi-flash",
            serde_json::to_value(config(true)).expect("valid test vector"),
            GraphConfig::default(),
            &block,
            &[],
            &[1],
            &h.items,
        );
        let out = events(&h.items);
        has(&out, json!({"kind":"data","value":0x5a,"lanes":4}));
    }
}
#[test]
fn dtr_address_mode_dummy_and_data_sample_both_edges_after_sdr_opcode() {
    // Original Winbond rev B tables4/5 and figs17a,23a,27a. EDh has
    // 3 DDR address clocks, one mode clock and seven dummy clocks.
    for (opcode, lanes, mode, dummy) in
        [(0x0d, 1, false, 6), (0xbd, 2, true, 4), (0xed, 4, true, 7)]
    {
        let mut w = Wire::new();
        w.select();
        w.sdr(opcode, 8, 1, false);
        w.dtr(0x123456, 24, lanes, false);
        if mode {
            w.dtr(0xff, 8, lanes, false)
        }
        w.dummy(dummy, true);
        w.dtr(0xa5, 8, lanes, true);
        w.release();
        let block = w.finish();
        let mut h = harness(
            config(false),
            GraphConfig {
                queue_capacity: 1,
                budget: 1,
            },
        );
        h.feed(&block, &[1, 11]);
        h.end();
        dsl_spi_support::logic_parity(
            "qspi-flash",
            serde_json::to_value(config(false)).expect("valid test vector"),
            GraphConfig {
                queue_capacity: 1,
                budget: 1,
            },
            &block,
            &[],
            &[1, 11],
            &h.items,
        );
        let out = events(&h.items);
        has(&out, json!({"kind":"address","value":0x123456}));
        has(&out, json!({"kind":"data","value":0xa5}));
    }
}
#[test]
fn gap_never_joins_flash_address_halves_and_results_are_block_invariant() {
    let mut w = Wire::new();
    w.select();
    w.sdr(0xeb, 8, 1, false);
    w.sdr(0x123456, 24, 4, false);
    w.sdr(0xff, 8, 4, false);
    w.dummy(4, false);
    w.sdr(0xa5, 8, 4, true);
    w.release();
    let block = w.finish();
    uiharu_core::testing::assert_block_invariant(
        |g| harness(config(false), g),
        &block,
        &[TickRange::new(Tick(49), Tick(55))],
    );
}
#[test]
fn sitronix_windows_and_rgb565_pixels_use_observed_dc_and_colmod() {
    let mut w = Wire::new();
    w.select();
    for (command, parameters) in [
        (0x3a, vec![0x55]),
        (0x2a, vec![0, 1, 0, 2]),
        (0x2c, vec![0xf8, 0]),
    ] {
        w.set(3, false);
        w.hold(2);
        w.sdr(command, 8, 1, false);
        w.set(3, true);
        w.hold(2);
        for value in parameters {
            w.sdr(value, 8, 1, false)
        }
    }
    w.release();
    let block = w.finish();
    macro_rules! display{($module:ident,$node:ident)=>{{use uiharu_dsl_protocols::$module::{Config,Event,$node};let config:Config=serde_json::from_value(json!({"cs":0,"clk":1,"data":2,"dc":3})).expect("valid test vector");let mut h=Harness::<Event>::build(8,TimeBase::new(1_000_000),GraphConfig{queue_capacity:1,budget:1},|graph,source|{let node=graph.add($node::new(config).expect("valid test vector"));graph.connect(source.samples,$node::input(node))?;Ok($node::events(node))}).expect("valid test vector");h.feed(&block,&[1,7]);h.end();dsl_spi_support::logic_parity(&stringify!($module).replace('_',"-"),serde_json::to_value(config).expect("valid test vector"),GraphConfig{queue_capacity:1,budget:1},&block,&[],&[1,7],&h.items);let out=h.items.iter().filter_map(|i|if let Item::Event(e)=i{Some(serde_json::to_value(e.payload).expect("valid test vector"))}else{None}).collect::<Vec<_>>();has(&out,json!({"kind":"window","column":true,"first":1,"last":2}));has(&out,json!({"kind":"pixel","value":0xf800,"bits":16}));}}}
    display!(st7735, St7735);
    display!(st7789, St7789);
}
#[test]
fn flash_erase_status_and_sfdp_have_semantic_fields() {
    let mut w = Wire::new();
    w.select();
    w.sdr(0x20, 8, 1, false);
    w.sdr(0x123456, 24, 1, false);
    w.release();
    w.select();
    w.sdr(0x05, 8, 1, false);
    w.sdr(3, 8, 1, true);
    w.release();
    w.select();
    w.sdr(0x35, 8, 1, false);
    w.sdr(0x82, 8, 1, true);
    w.release();
    // Original Microchip SFDP table8-1 header and JEDEC FF00 parameter header,
    // with one parameter header at08 and basic table at30. Density DWORD2
    // is 03FFFFFF =>64Mbit, little endian.
    w.select();
    w.sdr(0x5a, 8, 1, false);
    w.sdr(0, 24, 1, false);
    w.dummy(8, false);
    for byte in [
        0x53, 0x46, 0x44, 0x50, 6, 1, 0, 0xff, 0, 6, 1, 16, 0x30, 0, 0, 0xff,
    ] {
        w.sdr(byte, 8, 1, true)
    }
    w.release();
    w.select();
    w.sdr(0x5a, 8, 1, false);
    w.sdr(0x34, 24, 1, false);
    w.dummy(8, false);
    for byte in [0xff, 0xff, 0xff, 3] {
        w.sdr(byte, 8, 1, true)
    }
    w.release();
    let block = w.finish();
    let g = GraphConfig {
        queue_capacity: 1,
        budget: 1,
    };
    let mut h = harness(config(false), g);
    h.feed(&block, &[1, 11]);
    h.end();
    let out = events(&h.items);
    has(
        &out,
        json!({"kind":"erase","address":0x123456,"bytes":4096}),
    );
    has(&out, json!({"kind":"status-bit","field":"busy","value":1}));
    has(
        &out,
        json!({"kind":"status-bit","field":"quad-enable","value":1}),
    );
    has(
        &out,
        json!({"kind":"sfdp-header","signature":0x50444653u32,"valid":true,"parameterHeaders":1}),
    );
    has(
        &out,
        json!({"kind":"sfdp-parameter","id":0xff00,"pointer":0x30,"dwords":16}),
    );
    has(&out, json!({"kind":"density","bits":67108864}));
    h.assert_progress_promise();
    dsl_spi_support::logic_parity(
        "qspi-flash",
        serde_json::to_value(config(false)).expect("valid test vector"),
        g,
        &block,
        &[],
        &[1, 11],
        &h.items,
    );
}
#[test]
fn incomplete_erase_and_sfdp_headers_do_not_invent_complete_fields() {
    let mut w = Wire::new();
    w.select();
    w.sdr(0x20, 8, 1, false);
    w.sdr(0x1234, 16, 1, false);
    w.release();
    w.select();
    w.sdr(0x5a, 8, 1, false);
    w.sdr(0, 24, 1, false);
    w.dummy(8, false);
    w.sdr(0x53, 8, 1, true);
    w.sdr(0x46, 8, 1, true);
    w.release();
    let block = w.finish();
    let mut h = harness(config(false), GraphConfig::default());
    h.feed(&block, &[1]);
    h.end();
    let out = events(&h.items);
    assert!(
        !out.iter()
            .any(|v| v["kind"] == "erase" || v["kind"] == "sfdp-header")
    );
    has(&out, json!({"kind":"end","reason":"incomplete"}));
    h.assert_progress_promise();
    uiharu_core::testing::assert_block_invariant(
        |g| harness(config(false), g),
        &block,
        &[TickRange::new(Tick(71), Tick(77))],
    );
}
#[test]
fn winbond_mode3_samples_sdr_and_dtr_phases_on_the_documented_edges() {
    fn sdr3(w: &mut Wire, value: u32, bits: u8, lanes: u8, read: bool) {
        for shift in (0..bits / lanes).rev() {
            w.set(1, false);
            w.group((value >> (shift * lanes)) as u8, lanes, read);
            w.hold(2);
            w.set(1, true);
            w.hold(2)
        }
    }
    for dtr in [false, true] {
        let mut w = Wire::new();
        w.set(1, true);
        w.hold(3);
        w.select();
        sdr3(&mut w, if dtr { 0xed } else { 0x0b }, 8, 1, false);
        if dtr {
            w.set(1, false);
            w.hold(2);
            w.dtr(0x123456, 24, 4, false);
            w.dtr(0xff, 8, 4, false);
            w.dummy(7, true);
            w.dtr(0xa5, 8, 4, true)
        } else {
            sdr3(&mut w, 0x123456, 24, 1, false);
            for _ in 0..8 {
                sdr3(&mut w, 0, 1, 1, false)
            }
            sdr3(&mut w, 0xa5, 8, 1, true)
        }
        w.set(1, true);
        w.release();
        let block = w.finish();
        let mut c = config(false);
        c.mode = 3;
        let g = GraphConfig {
            queue_capacity: 1,
            budget: 1,
        };
        let mut h = harness(c, g);
        h.feed(&block, &[1, 11]);
        h.end();
        let out = events(&h.items);
        has(&out, json!({"kind":"address","value":0x123456}));
        has(&out, json!({"kind":"data","value":0xa5}));
        has(&out, json!({"kind":"end","reason":"complete"}));
        dsl_spi_support::logic_parity(
            "qspi-flash",
            serde_json::to_value(c).expect("config"),
            g,
            &block,
            &[],
            &[1, 11],
            &h.items,
        );
    }
    for mode in [1, 2] {
        let mut c = config(false);
        c.mode = mode;
        assert!(QspiFlash::new(c).is_err());
    }
}

#[test]
fn winbond_observed_qe_qpi_entry_exit_and_address_modes_change_at_cs_release() {
    let mut w = Wire::new();
    // Readback establishes QE; a write alone is not accepted-status evidence.
    w.select();
    w.sdr(0x35, 8, 1, false);
    w.sdr(2, 8, 1, true);
    w.release();
    w.select();
    w.sdr(0x38, 8, 1, false);
    w.release();
    w.select();
    w.sdr(0x9f, 8, 4, false);
    w.sdr(0xef, 8, 4, true);
    w.release();
    w.select();
    w.sdr(0xff, 8, 4, false);
    w.release();
    w.select();
    w.sdr(0xb7, 8, 1, false);
    w.release();
    w.select();
    w.sdr(3, 8, 1, false);
    w.sdr(0x12345678, 32, 1, false);
    w.sdr(0x5a, 8, 1, true);
    w.release();
    w.select();
    w.sdr(0xe9, 8, 1, false);
    w.release();
    // Dedicated address opcodes retain32bits in3-byte mode.
    w.select();
    w.sdr(0x13, 8, 1, false);
    w.sdr(0x89abcdef, 32, 1, false);
    w.sdr(0xa5, 8, 1, true);
    w.release();
    // Armed reset restores the configured power-on address assumption and SPI.
    w.select();
    w.sdr(0xb7, 8, 1, false);
    w.release();
    w.select();
    w.sdr(0x66, 8, 1, false);
    w.release();
    w.select();
    w.sdr(0x99, 8, 1, false);
    w.release();
    w.select();
    w.sdr(3, 8, 1, false);
    w.sdr(0x345678, 24, 1, false);
    w.sdr(0x66, 8, 1, true);
    w.release();
    let block = w.finish();
    let c = config(false);
    let mut h = harness(c, GraphConfig::default());
    h.run_all(&block);
    let out = events(&h.items);
    has(
        &out,
        json!({"kind":"mode-request","from":"spi","to":"qpi","quadEnable":true,"followed":true}),
    );
    has(
        &out,
        json!({"kind":"mode-request","from":"qpi","to":"spi","followed":true}),
    );
    has(&out, json!({"kind":"data","value":0xef,"lanes":4}));
    has(&out, json!({"kind":"address","value":0x12345678,"bits":32}));
    has(
        &out,
        json!({"kind":"address","value":0x89abcdefu32,"bits":32}),
    );
    has(&out, json!({"kind":"address","value":0x345678,"bits":24}));
    has(&out, json!({"kind":"address-mode","bytes":3,"reset":true}));
    dsl_spi_support::logic_parity(
        "qspi-flash",
        json!(c),
        GraphConfig::default(),
        &block,
        &[],
        &[1, 3, 17],
        &h.items,
    );
    uiharu_core::testing::assert_block_invariant(|g| harness(c, g), &block, &[]);
}

#[test]
fn qpi_entry_with_unknown_qe_suspends_interpretation_and_qe_zero_keeps_spi() {
    for observed in [None, Some(0u8)] {
        let mut w = Wire::new();
        if let Some(qe) = observed {
            w.select();
            w.sdr(0x35, 8, 1, false);
            w.sdr(u32::from(qe), 8, 1, true);
            w.release();
        }
        w.select();
        w.sdr(0x38, 8, 1, false);
        w.release();
        w.select();
        w.sdr(6, 8, 1, false);
        w.release();
        let block = w.finish();
        let c = config(false);
        let mut h = harness(c, GraphConfig::default());
        h.run_all(&block);
        let out = events(&h.items);
        has(
            &out,
            json!({"kind":"mode-request","to":"qpi","followed":false,"quadEnable":observed.map(|_|false)}),
        );
        assert_eq!(
            out.iter()
                .any(|e| e["kind"] == "command" && e["opcode"] == 6),
            observed.is_some()
        );
        dsl_spi_support::logic_parity(
            "qspi-flash",
            json!(c),
            GraphConfig::default(),
            &block,
            &[],
            &[1, 7],
            &h.items,
        );
    }
}

#[test]
fn reset_arm_is_disarmed_by_intervening_unknown_or_incomplete_commands() {
    let mut w = Wire::new();
    for opcode in [0xb7, 0x66, 0x06, 0x99] {
        w.select();
        w.sdr(opcode, 8, 1, false);
        w.release();
    }
    w.select();
    w.sdr(3, 8, 1, false);
    w.sdr(0x10203040, 32, 1, false);
    w.sdr(0x55, 8, 1, true);
    w.release();
    // Trailing clocks invalidate a mode-setting command's exact boundary.
    w.select();
    w.sdr(0xe9, 8, 1, false);
    w.sdr(0, 8, 1, false);
    w.release();
    w.select();
    w.sdr(3, 8, 1, false);
    w.sdr(0x50607080, 32, 1, false);
    w.sdr(0xaa, 8, 1, true);
    w.release();
    let block = w.finish();
    let c = config(false);
    let mut h = harness(c, GraphConfig::default());
    h.run_all(&block);
    let out = events(&h.items);
    has(&out, json!({"kind":"address","value":0x10203040,"bits":32}));
    has(&out, json!({"kind":"address","value":0x50607080,"bits":32}));
    assert!(
        !out.iter()
            .any(|e| e["kind"] == "address-mode" && e["reset"] == true)
    );
    has(&out, json!({"kind":"end","reason":"unsupported"}));
    dsl_spi_support::logic_parity(
        "qspi-flash",
        json!(c),
        GraphConfig::default(),
        &block,
        &[],
        &[1, 5],
        &h.items,
    );
    uiharu_core::testing::assert_block_invariant(|g| harness(c, g), &block, &[]);
}

#[test]
fn dedicated_quad_address_id_unique_and_security_layouts_keep_lane_phases() {
    let mut w = Wire::new();
    w.select();
    w.sdr(0xec, 8, 1, false);
    w.sdr(0x12345678, 32, 4, false);
    w.sdr(0xff, 8, 4, false);
    w.dummy(4, false);
    w.sdr(0xab, 8, 4, true);
    w.release();
    w.select();
    w.sdr(0x92, 8, 1, false);
    w.sdr(0, 24, 2, false);
    w.sdr(0xff, 8, 2, false);
    w.sdr(0xef, 8, 2, true);
    w.release();
    w.select();
    w.sdr(0x4b, 8, 1, false);
    w.dummy(32, false);
    w.sdr(0x77, 8, 1, true);
    w.release();
    w.select();
    w.sdr(0x48, 8, 1, false);
    w.sdr(0x001000, 24, 1, false);
    w.dummy(8, false);
    w.sdr(0x88, 8, 1, true);
    w.release();
    let block = w.finish();
    let c = config(false);
    let mut h = harness(c, GraphConfig::default());
    h.run_all(&block);
    let out = events(&h.items);
    has(
        &out,
        json!({"kind":"address","value":0x12345678,"bits":32,"lanes":4}),
    );
    for value in [0xab, 0xef, 0x77, 0x88] {
        has(&out, json!({"kind":"data","value":value}));
    }
    dsl_spi_support::logic_parity(
        "qspi-flash",
        json!(c),
        GraphConfig::default(),
        &block,
        &[],
        &[1, 5, 11],
        &h.items,
    );
    uiharu_core::testing::assert_block_invariant(|g| harness(c, g), &block, &[]);
}
