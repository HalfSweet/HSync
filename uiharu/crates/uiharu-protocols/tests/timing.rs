// SPDX-License-Identifier: Apache-2.0

//! The edge timing decoder on synthesized edge sequences.

use std::sync::Arc;

use uiharu_core::testing::{Harness, assert_block_invariant};
use uiharu_core::{
    GraphConfig, Item, Levels, LogicBlock, LogicBlockBuilder, SignalItem, Tick, TickRange, TimeBase,
};
use uiharu_protocols::timing::synth::EdgeWaveform;
use uiharu_protocols::timing::{Edges, Slope, Timing, TimingConfig, TimingEvent};

const CHANNELS: u8 = 4;

fn harness(config: &TimingConfig, graph_config: GraphConfig) -> Harness<TimingEvent> {
    Harness::build(
        CHANNELS,
        TimeBase::new(1_000_000),
        graph_config,
        |graph, source| {
            let timing = graph.add(Timing::new(config.clone()).expect("a valid configuration"));
            graph.connect(source.samples, Timing::input(timing))?;
            Ok(Timing::events(timing))
        },
    )
    .expect("builds")
}

fn describe(slope: Slope, since_previous: Option<u64>, tick: Tick) -> String {
    let slope = match slope {
        Slope::Rising => "rising",
        Slope::Falling => "falling",
    };
    let since = since_previous.map_or("-".to_owned(), |ticks| format!("+{ticks}"));
    format!("{slope} {since} @{tick}")
}

/// One line per item, progress left out, checking that every edge covers its own tick.
fn lines(items: &[Item<TimingEvent>]) -> Vec<String> {
    items
        .iter()
        .filter_map(|item| match item {
            Item::Event(event) => {
                let TimingEvent::Edge {
                    slope,
                    since_previous,
                } = event.payload;
                assert_eq!(event.span, TickRange::one(event.settled), "{event:?}");
                Some(describe(slope, since_previous, event.settled))
            }
            Item::Gap(range) => Some(format!("gap {range}")),
            Item::Progress(_) => None,
            Item::End(tick) => Some(format!("eof {tick}")),
        })
        .collect()
}

fn decode(config: &TimingConfig, block: &LogicBlock, gaps: &[TickRange]) -> Vec<String> {
    let mut harness = harness(config, GraphConfig::default());
    harness.feed_with_gaps(block, gaps, &[u64::MAX]);
    harness.end();
    lines(&harness.items)
}

/// Edges at 10 (rising), 15, 22 and 25 on channel 2; the samples end at 45.
fn four_edges(config: &TimingConfig) -> LogicBlock {
    let mut waveform = EdgeWaveform::new(config, CHANNELS, Tick::ZERO, false);
    waveform.hold(10);
    waveform.toggles([5, 7, 3, 20]);
    waveform.finish()
}

#[test]
fn reports_every_edge_with_the_ticks_since_the_one_before() {
    let config = TimingConfig {
        channel: 2,
        edges: Edges::Any,
    };
    let block = four_edges(&config);
    assert_eq!(
        decode(&config, &block, &[]),
        vec![
            "rising - @10",
            "falling +5 @15",
            "rising +7 @22",
            "falling +3 @25",
            "eof 45"
        ]
    );
}

#[test]
fn one_kind_of_edge_measures_the_period() {
    let rising = TimingConfig {
        channel: 2,
        edges: Edges::Rising,
    };
    let block = four_edges(&rising);
    assert_eq!(
        decode(&rising, &block, &[]),
        vec!["rising - @10", "rising +12 @22", "eof 45"]
    );
    let falling = TimingConfig {
        edges: Edges::Falling,
        ..rising
    };
    assert_eq!(
        decode(&falling, &block, &[]),
        vec!["falling - @15", "falling +10 @25", "eof 45"]
    );
}

#[test]
fn the_first_sample_is_no_edge() {
    let config = TimingConfig::default();
    // The line is high from the first sample on, which does not start at tick 0.
    let mut waveform = EdgeWaveform::new(&config, CHANNELS, Tick(1000), true);
    waveform.hold(6);
    waveform.toggles([4, 5]);
    let block = waveform.finish();
    assert_eq!(
        decode(&config, &block, &[]),
        vec!["falling - @1006", "rising +4 @1010", "eof 1015"]
    );
}

