// SPDX-License-Identifier: Apache-2.0

//! The parallel bus decoder on synthesized buses, with and without a clock.

use uiharu_core::testing::{Harness, assert_block_invariant};
use uiharu_core::{
    GraphConfig, Item, Levels, LogicBlock, LogicBlockBuilder, Tick, TickRange, TimeBase,
};
use uiharu_protocols::parallel::synth::ParallelWaveform;
use uiharu_protocols::parallel::{Clock, ClockEdge, Parallel, ParallelConfig, ParallelEvent};

fn harness(
    config: &ParallelConfig,
    channels: u8,
    graph_config: GraphConfig,
) -> Harness<ParallelEvent> {
    Harness::build(
        channels,
        TimeBase::new(1_000_000),
        graph_config,
        |graph, source| {
            let parallel = graph.add(Parallel::new(config.clone()).expect("a valid configuration"));
            graph.connect(source.samples, Parallel::input(parallel))?;
            Ok(Parallel::events(parallel))
        },
    )
    .expect("builds")
}

fn describe(value: u64, bits: u8, initial: bool, tick: Tick) -> String {
    let initial = if initial { " initial" } else { "" };
    format!("word {value:#x}/{bits}{initial} @{tick}")
}

/// One line per item, progress left out, checking that every word covers its own tick.
fn lines(items: &[Item<ParallelEvent>]) -> Vec<String> {
    items
        .iter()
        .filter_map(|item| match item {
            Item::Event(event) => {
                let ParallelEvent::Word {
                    value,
                    bits,
                    initial,
                } = event.payload;
                assert_eq!(event.span, TickRange::one(event.settled), "{event:?}");
                Some(describe(value, bits, initial, event.settled))
            }
            Item::Gap(range) => Some(format!("gap {range}")),
            Item::Progress(_) => None,
            Item::End(tick) => Some(format!("eof {tick}")),
        })
        .collect()
}

fn decode(
    config: &ParallelConfig,
    channels: u8,
    block: &LogicBlock,
    gaps: &[TickRange],
) -> Vec<String> {
    let mut harness = harness(config, channels, GraphConfig::default());
    harness.feed_with_gaps(block, gaps, &[u64::MAX]);
    harness.end();
    lines(&harness.items)
}

fn nibble_bus(clock: Option<ClockEdge>) -> ParallelConfig {
    ParallelConfig {
        data: vec![0, 1, 2, 3],
        clock: clock.map(|edge| Clock { channel: 4, edge }),
    }
}

#[test]
fn without_a_clock_every_change_is_a_word() {
    let config = nibble_bus(None);
    let mut waveform = ParallelWaveform::new(config.clone(), 4, 1, Tick::ZERO);
    waveform.set(0b0101, 10);
    // Lines 0 and 1 change at the same tick: one word.
    waveform.set(0b0110, 5);
    waveform.set(0b0111, 5);
    // Lines 3 and 2 change a tick apart: a word each.
    waveform.set(0b1111, 1);
    waveform.set(0b1011, 4);
    // No line changes: no word.
    waveform.set(0b1011, 5);
    let block = waveform.finish();
    assert_eq!(
        decode(&config, 4, &block, &[]),
        vec![
            "word 0x5/4 initial @0",
            "word 0x6/4 @10",
            "word 0x7/4 @15",
            "word 0xf/4 @20",
            "word 0xb/4 @21",
            "eof 30"
        ]
    );
}

/// Four words clocked out at three samples per half period after five idle samples, read at
/// 8, 14, 20 and 26 on every kind of clock; the samples end at 34.
fn four_clocked_words(config: &ParallelConfig) -> LogicBlock {
    let mut waveform = ParallelWaveform::new(config.clone(), 5, 3, Tick::ZERO);
    waveform.idle(5);
    waveform.clocked_words([0x3, 0xa, 0xf, 0x0]);
    waveform.idle(5);
    waveform.finish()
}

#[test]
fn a_clock_reads_a_word_on_its_edges() {
    for edge in [ClockEdge::Rising, ClockEdge::Falling, ClockEdge::Either] {
        let config = nibble_bus(Some(edge));
        let block = four_clocked_words(&config);
        // A clock read on one edge goes back on the other as the data changes: that edge
        // reads nothing.
        assert_eq!(
            decode(&config, 5, &block, &[]),
            vec![
                "word 0x3/4 @8",
                "word 0xa/4 @14",
                "word 0xf/4 @20",
                "word 0x0/4 @26",
                "eof 34"
            ],
            "{edge:?}"
        );
    }
    // A clock that changes once per word, read on one edge only, reads every other word.
    let block = four_clocked_words(&nibble_bus(Some(ClockEdge::Either)));
    assert_eq!(
        decode(&nibble_bus(Some(ClockEdge::Rising)), 5, &block, &[]),
        vec!["word 0x3/4 @8", "word 0xf/4 @20", "eof 34"]
    );
    assert_eq!(
        decode(&nibble_bus(Some(ClockEdge::Falling)), 5, &block, &[]),
        vec!["word 0xa/4 @14", "word 0x0/4 @26", "eof 34"]
    );
}

