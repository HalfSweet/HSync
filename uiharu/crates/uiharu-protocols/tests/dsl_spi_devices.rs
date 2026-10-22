// SPDX-License-Identifier: Apache-2.0
//! Manufacturer-derived wire vectors for the DSL SPI device profiles.
mod dsl_spi_support;
use serde_json::{Value, json};
use uiharu_core::testing::Harness;
use uiharu_core::{GraphConfig, Item, LogicBlock, Tick, TimeBase};
use uiharu_core::{InputRef, Node, OutputRef};
use uiharu_interp::{DynEvent, Interpreter, ToDyn};
use uiharu_protocols::spi::SpiEvent;
use uiharu_protocols::spi::synth::SpiWaveform;
use uiharu_protocols::spi::{Spi, SpiConfig};

fn wave(bits: u8, frames: &[Vec<(u8, u8)>]) -> LogicBlock {
    let config = SpiConfig {
        word_bits: bits,
        ..Default::default()
    };
    let mut w = SpiWaveform::new(config, 8, 2, Tick::ZERO);
    w.idle(5);
    for frame in frames {
        w.transaction(frame.iter().copied());
        w.idle(5);
    }
    w.finish()
}
fn bit_frame(mosi: u32, miso: u32, bits: u8) -> Vec<(u8, u8)> {
    (0..bits)
        .rev()
        .map(|bit| (((mosi >> bit) & 1) as u8, ((miso >> bit) & 1) as u8))
        .collect()
}
fn contains(events: &[Value], expected: Value) {
    assert!(
        events.iter().any(|actual| expected
            .as_object()
            .expect("valid test vector")
            .iter()
            .all(|(key, value)| actual.get(key) == Some(value))),
        "missing {expected} in {events:?}"
    );
}
macro_rules! decode {
    ($module:ident,$node:ident,$event:ident,$config:expr,$bits:expr,$block:expr,$sizes:expr,$g:expr) => {{
        use uiharu_dsl_protocols::$module::{Config, $event, $node};
        let config: Config = serde_json::from_value($config).expect("valid config");
        let mut h = Harness::<$event>::build(8, TimeBase::new(1_000_000), $g, |graph, source| {
            let spi = graph.add(
                Spi::new(SpiConfig {
                    word_bits: $bits,
                    ..Default::default()
                })
                .expect("SPI"),
            );
            let device = graph.add($node::new(config).expect("device"));
            graph.connect(source.samples, Spi::input(spi))?;
            graph.connect(Spi::events(spi), $node::input(device))?;
            Ok($node::events(device))
        })
        .expect("graph");
        h.feed($block, $sizes);
        h.end();
        let mut interpreted =
            Harness::<DynEvent>::build(8, TimeBase::new(1_000_000), $g, |graph, source| {
                let spi_node = Spi::new(SpiConfig {
                    word_bits: $bits,
                    ..Default::default()
                })
                .expect("valid test vector");
                let props = spi_node.spec().outputs[0].props.clone();
                let spi = graph.add(spi_node);
                let bridge = graph.add(ToDyn::<SpiEvent>::new(props));
                let module = stringify!($module).replace('_', "-");
                let device = graph.add(
                    Interpreter::new(
                        dsl_spi_support::program(&module),
                        &serde_json::to_value(config).expect("valid test vector"),
                    )
                    .expect("interpreted config"),
                );
                graph.connect(source.samples, Spi::input(spi))?;
                graph.connect(Spi::events(spi), InputRef::<SpiEvent>::of_node(bridge, 0))?;
                graph.connect(
                    OutputRef::<DynEvent>::of_node(bridge, 0),
                    Interpreter::bus_input(device),
                )?;
                Ok(Interpreter::events(device))
            })
            .expect("interpreted graph");
        interpreted.feed($block, $sizes);
        interpreted.end();
        dsl_spi_support::assert_items(&h.items, &interpreted.items, stringify!($module));
        h.items
            .iter()
            .filter_map(|item| match item {
                Item::Event(event) => {
                    Some(serde_json::to_value(event.payload).expect("valid test vector"))
                }
                _ => None,
            })
            .collect::<Vec<_>>()
    }};
}
#[test]
fn dac_and_adc_use_documented_voltage_scales_and_padding() {
    // AD5626: nominal 1 mV/LSB; AD7920: 4 leading zeros, 12-bit sample.
    let block = wave(1, &[bit_frame(0xabc, 0, 12)]);
    let events = decode!(
        ad5626,
        Ad5626,
        DeviceEvent,
        json!({}),
        1,
        &block,
        &[1, 7, 2],
        GraphConfig {
            queue_capacity: 1,
            budget: 1
        }
    );
    contains(
        &events,
        json!({"kind":"dac","code":2748,"microvolts":2748000}),
    );
    let block = wave(1, &[bit_frame(0, 0x0800, 16), bit_frame(0, 0xf800, 16)]);
    let events = decode!(
        ad79x0,
        Ad79x0,
        DeviceEvent,
        json!({"ten_bit":false,"reference_microvolts":2500000}),
        1,
        &block,
        &[3],
        GraphConfig::default()
    );
    contains(
        &events,
        json!({"kind":"sample","code":2048,"microvolts":1250000,"valid":true}),
    );
    contains(&events, json!({"kind":"sample","valid":false}));
    let short = wave(1, &[bit_frame(0, 0, 5)]);
    let events = decode!(
        ad79x0,
        Ad79x0,
        DeviceEvent,
        json!({"ten_bit":true,"reference_microvolts":3300000}),
        1,
        &short,
        &[u64::MAX],
        GraphConfig::default()
    );
    contains(
        &events,
        json!({"kind":"mode","mode":"power-down-request","clocks":5}),
    );
    assert!(!events.iter().any(|v| v["kind"] == "sample"));
}
#[test]
fn adf4351_fields_are_not_only_register_numbers() {
    // Register 0: INT=120, FRAC=17, address=0; datasheet register map.
    let block = wave(1, &[bit_frame((120 << 15) | (17 << 3), 0, 32)]);
    let events = decode!(
        adf435x,
        Adf435x,
        DeviceEvent,
        json!({}),
        1,
        &block,
        &[1, 13],
        GraphConfig::default()
    );
    contains(
        &events,
        json!({"kind":"divider","integer":120,"fraction":17}),
    );
}
#[test]
fn max7219_named_controls_digits_and_warnings() {
    let block = wave(
        8,
        &[vec![(1, 0), (0x81, 0), (10, 0), (7, 0), (13, 0), (0, 0)]],
    );
    let events = decode!(
        max7219,
        Max7219,
        Event,
        json!({}),
        8,
        &block,
        &[2],
        GraphConfig::default()
    );
    contains(
        &events,
        json!({"kind":"digit","digit":0,"segments":1,"decimalPoint":true}),
    );
    contains(&events, json!({"kind":"intensity","level":7}));
    contains(&events, json!({"kind":"warning","address":13}));
}
#[test]
fn adxl345_little_endian_signed_axes_and_controls() {
    // DATAX0..DATAZ1 read, multibyte bit set; -1, +256, -32768.
    let block = wave(
        8,
        &[
            vec![
                (0xf2, 0),
                (0, 0xff),
                (0, 0xff),
                (0, 0),
                (0, 1),
                (0, 0),
                (0, 0x80),
            ],
            vec![(0x31, 0), (0x0b, 0)],
        ],
    );
    let events = decode!(
        adxl345_spi,
        Adxl345Spi,
        Event,
        json!({}),
        8,
        &block,
        &[1, 5, 17],
        GraphConfig {
            queue_capacity: 1,
            budget: 1
        }
    );
    contains(
        &events,
        json!({"kind":"field","field":"x-raw","negative":true,"value":1}),
    );
    contains(
        &events,
        json!({"kind":"field","field":"y-raw","negative":false,"value":256}),
    );
    contains(
        &events,
        json!({"kind":"field","field":"z-raw","negative":true,"value":32768}),
    );
    contains(
        &events,
        json!({"kind":"field","field":"full-resolution","value":1}),
    );
}
#[test]
fn ade7753_uses_register_width_and_signed_code() {
    let block = wave(
        8,
        &[vec![
            (1, 0),
            (0, 0xff),
            (0, 0xff),
            (0, 0xfe),
            (0x89, 0),
            (0x80, 0),
            (0x40, 0),
        ]],
    );
    let events = decode!(
        ade77xx,
        Ade77xx,
        Event,
        json!({"model":"ade7753","framed":true}),
        8,
        &block,
        &[1],
        GraphConfig {
            queue_capacity: 1,
            budget: 1
        }
    );
    contains(
        &events,
        json!({"kind":"register","register":"waveform","bits":24,"negative":true,"magnitude":2}),
    );
    contains(
        &events,
        json!({"kind":"mode","positiveOnly":true,"reset":true}),
    );
}
#[test]
fn st25_spaces_fifo_status_and_direct_commands_are_distinct() {
    let block = wave(
        8,
        &[
            vec![(0xfb, 0), (0x41, 0), (0, 0xaa)],
            vec![(0x5f, 0), (0, 0xb0)],
            vec![(0xdb, 0)],
        ],
    );
    let events = decode!(
        st25r39xx_spi,
        St25r39xxSpi,
        Event,
        json!({}),
        8,
        &block,
        &[3],
        GraphConfig::default()
    );
    contains(&events, json!({"kind":"space","space":"b"}));
    contains(
        &events,
        json!({"kind":"field","field":"fifo-underflow","value":1}),
    );
    contains(
        &events,
        json!({"kind":"field","field":"fifo-overflow","value":1}),
    );
    contains(&events, json!({"kind":"direct","command":"clear-fifo"}));
    assert!(
        !events
            .iter()
            .any(|v| v["kind"] == "field" && v["address"] == 1)
    );
}
#[test]
fn rgb_triplets_support_all_orders_and_reject_partial_pixel() {
    for (order, expected) in [
        ("rgb", [1, 2, 3]),
        ("rbg", [1, 3, 2]),
        ("grb", [2, 1, 3]),
        ("gbr", [3, 1, 2]),
        ("brg", [2, 3, 1]),
        ("bgr", [3, 2, 1]),
    ] {
        let block = wave(8, &[vec![(1, 0), (2, 0), (3, 0), (9, 0)]]);
        let events = decode!(
            rgb_led_spi,
            RgbLedSpi,
            Event,
            json!({"order":order,"framed":true}),
            8,
            &block,
            &[1],
            GraphConfig::default()
        );
        contains(
            &events,
            json!({"kind":"pixel","red":expected[0],"green":expected[1],"blue":expected[2]}),
        );
        assert_eq!(events.iter().filter(|v| v["kind"] == "pixel").count(), 1);
        contains(&events, json!({"kind":"end","reason":"incomplete"}));
    }
}
#[test]
fn same_adxl_samples_with_block_queue_and_budget_changes() {
    let block = wave(
        8,
        &[vec![(0xf2, 0), (0, 0x34), (0, 0x12), (0, 0xff), (0, 0xff)]],
    );
    let baseline = decode!(
        adxl345_spi,
        Adxl345Spi,
        Event,
        json!({}),
        8,
        &block,
        &[u64::MAX],
        GraphConfig::default()
    );
    for sizes in [&[1][..], &[13, 1, 7][..]] {
        let actual = decode!(
            adxl345_spi,
            Adxl345Spi,
            Event,
            json!({}),
            8,
            &block,
            sizes,
            GraphConfig {
                queue_capacity: 1,
                budget: 1
            }
        );
        assert_eq!(baseline, actual);
    }
}
#[test]
fn ltc2422_identifies_channel_and_converts_signed_microvolts() {
    // EOC=0, channel1, SIG=1, half-scale code80000; negative quarter-scale1C0000.
    let block = wave(1, &[bit_frame(0, 0x680000, 24), bit_frame(0, 0x1c0000, 24)]);
    let events = decode!(
        ltc242x,
        Ltc242x,
        DeviceEvent,
        json!({"model":"ltc2422","reference_microvolts":2500000}),
        1,
        &block,
        &[1, 9],
        GraphConfig::default()
    );
    contains(
        &events,
        json!({"kind":"sample","channel":1,"microvolts":1250000,"negative":false,"valid":true}),
    );
    contains(
        &events,
        json!({"kind":"sample","channel":0,"microvolts":625000,"negative":true,"valid":true}),
    );
}
#[test]
fn radio_commands_and_control_subfields_use_chip_layouts() {
    let block = wave(
        8,
        &[
            vec![(0x34, 0)],
            vec![(8, 0), (0x45, 0)],
            vec![(0xf3, 0), (0, 0x85)],
        ],
    );
    let events = decode!(
        cc1101,
        Cc1101,
        Event,
        json!({}),
        8,
        &block,
        &[1],
        GraphConfig::default()
    );
    contains(&events, json!({"kind":"operation","operation":"receive"}));
    contains(
        &events,
        json!({"kind":"field","field":"whitening","value":1}),
    );
    contains(
        &events,
        json!({"kind":"field","field":"crc-enabled","value":1}),
    );
    contains(&events, json!({"kind":"field","field":"crc-ok","value":1}));
    let block = wave(8, &[vec![(1, 0), (0x3f, 0)], vec![(9, 0), (0xdf, 0)]]);
    let events = decode!(
        nrf905,
        Nrf905,
        Event,
        json!({}),
        8,
        &block,
        &[1],
        GraphConfig::default()
    );
    contains(
        &events,
        json!({"kind":"field","field":"auto-retransmit","value":1}),
    );
    contains(&events, json!({"kind":"field","field":"crc-16","value":1}));
    contains(
        &events,
        json!({"kind":"field","field":"crystal-code","value":3}),
    );
    let block = wave(
        8,
        &[vec![(0x82, 0xc0), (0x80, 0)], vec![(0xc6, 0), (0x23, 0)]],
    );
    let events = decode!(
        rfm12,
        Rfm12,
        Event,
        json!({}),
        8,
        &block,
        &[1],
        GraphConfig::default()
    );
    contains(
        &events,
        json!({"kind":"command","command":"power-management"}),
    );
    contains(
        &events,
        json!({"kind":"field","field":"receiver-enabled","value":1}),
    );
    contains(
        &events,
        json!({"kind":"field","field":"power-on-reset","value":1}),
    );
    contains(
        &events,
        json!({"kind":"field","field":"data-rate-bps","value":9578}),
    );
    let block = wave(8, &[vec![(0xc0, 0)], vec![(0x0f, 0), (0x22, 0)]]);
    let events = decode!(
        a7105,
        A7105,
        Event,
        json!({}),
        8,
        &block,
        &[2],
        GraphConfig::default()
    );
    contains(&events, json!({"kind":"operation","operation":"receive"}));
    contains(
        &events,
        json!({"kind":"field","field":"channel","value":0x22}),
    );
}
#[test]
fn mrf24_fifo_frame_and_retry_failure_flags() {
    // Long writes: length at1, bytes at2/3. Short TXSTAT24h => header48h.
    let block = wave(
        8,
        &[
            vec![(0x80, 0), (0x30, 0), (2, 0)],
            vec![(0x80, 0), (0x50, 0), (0xaa, 0)],
            vec![(0x80, 0), (0x70, 0), (0xbb, 0)],
            vec![(0x48, 0), (0, 0xe1)],
        ],
    );
    let events = decode!(
        mrf24j40,
        Mrf24j40,
        Event,
        json!({}),
        8,
        &block,
        &[1, 5],
        GraphConfig::default()
    );
    contains(
        &events,
        json!({"kind":"frame-begin","receive":false,"length":2}),
    );
    contains(
        &events,
        json!({"kind":"frame-byte","offset":1,"value":0xbb}),
    );
    contains(&events, json!({"kind":"frame-end","reason":"complete"}));
    contains(
        &events,
        json!({"kind":"field","field":"retry-count","value":3}),
    );
    contains(
        &events,
        json!({"kind":"field","field":"cca-failed","value":1}),
    );
    contains(
        &events,
        json!({"kind":"field","field":"tx-failed","value":1}),
    );
}
#[test]
fn enc28_common_register_bits_and_mac_dummy_byte() {
    let block = wave(
        8,
        &[
            vec![(0x1f, 0), (0, 0x2f)],
            vec![(0, 0), (0, 0xaa), (0, 0xbb)],
        ],
    );
    let events = decode!(
        enc28j60,
        Enc28j60,
        Event,
        json!({"dummy_mode":"ethernet"}),
        8,
        &block,
        &[1],
        GraphConfig::default()
    );
    contains(
        &events,
        json!({"kind":"field","field":"receive-enabled","value":1}),
    );
    contains(
        &events,
        json!({"kind":"field","field":"bank-select","value":3}),
    );
    let block = wave(8, &[vec![(0, 0), (0, 0xaa), (0, 0xbb)]]);
    let events = decode!(
        enc28j60,
        Enc28j60,
        Event,
        json!({"dummy_mode":"mac-mii"}),
        8,
        &block,
        &[1],
        GraphConfig::default()
    );
    contains(&events, json!({"kind":"data","address":0,"value":0xbb}));
}
#[test]
fn tpm_wait_bytes_are_not_payload_and_length_is_enforced() {
    let block = wave(
        8,
        &[vec![
            (0x81, 0),
            (0xd4, 0),
            (0, 0),
            (0x18, 0),
            (0, 0),
            (0, 1),
            (0, 0x12),
            (0, 0x34),
        ]],
    );
    let events = decode!(
        spi_tpm,
        SpiTpm,
        Event,
        json!({"tpm_version":"tpm20"}),
        8,
        &block,
        &[1],
        GraphConfig {
            queue_capacity: 1,
            budget: 1
        }
    );
    contains(
        &events,
        json!({"kind":"begin","read":true,"address":0xd40018,"length":2}),
    );
    contains(&events, json!({"kind":"wait","bytes":2}));
    contains(&events, json!({"kind":"data","offset":0,"value":0x12}));
    contains(&events, json!({"kind":"data","offset":1,"value":0x34}));
    contains(&events, json!({"kind":"end","reason":"complete"}));
}
#[test]
fn short_and_long_dac_frames_do_not_publish_semantic_codes() {
    let block = wave(1, &[bit_frame(0xaaa, 0, 11), bit_frame(0xaaa, 0, 13)]);
    let events = decode!(
        ad5626,
        Ad5626,
        DeviceEvent,
        json!({}),
        1,
        &block,
        &[1],
        GraphConfig::default()
    );
    assert!(!events.iter().any(|v| v["kind"] == "dac"));
    contains(&events, json!({"kind":"end","reason":"invalid-length"}));
}
#[test]
fn remaining_fixed_width_and_mouse_profiles_have_semantic_vectors() {
    let block = wave(8, &[vec![(12, 0), (0, 0), (15, 0), (1, 0)]]);
    let events = decode!(
        max72xx,
        Max72xx,
        Event,
        json!({}),
        8,
        &block,
        &[1],
        GraphConfig::default()
    );
    contains(&events, json!({"kind":"shutdown","enabled":true}));
    contains(&events, json!({"kind":"display-test","enabled":true}));
    let block = wave(8, &[vec![(2, 0), (0x90, 0)], vec![(3, 0), (0xfe, 0)]]);
    let events = decode!(
        adns5020,
        Adns5020,
        Event,
        json!({"shared_data":true}),
        8,
        &block,
        &[1],
        GraphConfig::default()
    );
    contains(&events, json!({"kind":"field","field":"motion","value":1}));
    contains(
        &events,
        json!({"kind":"field","field":"overflow","value":1}),
    );
    contains(
        &events,
        json!({"kind":"field","field":"delta-x-raw","value":254}),
    );
    let block = wave(
        1,
        &[
            bit_frame((0xab << 16) | 0x1234, 0, 24),
            bit_frame(0xae << 16, 0x5678, 24),
        ],
    );
    let events = decode!(
        x2444m,
        X2444m,
        Event,
        json!({}),
        1,
        &block,
        &[1],
        GraphConfig::default()
    );
    contains(&events, json!({"kind":"begin","opcode":3,"address":5}));
    contains(&events, json!({"kind":"word","read":false,"value":0x1234}));
    contains(&events, json!({"kind":"word","read":true,"value":0x5678}));
}
#[test]
fn sd_spi_ocr_and_csd_capacity_and_crc_fault() {
    let block = wave(
        8,
        &[vec![
            (0x7a, 0xff),
            (0, 0xff),
            (0, 0xff),
            (0, 0xff),
            (0, 0xff),
            (1, 0xff),
            (0xff, 0),
            (0xff, 0xc1),
            (0xff, 0xff),
            (0xff, 0x80),
            (0xff, 0),
        ]],
    );
    let events = decode!(
        sd_registers,
        SdRegisters,
        Event,
        json!({}),
        8,
        &block,
        &[1],
        GraphConfig::default()
    );
    contains(
        &events,
        json!({"kind":"ocr","ready":true,"highCapacity":true,"switch18":true}),
    );
    // CSD structure1 and C_SIZE=0 =>1024 x512-byte blocks; deliberately incorrect CRC.
    let mut frame = vec![
        (0x49, 0xff),
        (0, 0xff),
        (0, 0xff),
        (0, 0xff),
        (0, 0xff),
        (1, 0xff),
        (0xff, 0),
        (0xff, 0xfe),
    ];
    let mut csd = [0u8; 16];
    csd[0] = 0x40;
    csd[5] = 9;
    frame.extend(csd.into_iter().map(|v| (0xff, v)));
    frame.extend([(0xff, 0), (0xff, 0)]);
    let block = wave(8, &[frame]);
    let events = decode!(
        sd_registers,
        SdRegisters,
        Event,
        json!({}),
        8,
        &block,
        &[1, 9],
        GraphConfig::default()
    );
    contains(
        &events,
        json!({"kind":"csd","structure":1,"blocks":1024,"readBlockBits":9,"crcOk":false,"supported":true}),
    );
}
#[test]
fn xn297_calibration_registers_and_64_byte_payload_are_not_nrf24_limits() {
    let mut payload = vec![(0xa0, 0x2e)];
    payload.extend((0..64).map(|n| (n, 0)));
    let block = wave(
        8,
        &[
            vec![
                (0x3e, 0x4f),
                (0x95, 0),
                (0x2b, 0),
                (0x83, 0),
                (0x61, 0),
                (0xb0, 0),
                (0x9a, 0),
                (0xca, 0),
            ],
            payload,
        ],
    );
    let events = decode!(
        xn297,
        Xn297,
        Event,
        json!({"model":"original"}),
        8,
        &block,
        &[1, 9],
        GraphConfig {
            queue_capacity: 1,
            budget: 1
        }
    );
    contains(
        &events,
        json!({"kind":"command","register":"rf-cal","address":30}),
    );
    contains(&events, json!({"kind":"data","offset":6,"value":0xca}));
    contains(&events, json!({"kind":"data","offset":63,"value":63}));
    contains(
        &events,
        json!({"kind":"field","field":"rx-ready","value":1}),
    );
    assert!(!events.iter().any(|v| v["reason"] == "too-long"));
    let mut payload = vec![(0xa0, 0)];
    payload.extend((0..65).map(|n| (n, 0)));
    let block = wave(8, &[payload, vec![(0x50, 0), (0x72, 0)]]);
    let events = decode!(
        xn297,
        Xn297,
        Event,
        json!({"model":"original"}),
        8,
        &block,
        &[3],
        GraphConfig::default()
    );
    contains(&events, json!({"kind":"end","reason":"too-long"}));
    contains(&events, json!({"kind":"end","reason":"invalid-key"}));
}

