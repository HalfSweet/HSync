// SPDX-License-Identifier: Apache-2.0
//! Independent generic RGB bytes on clock/MOSI, without CS or MISO.
mod dsl_spi_support;
use serde_json::json;
use uiharu_core::testing::Harness;
use uiharu_core::{GraphConfig, InputRef, Item, LogicBlock, OutputRef, Tick, TickRange, TimeBase};
use uiharu_dsl_protocols::rgb_led_spi::{Config, Event, RgbLedSpi};
use uiharu_interp::{DynEvent, Interpreter, ToDyn};
use uiharu_protocols::spi::{Spi, SpiConfig, SpiEvent, synth::SpiWaveform};
fn config() -> Config {
    serde_json::from_value(json!({"order":"rgb","framed":false})).expect("config")
}
fn run(mode: u8, gaps: &[TickRange], g: GraphConfig, sizes: &[u64]) -> Vec<Item<Event>> {
    let spi_config = SpiConfig {
        cs: None,
        miso: None,
        mode,
        ..Default::default()
    };
    let mut wire = SpiWaveform::new(spi_config, 8, 2, Tick::ZERO);
    wire.idle(5);
    wire.transaction([
        (0x12, 0),
        (0x34, 0),
        (0x56, 0),
        (0x78, 0),
        (0x9a, 0),
        (0xbc, 0),
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
        let rgb = graph.add(RgbLedSpi::new(config()).expect("rgb"));
        graph.connect(source.samples, Spi::input(spi))?;
        graph.connect(Spi::events(spi), RgbLedSpi::input(rgb))?;
        Ok(RgbLedSpi::events(rgb))
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
                    dsl_spi_support::program("rgb-led-spi"),
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
    dsl_spi_support::assert_items(&h.items, &interpreted.items, "RGB continuous");
    h.items
}
#[test]
fn two_wire_rgb_in_mode_zero_and_three_agrees_with_interpreter_and_schedules() {
    for mode in [0, 3] {
        let first = run(
            mode,
            &[],
            GraphConfig {
                queue_capacity: 1,
                budget: 1,
            },
            &[1, 17],
        );
        let second = run(
            mode,
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
        let pixels: Vec<_> = first
            .iter()
            .filter_map(|item| match item {
                Item::Event(event) => match event.payload {
                    Event::Pixel {
                        red, green, blue, ..
                    } => Some((red, green, blue)),
                    _ => None,
                },
                _ => None,
            })
            .collect();
        assert_eq!(pixels, [(0x12, 0x34, 0x56), (0x78, 0x9a, 0xbc)]);
    }
}
#[test]
fn lost_samples_suspend_csless_triplet_alignment() {
    let items = run(
        0,
        &[TickRange::new(Tick(120), Tick(125))],
        GraphConfig {
            queue_capacity: 1,
            budget: 1,
        },
        &[1, 13],
    );
    assert_eq!(
        items
            .iter()
            .filter(
                |item| matches!(item,Item::Event(event)if matches!(event.payload,Event::Pixel{..}))
            )
            .count(),
        1
    );
}
#[test]
fn falling_edge_spi_modes_are_rejected_by_the_checked_graph() {
    for mode in [1, 2] {
        assert!(
            Harness::<Event>::build(
                8,
                TimeBase::new(1_000_000),
                GraphConfig::default(),
                |graph, source| {
                    let spi = graph.add(
                        Spi::new(SpiConfig {
                            cs: None,
                            miso: None,
                            mode,
                            ..Default::default()
                        })
                        .expect("spi"),
                    );
                    let rgb = graph.add(RgbLedSpi::new(config()).expect("rgb"));
                    graph.connect(source.samples, Spi::input(spi))?;
                    graph.connect(Spi::events(spi), RgbLedSpi::input(rgb))?;
                    Ok(RgbLedSpi::events(rgb))
                }
            )
            .is_err()
        );
    }
}
#[test]
fn initial_partial_begin_is_rejected_until_a_clean_cs_boundary() {
    let spi_config = SpiConfig {
        miso: None,
        ..Default::default()
    };
    let mut wire = SpiWaveform::new(spi_config, 8, 2, Tick::ZERO);
    wire.idle(5);
    wire.transaction([
        (0x10, 0),
        (0x20, 0),
        (0x30, 0),
        (0x40, 0),
        (0x50, 0),
        (0x60, 0),
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
        assert!(!items.iter().any(
            |item| matches!(item,Item::Event(event)if matches!(event.payload,Event::Pixel{..}))
        ));
    }
    let mut wire = SpiWaveform::new(spi_config, 8, 2, Tick::ZERO);
    wire.idle(5);
    wire.transaction([
        (0x10, 0),
        (0x20, 0),
        (0x30, 0),
        (0x40, 0),
        (0x50, 0),
        (0x60, 0),
    ]);
    wire.idle(5);
    wire.transaction([(0xaa, 0), (0xbb, 0), (0xcc, 0)]);
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
    let pixels: Vec<_> = items
        .iter()
        .filter_map(|item| match item {
            Item::Event(event) => match event.payload {
                Event::Pixel {
                    red, green, blue, ..
                } => Some((red, green, blue)),
                _ => None,
            },
            _ => None,
        })
        .collect();
    assert_eq!(pixels, [(0xaa, 0xbb, 0xcc)]);
}
