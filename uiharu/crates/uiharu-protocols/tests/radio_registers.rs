// SPDX-License-Identifier: Apache-2.0
//! Independent register-table and addressing vectors from TI SWRS061I/ST DS12484 Rev8.
mod dsl_spi_support;
use serde_json::{Value, json};
use uiharu_core::testing::Harness;
use uiharu_core::{GraphConfig, Item, LogicBlock, Tick, TickRange, TimeBase};
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
    ($module:ident,$node:ident,$event:ident,$config:expr,$bits:expr,$block:expr,$sizes:expr,$g:expr,$gaps:expr) => {{
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
        h.feed_cut($block, $gaps, $sizes, &[0, 1, 7]);
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
        interpreted.feed_cut($block, $gaps, $sizes, &[0, 1, 7]);
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
fn cc1101_configuration_status_and_fifo_names_follow_manufacturer_address_table() {
    // SWRS061I Tables43/44: config0..2E, status30..3D individually selected B=1.
    let mut frames = vec![(0x40, 0)];
    frames.extend((0..47).map(|value| (value, 0)));
    let mut transactions = vec![frames];
    for address in 0x30..=0x3d {
        transactions.push(vec![(0xc0 | address, 0), (0, address)])
    }
    transactions.push(vec![(0x7e, 0), (1, 0), (2, 0)]);
    transactions.push(vec![(0xff, 0), (0, 0xa5), (0, 0x5a)]);
    let block = wave(8, &transactions);
    for graph in [
        GraphConfig::default(),
        GraphConfig {
            queue_capacity: 1,
            budget: 3,
        },
    ] {
        let events = decode!(
            cc1101,
            Cc1101,
            Event,
            json!({}),
            8,
            &block,
            &[1, 13, 211],
            graph,
            &[]
        );
        for (address, name) in [
            (0, "IOCFG2"),
            (0x2e, "TEST0"),
            (0x30, "PARTNUM"),
            (0x37, "WORTIME0"),
            (0x3d, "RCCTRL0_STATUS"),
            (0x3e, "PATABLE"),
            (0x3f, "FIFO"),
        ] {
            contains(
                &events,
                json!({"kind":"register","address":address,"register":name}),
            );
        }
        assert_eq!(
            events
                .iter()
                .filter(|event| event["kind"] == "register"
                    && event["address"]
                        .as_u64()
                        .is_some_and(|address| address < 47))
                .count(),
            47
        );
        assert!(
            !events
                .iter()
                .any(|event| event["kind"] == "register" && event["register"] == "unknown")
        );
    }
}
#[test]
fn cc1101_status_is_single_access_and_strobes_ignore_read_bit() {
    // §10.2: status access B=1 does not enable address-increment bursts.
    // §10.4: strobe R/W may be either value. A second byte is a new header.
    let block = wave(
        8,
        &[
            vec![(0xf0, 0), (0, 0x11), (0xf5, 0), (0, 0x22)],
            vec![(0xb1, 0), (0xb2, 0), (0xb8, 0), (0xbc, 0)],
            vec![(0x70, 0), (0, 0)],
            vec![(0x37, 0)],
        ],
    );
    let events = decode!(
        cc1101,
        Cc1101,
        Event,
        json!({}),
        8,
        &block,
        &[3, 1, 17],
        GraphConfig {
            queue_capacity: 1,
            budget: 2
        },
        &[]
    );
    contains(
        &events,
        json!({"kind":"register","register":"PARTNUM","value":0x11}),
    );
    contains(
        &events,
        json!({"kind":"register","register":"MARCSTATE","value":0x22}),
    );
    for operation in [
        "frequency-synth-on",
        "crystal-off",
        "wake-on-radio",
        "reset-wor-timer",
    ] {
        contains(&events, json!({"kind":"operation","operation":operation}));
    }
    assert_eq!(
        events
            .iter()
            .filter(|event| event["kind"] == "end" && event["reason"] == "unsupported")
            .count(),
        2
    );
}
#[test]
fn st25_register_spaces_have_complete_distinct_names_without_fifo_aliases() {
    // DS12484 Rev8 Tables17/18: all64A names, explicit16B names, test01 only.
    let mut space_a = vec![(0x40, 0)];
    space_a.extend((0..64).map(|value| (0, value)));
    let block = wave(
        8,
        &[
            space_a,
            vec![(0xfb, 0), (0x45, 0), (0, 0x11), (0, 0x22)],
            vec![(0xfb, 0), (0x68, 0), (0, 1), (0, 2), (0, 3), (0, 4), (0, 5)],
            vec![(0xfc, 0), (0x41, 0), (0, 0x33)],
            vec![(0x9f, 0), (0, 0xaa), (0, 0xbb)],
            vec![(0x7f, 0), (0, 0x44)],
        ],
    );
    let events = decode!(
        st25r39xx_spi,
        St25r39xxSpi,
        Event,
        json!({}),
        8,
        &block,
        &[1, 17, 5],
        GraphConfig {
            queue_capacity: 1,
            budget: 3
        },
        &[]
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| event["kind"] == "register"
                && event["tx"] == 1
                && event["space"] == "a")
            .count(),
        64
    );
    for (space, address, name) in [
        ("a", 0, "io-configuration-1"),
        ("a", 63, "ic-identity"),
        ("b", 5, "emd-suppression-configuration"),
        ("b", 6, "subcarrier-start-timer"),
        ("b", 40, "auxiliary-modulation-setting"),
        ("b", 44, "regulator-display"),
        ("test", 1, "analog-test-observation-1"),
    ] {
        contains(
            &events,
            json!({"kind":"register","space":space,"address":address,"register":name}),
        );
    }
    assert!(
        !events
            .iter()
            .any(|event| event["kind"] == "register" && event["tx"] == 5)
    );
    contains(
        &events,
        json!({"kind":"register","tx":6,"space":"a","register":"ic-identity"}),
    );
}
#[test]
fn radio_register_context_is_invariant_through_gaps_and_backpressure() {
    let block = wave(
        8,
        &[
            vec![(0xfb, 0), (0x68, 0), (0, 1), (0, 2), (0, 3)],
            vec![(0x40, 0), (0, 0x44), (0, 0x55)],
            vec![(0xc0, 0)],
        ],
    );
    let gaps = [TickRange::new(Tick(50), Tick(90))];
    let events = decode!(
        st25r39xx_spi,
        St25r39xxSpi,
        Event,
        json!({}),
        8,
        &block,
        &[1, 7, 37],
        GraphConfig {
            queue_capacity: 1,
            budget: 1
        },
        &gaps
    );
    contains(
        &events,
        json!({"kind":"register","space":"a","address":0,"register":"io-configuration-1","value":0x44}),
    );
    let block = wave(
        8,
        &[
            vec![(0x40, 0), (1, 0), (2, 0), (3, 0)],
            vec![(0xf0, 0), (0, 0x11)],
        ],
    );
    let events = decode!(
        cc1101,
        Cc1101,
        Event,
        json!({}),
        8,
        &block,
        &[2, 11, 17],
        GraphConfig {
            queue_capacity: 1,
            budget: 1
        },
        &gaps
    );
    contains(
        &events,
        json!({"kind":"register","register":"PARTNUM","value":0x11}),
    );
}