#[test]
fn enc28_observed_banks_select_register_names_and_read_dummy_cycles() {
    let block = wave(
        8,
        &[
            vec![(0x00, 0), (0, 0x11)], // unknown bank: refuse ambiguous read alignment
            vec![(0xff, 0)],            // reset establishes bank zero
            vec![(0x40, 0), (0x34, 0)], // ERDPTL
            vec![(0x9f, 0), (2, 0)],    // BFS ECON1 bank two
            vec![(0x00, 0), (0, 0xee), (0, 0x0d)], // MACON1 dummy then data
            vec![(0xbf, 0), (2, 0)],    // BFC bank back zero
            vec![(0x00, 0), (0, 0x56)], // ERDPTL has no dummy
            vec![(0x5f, 0), (3, 0)],    // WCR ECON1 bank three
            vec![(0x00, 0), (0, 0xee), (0, 0xab)], // MAADR5
        ],
    );
    let events = decode!(
        enc28j60,
        Enc28j60,
        Event,
        json!({"dummy_mode":"auto"}),
        8,
        &block,
        &[1, 17],
        GraphConfig {
            queue_capacity: 1,
            budget: 1
        }
    );
    contains(&events, json!({"kind":"end","reason":"unknown-bank"}));
    contains(&events, json!({"kind":"register","bank":0,"name":"erdptl"}));
    contains(&events, json!({"kind":"register","bank":2,"name":"macon1"}));
    contains(&events, json!({"kind":"register","bank":3,"name":"maadr5"}));
    for value in [0x34, 0x0d, 0x56, 0xab] {
        contains(&events, json!({"kind":"data","value":value}));
    }
    assert!(
        !events
            .iter()
            .any(|event| event["kind"] == "data" && event["value"] == 0xee)
    );
}

