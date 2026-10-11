// SPDX-License-Identifier: Apache-2.0

//! The PWM decoder on synthesized pulse trains.

use uiharu_core::testing::{Harness, assert_block_invariant};
use uiharu_core::{
    GraphConfig, Item, Levels, LogicBlock, LogicBlockBuilder, Tick, TickRange, TimeBase,
};
use uiharu_protocols::pwm::synth::PwmWaveform;
use uiharu_protocols::pwm::{Polarity, Pwm, PwmConfig, PwmEvent};

const CHANNELS: u8 = 2;

fn harness(config: &PwmConfig, graph_config: GraphConfig) -> Harness<PwmEvent> {
    Harness::build(
        CHANNELS,
        TimeBase::new(1_000_000),
        graph_config,
        |graph, source| {
            let pwm = graph.add(Pwm::new(config.clone()).expect("a valid configuration"));
            graph.connect(source.samples, Pwm::input(pwm))?;
            Ok(Pwm::events(pwm))
        },
    )
    .expect("builds")
}

fn cycle(period: u64, active: u64, start: Tick, end: Tick) -> String {
    format!("cycle {period}/{active} [{start},{end})@{end}")
}

/// One line per item, progress left out.
fn lines(items: &[Item<PwmEvent>]) -> Vec<String> {
    items
        .iter()
        .filter_map(|item| match item {
            Item::Event(event) => {
                let PwmEvent::Cycle {
                    period_ticks,
                    active_ticks,
                } = event.payload;
                // A cycle covers its own ticks and is decided by the leading edge after it.
                assert_eq!(event.span.end, event.settled, "{event:?}");
                assert_eq!(event.span.len(), period_ticks, "{event:?}");
                Some(cycle(
                    period_ticks,
                    active_ticks,
                    event.span.start,
                    event.span.end,
                ))
            }
            Item::Gap(range) => Some(format!("gap {range}")),
            Item::Progress(_) => None,
            Item::End(tick) => Some(format!("eof {tick}")),
        })
        .collect()
}

fn decode(config: &PwmConfig, block: &LogicBlock, gaps: &[TickRange]) -> Vec<String> {
    let mut harness = harness(config, GraphConfig::default());
    harness.feed_with_gaps(block, gaps, &[u64::MAX]);
    harness.end();
    lines(&harness.items)
}

/// Cycles of varying period and duty from tick 10, leading edges at 10, 110, 210, 290, 410
/// and 470; the cycle that begins at 470 is cut off by the end of the samples at 480.
fn varying(config: &PwmConfig) -> LogicBlock {
    let mut waveform = PwmWaveform::new(config, CHANNELS, Tick::ZERO);
    waveform.level(false, 10);
    waveform.cycles([(100, 25), (100, 50), (80, 10), (120, 119), (60, 1)]);
    waveform.level(true, 5);
    waveform.level(false, 5);
    waveform.finish()
}

fn varying_lines() -> Vec<String> {
    vec![
        "cycle 100/25 [10,110)@110".to_owned(),
        "cycle 100/50 [110,210)@210".to_owned(),
        "cycle 80/10 [210,290)@290".to_owned(),
        "cycle 120/119 [290,410)@410".to_owned(),
        "cycle 60/1 [410,470)@470".to_owned(),
        "eof 480".to_owned(),
    ]
}

#[test]
fn measures_cycles_of_varying_period_and_duty() {
    let config = PwmConfig {
        channel: 1,
        polarity: Polarity::ActiveHigh,
    };
    assert_eq!(decode(&config, &varying(&config), &[]), varying_lines());
}

#[test]
fn active_low_measures_the_low_time() {
    let config = PwmConfig {
        channel: 1,
        polarity: Polarity::ActiveLow,
    };
    let block = varying(&config);
    assert_eq!(decode(&config, &block, &[]), varying_lines());
    // Read as active high, the same line has the complementary cycles, from rising edge to
    // rising edge.
    let inverted = PwmConfig {
        polarity: Polarity::ActiveHigh,
        ..config
    };
    assert_eq!(
        decode(&inverted, &block, &[]),
        vec![
            "cycle 125/75 [35,160)@160",
            "cycle 60/50 [160,220)@220",
            "cycle 189/70 [220,409)@409",
            "cycle 2/1 [409,411)@411",
            "cycle 64/59 [411,475)@475",
            "eof 480"
        ]
    );
}