#[test]
fn a_gap_restarts_the_intervals() {
    let config = TimingConfig::default();
    let mut waveform = EdgeWaveform::new(&config, CHANNELS, Tick::ZERO, false);
    waveform.hold(10);
    waveform.toggles([10; 6]);
    let block = waveform.finish();
    // The edge at 40 lands on the first sample after the gap, which is no edge.
    let gap = TickRange::new(Tick(25), Tick(40));
    assert_eq!(
        decode(&config, &block, &[gap]),
        vec![
            "rising - @10",
            "falling +10 @20",
            "gap 25..40",
            "rising - @50",
            "falling +10 @60",
            "eof 70"
        ]
    );
}

#[test]
fn progress_is_passed_on() {
    let config = TimingConfig::default();
    let mut harness = harness(&config, GraphConfig::default());
    let mut builder = LogicBlockBuilder::new(Tick::ZERO, CHANNELS);
    builder.push_repeated(Levels(1), 100);
    harness.push(SignalItem::Data(Arc::new(
        builder.finish().expect("a block"),
    )));
    harness.collect();
    assert_eq!(harness.items, vec![Item::Progress(Tick(100))]);
}

/// A long, irregular sequence of edges on channel 1, the other channels busy too.
fn irregular() -> LogicBlock {
    let mut builder = LogicBlockBuilder::new(Tick::ZERO, CHANNELS);
    let mut level = 0u64;
    for i in 0..400u64 {
        if i % 3 != 0 {
            level ^= 0b10;
        }
        let noise = (i * 5) & 0b1101;
        builder.push_repeated(Levels(level | noise), (i * 37 + 11) % 23 + 1);
    }
    builder.finish().expect("a block")
}

/// What the decoder must report, found sample by sample.
fn naive(config: &TimingConfig, block: &LogicBlock, gaps: &[TickRange]) -> Vec<String> {
    let mut lines = Vec::new();
    let mut previous_level: Option<bool> = None;
    let mut previous_edge: Option<Tick> = None;
    let mut tick = block.start();
    while tick < block.end() {
        if let Some(gap) = gaps.iter().find(|gap| gap.contains(tick)) {
            lines.push(format!("gap {gap}"));
            previous_level = None;
            previous_edge = None;
            tick = gap.end;
            continue;
        }
        let level = block.level(config.channel, tick).expect("in the block");
        if previous_level.is_some_and(|previous| previous != level) {
            let slope = if level { Slope::Rising } else { Slope::Falling };
            if config.edges.accepts(slope) {
                let since = previous_edge.map(|edge| edge.until(tick));
                lines.push(describe(slope, since, tick));
                previous_edge = Some(tick);
            }
        }
        previous_level = Some(level);
        tick = tick.next();
    }
    lines.push(format!("eof {}", block.end()));
    lines
}

#[test]
fn edges_match_a_sample_by_sample_scan() {
    let block = irregular();
    let gaps = [
        TickRange::new(Tick(300), Tick(345)),
        TickRange::new(Tick(1000), Tick(1001)),
        TickRange::new(Tick(1500), Tick(1600)),
    ];
    for edges in [Edges::Any, Edges::Rising, Edges::Falling] {
        let config = TimingConfig { channel: 1, edges };
        let decoded = decode(&config, &block, &gaps);
        assert!(decoded.len() > 100, "{edges:?}: {decoded:?}");
        assert_eq!(decoded, naive(&config, &block, &gaps), "{edges:?}");
    }
}

#[test]
fn blocks_pauses_and_gaps_do_not_change_the_edges() {
    let block = irregular();
    let gaps = [
        TickRange::new(Tick(64), Tick(65)),
        TickRange::new(Tick(700), Tick(763)),
        TickRange::new(Tick(2000), Tick(2100)),
    ];
    for edges in [Edges::Any, Edges::Rising, Edges::Falling] {
        let config = TimingConfig { channel: 1, edges };
        assert_block_invariant(|graph| harness(&config, graph), &block, &gaps);
    }
}