#[test]
fn xn297l_primary_calibration_widths_are_explicitly_distinct() {
    let mut frames = Vec::new();
    for (address, width) in [(0x19, 1), (0x1a, 6), (0x1b, 3), (0x1e, 3), (0x1f, 1)] {
        let mut frame = vec![(0x20 | address, 0)];
        frame.extend((0..width).map(|i| (i + 1, 0)));
        frames.push(frame);
    }
    let block = wave(8, &frames);
    let events = decode!(
        xn297,
        Xn297,
        Event,
        json!({"model":"xn297l"}),
        8,
        &block,
        &[1, 19],
        GraphConfig {
            queue_capacity: 1,
            budget: 1
        }
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| event["kind"] == "end" && event["reason"] == "complete")
            .count(),
        5
    );
    contains(&events, json!({"kind":"command","register":"rf-cal2"}));
    contains(&events, json!({"kind":"command","register":"dem-cal2"}));
    let block = wave(8, &[vec![(0x3f, 0), (1, 0), (2, 0)]]);
    let events = decode!(
        xn297,
        Xn297,
        Event,
        json!({"model":"xn297l"}),
        8,
        &block,
        &[1],
        GraphConfig::default()
    );
    contains(&events, json!({"kind":"end","reason":"too-long"}));
}
#[test]
fn adf4351_all_six_registers_expose_documented_control_fields() {
    let words = [
        0x00300000, 0x18008011, 0x00004e42, 0x00e4802b, 0x00800024, 0x00580005,
    ];
    let frames: Vec<_> = words
        .into_iter()
        .map(|word| bit_frame(word, 0, 32))
        .collect();
    let block = wave(1, &frames);
    let events = decode!(
        adf435x,
        Adf435x,
        DeviceEvent,
        json!({}),
        1,
        &block,
        &[1, 13],
        GraphConfig {
            queue_capacity: 1,
            budget: 1
        }
    );
    for (address, field, value) in [
        (0, "integer", 96),
        (1, "phase-adjust", 1),
        (1, "prescaler-89", 1),
        (1, "phase", 1),
        (1, "modulus", 2),
        (2, "reference-counter", 1),
        (2, "charge-pump-current", 7),
        (2, "detector-positive", 1),
        (3, "band-select-high", 1),
        (3, "antibacklash-3ns", 1),
        (3, "charge-cancel", 1),
        (3, "cycle-slip-reduction", 1),
        (3, "clock-divider-mode", 1),
        (3, "clock-divider", 5),
        (4, "feedback-fundamental", 1),
        (4, "rf-enabled", 1),
        (5, "lock-detect-pin-mode", 1),
    ] {
        contains(
            &events,
            json!({"kind":"field","address":address,"field":field,"value":value}),
        );
    }
}
#[test]
fn adxl345_threshold_timing_and_observed_format_scale_have_engineering_values() {
    let block = wave(
        8,
        &[
            vec![
                (0x5d, 0),
                (2, 0),
                (0xff, 0),
                (1, 0),
                (0, 0),
                (4, 0),
                (3, 0),
                (5, 0),
                (6, 0),
                (7, 0),
                (8, 0),
                (0xa5, 0),
                (9, 0),
                (10, 0),
                (0x0d, 0),
                (0x5b, 0),
            ],
            vec![(0x31, 0), (0x0b, 0)], // full resolution, +/-16g, right justified
            vec![(0xf2, 0), (0, 0), (0, 1), (0, 0), (0, 0xff)], // +256 X, -256 Y
            vec![(0x31, 0), (0x07, 0)], // fixed10bit +/-16g, left justified
            vec![(0xf2, 0), (0, 0), (0, 0x10)], // +64 counts =2g
        ],
    );
    let events = decode!(
        adxl345_spi,
        Adxl345Spi,
        Event,
        json!({}),
        8,
        &block,
        &[1, 23],
        GraphConfig {
            queue_capacity: 1,
            budget: 1
        }
    );
    for (field, value, negative) in [
        ("tap-threshold-microg", 125000, false),
        ("offset-x-microg", 15600, true),
        ("tap-duration-micros", 2500, false),
        ("tap-latency-micros", 3750, false),
        ("tap-window-micros", 6250, false),
        ("activity-threshold-microg", 375000, false),
        ("inactivity-threshold-microg", 437500, false),
        ("inactivity-time-seconds", 8, false),
        ("freefall-threshold-microg", 562500, false),
        ("freefall-time-millis", 50, false),
        ("x-microg", 1000000, false),
        ("y-microg", 1000000, true),
        ("x-microg", 2000000, false),
    ] {
        contains(
            &events,
            json!({"kind":"field","field":field,"value":value,"negative":negative}),
        );
    }
}