#[test]
fn the_first_cycle_begins_at_the_first_leading_edge() {
    let config = PwmConfig::default();
    // Active at the first sample, which is no edge: the trailing edge at 15 belongs to no
    // cycle, and the first one begins at 35.
    let mut waveform = PwmWaveform::new(&config, CHANNELS, Tick::ZERO);
    waveform.level(true, 15);
    waveform.level(false, 20);
    waveform.cycles([(30, 10), (30, 10)]);
    waveform.level(true, 5);
    let block = waveform.finish();
    assert_eq!(
        decode(&config, &block, &[]),
        vec![
            "cycle 30/10 [35,65)@65",
            "cycle 30/10 [65,95)@95",
            "eof 100"
        ]
    );
}

#[test]
fn a_gap_drops_the_cycle_it_cuts() {
    let config = PwmConfig::default();
    let mut waveform = PwmWaveform::new(&config, CHANNELS, Tick::ZERO);
    waveform.level(false, 10);
    waveform.cycles([(50, 20); 6]);
    waveform.level(true, 5);
    waveform.level(false, 5);
    let block = waveform.finish();
    // Leading edges at 10, 60, … 310. The first gap cuts the cycle from 110; the second cuts
    // the one from 160 and ends on the leading edge at 210, which is then no edge.
    let gaps = [
        TickRange::new(Tick(125), Tick(140)),
        TickRange::new(Tick(190), Tick(210)),
    ];
    assert_eq!(
        decode(&config, &block, &gaps),
        vec![
            "cycle 50/20 [10,60)@60",
            "cycle 50/20 [60,110)@110",
            "gap 125..140",
            "gap 190..210",
            "cycle 50/20 [260,310)@310",
            "eof 320"
        ]
    );
}

#[test]
fn the_end_of_the_samples_drops_the_cycle_under_way() {
    let config = PwmConfig::default();
    let mut waveform = PwmWaveform::new(&config, CHANNELS, Tick::ZERO);
    waveform.level(false, 10);
    waveform.cycles([(40, 10); 3]);
    let block = waveform.finish();
    assert_eq!(
        decode(&config, &block, &[]),
        vec![
            "cycle 40/10 [10,50)@50",
            "cycle 40/10 [50,90)@90",
            "eof 130"
        ]
    );
    // A line held at one level has no cycles at all.
    let mut waveform = PwmWaveform::new(&config, CHANNELS, Tick::ZERO);
    waveform.level(true, 100);
    assert_eq!(decode(&config, &waveform.finish(), &[]), vec!["eof 100"]);
}

#[test]
fn a_cycle_shows_up_at_the_next_leading_edge() {
    let config = PwmConfig::default();
    let mut waveform = PwmWaveform::new(&config, CHANNELS, Tick::ZERO);
    waveform.level(false, 10);
    waveform.cycles([(50, 20); 3]);
    let block = waveform.finish();
    let cycles = ["cycle 50/20 [10,60)@60", "cycle 50/20 [60,110)@110"];
    let mut harness = harness(&config, GraphConfig::default());
    let mut from = Tick::ZERO;
    // The samples arrive up to the leading edges at 60 and 110, and through each.
    for (cut, seen) in [(60, 0), (61, 1), (110, 1), (111, 2)] {
        let piece = block
            .slice(TickRange::new(from, Tick(cut)))
            .expect("a slice");
        harness.feed(&piece, &[u64::MAX]);
        harness.collect();
        assert_eq!(lines(&harness.items), cycles[..seen], "up to {cut}");
        from = Tick(cut);
    }
}