#[test]
fn lines_are_listed_least_significant_first_on_wide_buses() {
    let config = ParallelConfig {
        data: vec![15, 3, 7, 0, 12, 9, 1, 14, 5, 10, 2, 13],
        clock: None,
    };
    let mut waveform = ParallelWaveform::new(config.clone(), 16, 1, Tick::ZERO);
    for value in [0xabc, 0x123, 0xfff, 0x800] {
        waveform.set(value, 4);
    }
    let block = waveform.finish();
    // Bit 0 is channel 15 and bit 11 channel 13.
    assert_eq!(block.level(15, Tick(0)), Some(false));
    assert_eq!(block.level(13, Tick(0)), Some(true));
    assert_eq!(
        decode(&config, 16, &block, &[]),
        vec![
            "word 0xabc/12 initial @0",
            "word 0x123/12 @4",
            "word 0xfff/12 @8",
            "word 0x800/12 @12",
            "eof 16"
        ]
    );

    let config = ParallelConfig {
        data: (0..64).rev().collect(),
        clock: None,
    };
    let mut waveform = ParallelWaveform::new(config.clone(), 64, 1, Tick::ZERO);
    for value in [(1 << 63) | 1, u64::MAX, 0x0123_4567_89ab_cdef, 0] {
        waveform.set(value, 3);
    }
    let block = waveform.finish();
    assert_eq!(
        decode(&config, 64, &block, &[]),
        vec![
            "word 0x8000000000000001/64 initial @0",
            "word 0xffffffffffffffff/64 @3",
            "word 0x123456789abcdef/64 @6",
            "word 0x0/64 @9",
            "eof 12"
        ]
    );
}

#[test]
fn after_a_gap_the_levels_are_found_again() {
    let config = ParallelConfig {
        data: vec![0, 1],
        clock: None,
    };
    let mut waveform = ParallelWaveform::new(config.clone(), 2, 1, Tick::ZERO);
    for value in [1, 2, 3, 0] {
        waveform.set(value, 10);
    }
    let block = waveform.finish();
    // The change at 20 is lost in the gap; the levels after it are an initial word again.
    let gap = TickRange::new(Tick(15), Tick(25));
    assert_eq!(
        decode(&config, 2, &block, &[gap]),
        vec![
            "word 0x1/2 initial @0",
            "word 0x2/2 @10",
            "gap 15..25",
            "word 0x3/2 initial @25",
            "word 0x0/2 @30",
            "eof 40"
        ]
    );

    // With a clock nothing is read at the first sample after a gap, even where the clock
    // reads a word in the samples.
    let config = nibble_bus(Some(ClockEdge::Rising));
    let block = four_clocked_words(&config);
    let gap = TickRange::new(Tick(12), Tick(14));
    assert_eq!(
        decode(&config, 5, &block, &[gap]),
        vec![
            "word 0x3/4 @8",
            "gap 12..14",
            "word 0xf/4 @20",
            "word 0x0/4 @26",
            "eof 34"
        ]
    );
}

/// Eight busy channels: each step flips one line, sometimes two at once, and holds 1 to 6
/// samples.
fn irregular() -> LogicBlock {
    let mut builder = LogicBlockBuilder::new(Tick::ZERO, 8);
    let mut state = 0x9e37_79b9_7f4a_7c15u64;
    let mut levels = 0u64;
    for _ in 0..1500 {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        levels ^= 1 << (state % 8);
        if state >> 20 & 3 == 0 {
            levels ^= 1 << (state >> 3 & 7);
        }
        builder.push_repeated(Levels(levels), (state >> 8) % 6 + 1);
    }
    builder.finish().expect("a block")
}

/// What the decoder must report, found sample by sample.
fn naive(config: &ParallelConfig, block: &LogicBlock, gaps: &[TickRange]) -> Vec<String> {
    let mut lines = Vec::new();
    let mut previous: Option<Levels> = None;
    let mut tick = block.start();
    while tick < block.end() {
        if let Some(gap) = gaps.iter().find(|gap| gap.contains(tick)) {
            lines.push(format!("gap {gap}"));
            previous = None;
            tick = gap.end;
            continue;
        }
        let levels = block.levels(tick).expect("in the block");
        let changed = |channel: u8| {
            previous.is_some_and(|before| before.high(channel) != levels.high(channel))
        };
        let read = match config.clock {
            None if previous.is_none() => Some(true),
            None => config
                .data
                .iter()
                .any(|&channel| changed(channel))
                .then_some(false),
            Some(clock) => (changed(clock.channel) && clock.edge.reads(levels.high(clock.channel)))
                .then_some(false),
        };
        if let Some(initial) = read {
            lines.push(describe(config.word(levels), config.bits(), initial, tick));
        }
        previous = Some(levels);
        tick = tick.next();
    }
    lines.push(format!("eof {}", block.end()));
    lines
}

fn irregular_configs() -> Vec<ParallelConfig> {
    let mut configs = vec![ParallelConfig {
        data: vec![6, 1, 4],
        clock: None,
    }];
    for edge in [ClockEdge::Rising, ClockEdge::Falling, ClockEdge::Either] {
        configs.push(ParallelConfig {
            data: vec![0, 7, 5, 2],
            clock: Some(Clock { channel: 3, edge }),
        });
    }
    configs
}

#[test]
fn words_match_a_sample_by_sample_scan() {
    let block = irregular();
    let gaps = [
        TickRange::new(Tick(100), Tick(120)),
        TickRange::new(Tick(1024), Tick(1025)),
        TickRange::new(Tick(3000), Tick(3200)),
    ];
    for config in irregular_configs() {
        let decoded = decode(&config, 8, &block, &gaps);
        assert!(decoded.len() > 50, "{config:?}: {decoded:?}");
        assert_eq!(decoded, naive(&config, &block, &gaps), "{config:?}");
    }
}

#[test]
fn blocks_pauses_and_gaps_do_not_change_the_words() {
    let block = irregular();
    let gaps = [
        TickRange::new(Tick(63), Tick(64)),
        TickRange::new(Tick(900), Tick(990)),
        TickRange::new(Tick(2500), Tick(2600)),
    ];
    for config in irregular_configs() {
        assert_block_invariant(|graph| harness(&config, 8, graph), &block, &gaps);
    }
}