#[test]
fn rfm12_all_seventeen_commands_have_parameters_and_returned_status() {
    let words = [
        0x80d7u16, 0x82d9, 0xa640, 0xc623, 0x94a3, 0xc2ac, 0xca83, 0xced4, 0xb000, 0xc4f7, 0x9850,
        0xcc77, 0xb8aa, 0xe196, 0xc80e, 0xc005, 0,
    ];
    let frames: Vec<_> = words
        .into_iter()
        .map(|value| vec![((value >> 8) as u8, 0xa5), (value as u8, 0x5a)])
        .collect();
    let block = wave(8, &frames);
    let events = decode!(
        rfm12,
        Rfm12,
        Event,
        json!({}),
        8,
        &block,
        &[1, 31],
        GraphConfig {
            queue_capacity: 1,
            budget: 1
        }
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| event["kind"] == "command")
            .count(),
        17
    );
    for (field, value) in [
        ("band-code", 1),
        ("crystal-load-femtofarads", 12000),
        ("frequency-hz", 434000000),
        ("data-rate-bps", 9578),
        ("pin16-vdi", 1),
        ("clock-auto-lock", 1),
        ("data-quality-threshold", 4),
        ("fifo-interrupt-bits", 8),
        ("fifo-fill-enabled", 1),
        ("afc-mode", 3),
        ("afc-range-limit", 3),
        ("tx-deviation-hz", 90000),
        ("clock-buffer-code", 3),
        ("dither-disabled", 1),
        ("wake-mantissa", 150),
        ("duty-cycle-basis-points", 1000),
        ("battery-threshold-microvolts", 2750000),
        ("fifo-byte", 0x5a),
        ("sync-byte", 0xd4),
    ] {
        contains(&events, json!({"kind":"field","field":field,"value":value}));
    }
    contains(&events, json!({"kind":"wake-period","microseconds":309500}));
}
#[test]
fn sd_scr_requires_accepted_application_prefix_and_validates_its_eight_bytes() {
    let prefix = vec![
        (0x77, 0xff),
        (0, 0xff),
        (0, 0xff),
        (0, 0xff),
        (0, 0xff),
        (1, 0xff),
        (0xff, 0),
    ];
    for (app, bad_crc, truncated) in [
        (true, false, false),
        (false, false, false),
        (true, true, false),
        (true, false, true),
    ] {
        let mut frame = vec![
            (0x73, 0xff),
            (0, 0xff),
            (0, 0xff),
            (0, 0xff),
            (0, 0xff),
            (1, 0xff),
            (0xff, 0),
            (0xff, 0xfe),
        ];
        frame.extend(
            [0x02, 0xb5, 0x84, 0x03, 0x12, 0x34, 0x56, 0x78]
                .into_iter()
                .map(|value| (0xff, value)),
        );
        frame.push((0xff, 0x8c));
        frame.push((0xff, if bad_crc { 0x0b } else { 0x0a }));
        if truncated {
            frame.truncate(12)
        }
        let mut frames = Vec::new();
        if app {
            frames.push(prefix.clone())
        }
        frames.push(frame);
        let block = wave(8, &frames);
        let events = decode!(
            sd_registers,
            SdRegisters,
            Event,
            json!({}),
            8,
            &block,
            &[1, 19],
            GraphConfig {
                queue_capacity: 1,
                budget: 1
            }
        );
        if app && !truncated {
            contains(
                &events,
                json!({"kind":"scr","structure":0,"sdSpec":2,"dataAfterErase":true,"security":3,"busWidths":5,"spec3":true,"spec4":true,"commandSupport":3,"vendorData":0x12345678,"crcOk":!bad_crc,"supported":true}),
            );
        } else {
            assert!(!events.iter().any(|event| event["kind"] == "scr"));
        }
    }
}

