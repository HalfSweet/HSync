// SPDX-License-Identifier: Apache-2.0
//! ADE7758 RevE table17 byte-stream register widths, without CS.
mod dsl_spi_support;
use serde_json::json;
use uiharu_core::testing::Harness;
use uiharu_core::{GraphConfig, InputRef, Item, LogicBlock, OutputRef, Tick, TickRange, TimeBase};
use uiharu_dsl_protocols::ade77xx::{Ade77xx, Config, Event};
use uiharu_interp::{DynEvent, Interpreter, ToDyn};
use uiharu_protocols::spi::{Spi, SpiConfig, SpiEvent, synth::SpiWaveform};
fn config() -> Config {
    serde_json::from_value(json!({"model":"ade7758","framed":false})).expect("config")
}
fn run(mode: u8, gaps: &[TickRange], g: GraphConfig, sizes: &[u64]) -> Vec<Item<Event>> {
    let spi_config = SpiConfig {
        cs: None,
        miso: Some(3),
        mode,
        ..Default::default()
    };
    let mut wire = SpiWaveform::new(spi_config, 8, 2, Tick::ZERO);
    wire.idle(5);
    wire.transaction([
        (0x01, 0),
        (0, 0x80),
        (0, 0),
        (0x0a, 0),
        (0, 0x12),
        (0, 0x34),
        (0, 0x56),
        (0x93, 0),
        (0x45, 0),
        (0xc0, 0),
        (0x7f, 0),
        (0x7f, 0),
        (0, 9),
    ]);
    wire.idle(5);
    let block = wire.finish();
    run_block(spi_config, &block, gaps, g, sizes)
}
fn run_block(
    spi_config: SpiConfig,
    block: &LogicBlock,
    gaps: &[TickRange],
    g: GraphConfig,
    sizes: &[u64],
) -> Vec<Item<Event>> {
    let mut h = Harness::<Event>::build(8, TimeBase::new(1_000_000), g, |graph, source| {
        let spi = graph.add(Spi::new(spi_config).expect("spi"));
        let rgb = graph.add(Ade77xx::new(config()).expect("rgb"));
        graph.connect(source.samples, Spi::input(spi))?;
        graph.connect(Spi::events(spi), Ade77xx::input(rgb))?;
        Ok(Ade77xx::events(rgb))
    })
    .expect("graph");
    h.feed_with_gaps(block, gaps, sizes);
    h.end();
    let mut interpreted =
        Harness::<DynEvent>::build(8, TimeBase::new(1_000_000), g, |graph, source| {
            let node = Spi::new(spi_config).expect("spi");
            let props = uiharu_core::Node::spec(&node).outputs[0].props.clone();
            let spi = graph.add(node);
            let bridge = graph.add(ToDyn::<SpiEvent>::new(props));
            let rgb = graph.add(
                Interpreter::new(
                    dsl_spi_support::program("ade77xx"),
                    &serde_json::to_value(config()).expect("config"),
                )
                .expect("interpreter"),
            );
            graph.connect(source.samples, Spi::input(spi))?;
            graph.connect(Spi::events(spi), InputRef::<SpiEvent>::of_node(bridge, 0))?;
            graph.connect(
                OutputRef::<DynEvent>::of_node(bridge, 0),
                Interpreter::bus_input(rgb),
            )?;
            Ok(Interpreter::events(rgb))
        })
        .expect("graph");
    interpreted.feed_with_gaps(block, gaps, sizes);
    interpreted.end();
    dsl_spi_support::assert_items(&h.items, &interpreted.items, "ADE continuous");
    h.items
}
#[test]
fn ade7758_seven_bit_addresses_and_variable_widths_are_intrinsically_framed() {
    let first = run(
        1,
        &[],
        GraphConfig {
            queue_capacity: 1,
            budget: 1,
        },
        &[1, 17],
    );
    let second = run(
        1,
        &[],
        GraphConfig {
            queue_capacity: 8,
            budget: 128,
        },
        &[u64::MAX],
    );
    assert_eq!(
        uiharu_core::testing::without_progress(&first),
        uiharu_core::testing::without_progress(&second)
    );
    let registers: Vec<_> = first
        .iter()
        .filter_map(|item| match item {
            Item::Event(event) => match event.payload {
                Event::Register {
                    register,
                    bits,
                    raw,
                    negative,
                    magnitude,
                    ..
                } => Some((
                    serde_json::to_value(register).expect("name"),
                    bits,
                    raw,
                    negative,
                    magnitude,
                )),
                _ => None,
            },
            _ => None,
        })
        .collect();
    assert_eq!(
        registers,
        [
            (json!("awatthr"), 16, 32768, true, 32768),
            (json!("airms"), 24, 0x123456, false, 0x123456),
            (json!("opmode"), 8, 0x45, false, 0x45),
            (json!("bphcal"), 7, 127, true, 1),
            (json!("version"), 8, 9, false, 9)
        ]
    );
    assert!(first.iter().any(|item|matches!(item,Item::Event(event)if matches!(event.payload,Event::OperationalMode{high_pass_disabled:true,cf_disabled:true,reset:true,..}))));
}
#[test]
fn a_gap_cannot_join_halves_of_a_csless_meter_register() {
    let items = run(
        1,
        &[TickRange::new(Tick(140), Tick(145))],
        GraphConfig {
            queue_capacity: 1,
            budget: 1,
        },
        &[1, 13],
    );
    assert_eq!(items.iter().filter(|item|matches!(item,Item::Event(event)if matches!(event.payload,Event::Register{..}))).count(),1);
}
#[test]
fn initial_partial_begin_does_not_fabricate_registers_and_clean_cs_recovers() {
    let spi_config = SpiConfig {
        mode: 1,
        ..Default::default()
    };
    let mut wire = SpiWaveform::new(spi_config, 8, 2, Tick::ZERO);
    wire.idle(5);
    wire.transaction([
        (0x01, 0),
        (0, 0x12),
        (0, 0x34),
        (0x01, 0),
        (0, 0x56),
        (0, 0x78),
    ]);
    wire.idle(5);
    let first = wire.finish();
    let partial = first
        .slice(TickRange::new(Tick(15), first.end()))
        .expect("mid-byte slice");
    let mut source = Harness::<SpiEvent>::build(
        8,
        TimeBase::new(1_000_000),
        GraphConfig::default(),
        |graph, input| {
            let spi = graph.add(Spi::new(spi_config).expect("spi"));
            graph.connect(input.samples, Spi::input(spi))?;
            Ok(Spi::events(spi))
        },
    )
    .expect("source graph");
    source.run_all(&partial);
    assert!(
        source
            .events()
            .iter()
            .any(|event| matches!(event.payload, SpiEvent::Begin { partial: true, .. }))
    );
    for graph in [
        GraphConfig::default(),
        GraphConfig {
            queue_capacity: 1,
            budget: 1,
        },
    ] {
        let items = run_block(spi_config, &partial, &[], graph, &[1, 13]);
        assert!(!items.iter().any(|item|matches!(item,Item::Event(event)if matches!(event.payload,Event::Register{..}|Event::Command{..}))));
    }
    let mut wire = SpiWaveform::new(spi_config, 8, 2, Tick::ZERO);
    wire.idle(5);
    wire.transaction([
        (0x01, 0),
        (0, 0x12),
        (0, 0x34),
        (0x01, 0),
        (0, 0x56),
        (0, 0x78),
    ]);
    wire.idle(5);
    wire.transaction([(0x01, 0), (0, 0x56), (0, 0x78)]);
    wire.idle(5);
    let whole = wire.finish();
    let block = whole
        .slice(TickRange::new(Tick(15), whole.end()))
        .expect("slice");
    let items = run_block(
        spi_config,
        &block,
        &[],
        GraphConfig {
            queue_capacity: 1,
            budget: 1,
        },
        &[1, 17],
    );
    let values: Vec<_> = items
        .iter()
        .filter_map(|item| match item {
            Item::Event(event) => match event.payload {
                Event::Register { raw, .. } => Some(raw),
                _ => None,
            },
            _ => None,
        })
        .collect();
    assert_eq!(values, [0x5678]);
}