#[test]
fn progress_is_held_back_to_the_cycle_under_way() {
    let config = PwmConfig::default();
    let mut waveform = PwmWaveform::new(&config, CHANNELS, Tick::ZERO);
    waveform.level(false, 10);
    waveform.cycles([(50, 20); 3]);
    let block = waveform.finish();
    let mut harness = harness(&config, GraphConfig::default());
    let mut from = Tick::ZERO;
    // Nothing is under way before the leading edge at 10; from there on, the cycle of the
    // last leading edge seen, at 10, 60 and 110.
    for (cut, held) in [(5, 5), (40, 10), (61, 60), (110, 60), (111, 110)] {
        let piece = block
            .slice(TickRange::new(from, Tick(cut)))
            .expect("a slice");
        harness.feed(&piece, &[u64::MAX]);
        harness.collect();
        assert_eq!(
            harness.items.last(),
            Some(&Item::Progress(Tick(held))),
            "up to {cut}"
        );
        from = Tick(cut);
    }
    harness.assert_progress_promise();
}

/// A long, irregular pulse train on channel 1, with channel 0 busy too.
fn irregular() -> LogicBlock {
    let mut builder = LogicBlockBuilder::new(Tick::ZERO, CHANNELS);
    for i in 0..300u64 {
        let period = (i * 53 + 7) % 40 + 2;
        let high = (i * 29 + 3) % period;
        let noise = i & 1;
        builder.push_repeated(Levels(0b10 | noise), high);
        builder.push_repeated(Levels(noise ^ 1), period - high);
    }
    builder.finish().expect("a block")
}

/// What the decoder must report, found sample by sample.
fn naive(config: &PwmConfig, block: &LogicBlock, gaps: &[TickRange]) -> Vec<String> {
    let mut lines = Vec::new();
    let mut was_active: Option<bool> = None;
    let mut leading: Option<Tick> = None;
    let mut trailing: Option<Tick> = None;
    let mut tick = block.start();
    while tick < block.end() {
        if let Some(gap) = gaps.iter().find(|gap| gap.contains(tick)) {
            lines.push(format!("gap {gap}"));
            was_active = None;
            leading = None;
            trailing = None;
            tick = gap.end;
            continue;
        }
        let high = block.level(config.channel, tick).expect("in the block");
        let active = config.polarity.is_active(high);
        match was_active {
            Some(false) if active => {
                if let (Some(start), Some(end)) = (leading, trailing) {
                    lines.push(cycle(start.until(tick), start.until(end), start, tick));
                }
                leading = Some(tick);
                trailing = None;
            }
            Some(true) if !active => trailing = Some(tick),
            _ => {}
        }
        was_active = Some(active);
        tick = tick.next();
    }
    lines.push(format!("eof {}", block.end()));
    lines
}

#[test]
fn cycles_match_a_sample_by_sample_scan() {
    let block = irregular();
    let gaps = [
        TickRange::new(Tick(500), Tick(530)),
        TickRange::new(Tick(2047), Tick(2048)),
        TickRange::new(Tick(4000), Tick(4100)),
    ];
    for polarity in [Polarity::ActiveHigh, Polarity::ActiveLow] {
        let config = PwmConfig {
            channel: 1,
            polarity,
        };
        let decoded = decode(&config, &block, &gaps);
        assert!(decoded.len() > 100, "{polarity:?}: {decoded:?}");
        assert_eq!(decoded, naive(&config, &block, &gaps), "{polarity:?}");
    }
}

#[test]
fn blocks_pauses_and_gaps_do_not_change_the_cycles() {
    let block = irregular();
    let gaps = [
        TickRange::new(Tick(127), Tick(128)),
        TickRange::new(Tick(1000), Tick(1090)),
        TickRange::new(Tick(3333), Tick(3500)),
    ];
    for polarity in [Polarity::ActiveHigh, Polarity::ActiveLow] {
        let config = PwmConfig {
            channel: 1,
            polarity,
        };
        assert_block_invariant(|graph| harness(&config, graph), &block, &gaps);
    }
}