#[test]
fn a7105_names_all_registers_and_does_not_label_reserved_pdv_as_power() {
    let mut frames: Vec<_> = (0..=0x32)
        .map(|address| vec![(address, 0), (0x55, 0)])
        .collect();
    frames.push(vec![(0xcf, 0)]);
    frames.push(vec![(0x40, 0), (0, 0x65)]);
    frames.push(vec![(0x28, 0), (0x17, 0)]);
    let block = wave(8, &frames);
    let events = decode!(
        a7105,
        A7105,
        Event,
        json!({}),
        8,
        &block,
        &[1, 29],
        GraphConfig {
            queue_capacity: 1,
            budget: 1
        }
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| event["kind"] == "register" && event["name"] != "unknown")
            .count(),
        53
    );
    contains(
        &events,
        json!({"kind":"register","address":50,"name":"filter-test"}),
    );
    contains(&events, json!({"kind":"operation","operation":"receive"}));
    for (field, value) in [
        ("fec-error", 1),
        ("crc-error", 1),
        ("pll-enabled", 1),
        ("trx-enabled", 1),
        ("pa-current-code", 2),
        ("tx-buffer-code", 7),
    ] {
        contains(&events, json!({"kind":"field","field":field,"value":value}));
    }
    assert!(
        !events
            .iter()
            .any(|event| event["kind"] == "field" && event["field"] == "tx-power-code")
    );
}
#[test]
fn tpm_interface_register_names_include_locality_and_version_boundaries() {
    for (version, addresses) in [
        (
            "tpm20",
            vec![
                (0xd40000, "access"),
                (0xd40008, "interrupt-enable"),
                (0xd42024, "data-fifo"),
                (0xd44024, "hash-data-or-fifo"),
                (0xd44028, "hash-start"),
                (0xd40030, "interface-id"),
                (0xd40080, "extended-data-fifo"),
                (0xd40f00, "device-vendor-id"),
                (0xd40f04, "revision-id"),
                (0x123456, "unknown"),
            ],
        ),
        (
            "tpm12",
            vec![
                (0xd40030, "reserved"),
                (0xd40f80, "first-legacy-address"),
                (0xd4001b, "reserved"),
            ],
        ),
    ] {
        let frames: Vec<_> = addresses
            .iter()
            .map(|(address, _)| {
                vec![
                    (0x80, 0),
                    ((address >> 16) as u8, 0),
                    ((address >> 8) as u8, 0),
                    (*address as u8, 1),
                    (0, 0x5a),
                ]
            })
            .collect();
        let block = wave(8, &frames);
        let events = decode!(
            spi_tpm,
            SpiTpm,
            Event,
            json!({"tpm_version":version}),
            8,
            &block,
            &[1, 17],
            GraphConfig {
                queue_capacity: 1,
                budget: 1
            }
        );
        for (address, name) in addresses {
            contains(
                &events,
                json!({"kind":"register","address":address,"name":name}),
            );
        }
    }
}
