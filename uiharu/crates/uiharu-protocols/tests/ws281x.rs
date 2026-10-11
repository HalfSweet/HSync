// SPDX-License-Identifier: Apache-2.0

//! The WS281x decoder on synthesized LED data.

use proptest::prelude::*;
use uiharu_core::testing::{Harness, assert_block_invariant, without_progress};
use uiharu_core::{GraphConfig, GraphError, Item, LogicBlock, Tick, TickRange, TimeBase};
use uiharu_protocols::ws281x::synth::{BitTiming, Ws281xWaveform};
use uiharu_protocols::ws281x::{Ws281x, Ws281xConfig, Ws281xEvent};

const CHANNELS: u8 = 2;
/// 20 MHz: a bit of 1.25 µs is 25 ticks, highs of 8 and 16, the threshold 12.5 ticks, the reset
/// 1000 ticks.
const RATE: u64 = 20_000_000;
const BIT: u64 = 25;
const RESET: u64 = 1000;

/// RGB LEDs on channel 1.
fn config() -> Ws281xConfig {
    Ws281xConfig {
        data: 1,
        ..Ws281xConfig::default()
    }
}

fn rgbw() -> Ws281xConfig {
    Ws281xConfig {
        bits_per_led: 32,
        ..config()
    }
}

fn try_build(
    config: &Ws281xConfig,
    rate: u64,
    graph_config: GraphConfig,
) -> Result<Harness<Ws281xEvent>, GraphError> {
    Harness::build(
        CHANNELS,
        TimeBase::new(rate),
        graph_config,
        |graph, source| {
            let leds = graph.add(Ws281x::new(config.clone()).expect("a valid configuration"));
            graph.connect(source.samples, Ws281x::input(leds))?;
            Ok(Ws281x::events(leds))
        },
    )
}

fn harness(config: &Ws281xConfig, rate: u64, graph_config: GraphConfig) -> Harness<Ws281xEvent> {
    try_build(config, rate, graph_config).expect("builds")
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn at(span: TickRange, settled: Tick) -> String {
    if span.is_empty() {
        format!("@{settled}")
    } else {
        format!("[{},{})@{settled}", span.start, span.end)
    }
}

/// One line per item, readable in assertions.
fn describe(items: &[Item<Ws281xEvent>]) -> Vec<String> {
    items
        .iter()
        .map(|item| match item {
            Item::Event(event) => {
                let what = match &event.payload {
                    Ws281xEvent::FrameBegin { frame } => format!("frame-begin f{}", frame.0),
                    Ws281xEvent::Led {
                        frame,
                        index,
                        bytes,
                    } => format!("led f{} #{index} {}", frame.0, hex(bytes)),
                    Ws281xEvent::FrameEnd {
                        frame,
                        leds,
                        reason,
                    } => format!("frame-end f{} {leds} {reason:?}", frame.0),
                    Ws281xEvent::Error {
                        frame,
                        reason,
                        bits,
                    } => format!("error f{} {reason:?} {bits}", frame.0),
                };
                format!("{what} {}", at(event.span, event.settled))
            }
            Item::Gap(range) => format!("gap {range}"),
            Item::Progress(tick) => format!("progress {tick}"),
            Item::End(tick) => format!("eof {tick}"),
        })
        .collect()
}

/// Decodes `block` less `gaps` in one piece: the lines, progress left out.
fn decode_with(
    config: &Ws281xConfig,
    rate: u64,
    block: &LogicBlock,
    gaps: &[TickRange],
) -> Vec<String> {
    let mut harness = harness(config, rate, GraphConfig::default());
    harness.feed_with_gaps(block, gaps, &[u64::MAX]);
    harness.end();
    describe(&harness.without_progress())
}

fn decode(config: &Ws281xConfig, block: &LogicBlock) -> Vec<String> {
    decode_with(config, RATE, block, &[])
}

/// A waveform at the typical timing, low until `idle`.
fn waveform(config: &Ws281xConfig, idle: u64) -> Ws281xWaveform {
    let mut waveform = Ws281xWaveform::new(config, CHANNELS, BitTiming::typical(RATE), Tick::ZERO);
    waveform.low(idle);
    waveform
}

/// Where bit `k` of a frame sent from `start` at the typical timing rises.
fn rise(start: u64, k: u64) -> u64 {
    start + k * BIT
}

/// Where bit `k` falls, given its value.
fn fall(start: u64, k: u64, one: bool) -> u64 {
    rise(start, k) + if one { 16 } else { 8 }
}

/// The line of LED `index` of a frame sent from `start`, `bits` bits per LED, whose last bit
/// is `last_one`: it spans whole bit times and settles at its last bit's fall.
fn led(start: u64, index: u64, bits: u64, bytes: &str, last_one: bool) -> String {
    let first = rise(start, index * bits);
    let last = index * bits + bits - 1;
    format!(
        "led f1 #{index} {bytes} [{first},{})@{}",
        rise(start, last + 1),
        fall(start, last, last_one)
    )
}

#[test]
fn a_frame_of_three_leds() {
    let config = config();
    let mut waveform = waveform(&config, 1200);
    waveform.bytes([0xff, 0x00, 0x10, 0x12, 0x34, 0x56, 0x80, 0x01, 0xa5]);
    waveform.low(1500);
    let block = waveform.finish();
    // The last bit, a 1, rises at 1200 + 71 × 25 and falls 16 ticks later; the reset passes
    // 1000 ticks after that.
    let reset = fall(1200, 71, true) + RESET;
    assert_eq!(
        decode(&config, &block),
        vec![
            "frame-begin f1 [1200,1201)@1200".to_owned(),
            led(1200, 0, 24, "ff0010", false),
            led(1200, 1, 24, "123456", false),
            led(1200, 2, 24, "8001a5", true),
            format!("frame-end f1 3 Reset @{reset}"),
            "eof 4500".to_owned(),
        ]
    );
    assert_eq!(reset, 3991);
}

#[test]
fn rgbw_leds_take_four_bytes() {
    let rgbw = rgbw();
    let mut waveform = waveform(&rgbw, 1100);
    waveform.bytes([0x11, 0x22, 0x33, 0x44, 0xfe, 0xdc, 0xba, 0x99]);
    waveform.low(1100);
    let block = waveform.finish();
    let reset = fall(1100, 63, true) + RESET;
    assert_eq!(
        decode(&rgbw, &block),
        vec![
            "frame-begin f1 [1100,1101)@1100".to_owned(),
            led(1100, 0, 32, "11223344", false),
            led(1100, 1, 32, "fedcba99", true),
            format!("frame-end f1 2 Reset @{reset}"),
            format!("eof {}", block.end()),
        ]
    );
    // Read as RGB, the same eight bytes are two LEDs and sixteen bits left over.
    let lines = decode(&config(), &block);
    assert_eq!(lines[1], led(1100, 0, 24, "112233", true));
    assert_eq!(lines[2], led(1100, 1, 24, "44fedc", false));
    assert_eq!(
        lines[3],
        format!(
            "error f1 PartialLed 16 [{},{})@{reset}",
            rise(1100, 48),
            rise(1100, 64)
        )
    );
    assert_eq!(lines[4], format!("frame-end f1 2 Reset @{reset}"));
}

#[test]
fn two_frames_each_latched_by_a_reset() {
    let config = config();
    let mut waveform = waveform(&config, 1000);
    waveform.bytes([0x01, 0x02, 0x03, 0x04, 0x05, 0x06]);
    let first_end = waveform.position().0;
    waveform.low(1300);
    let second = waveform.position().0;
    waveform.bytes([0xaa, 0xbb, 0xcc]);
    waveform.low(1000);
    let block = waveform.finish();
    // Frame 1's last bit, a 0, falls 8 ticks into its 25.
    let first_reset = first_end - BIT + 8 + RESET;
    let second_reset = second + 23 * BIT + 8 + RESET;
    let lines = decode(&config, &block);
    assert_eq!(
        lines,
        vec![
            "frame-begin f1 [1000,1001)@1000".to_owned(),
            led(1000, 0, 24, "010203", true),
            led(1000, 1, 24, "040506", false),
            format!("frame-end f1 2 Reset @{first_reset}"),
            format!("frame-begin f2 [{second},{})@{second}", second + 1),
            format!(
                "led f2 #0 aabbcc [{second},{})@{}",
                second + 24 * BIT,
                second + 23 * BIT + 8
            ),
            format!("frame-end f2 1 Reset @{second_reset}"),
            format!("eof {}", block.end()),
        ]
    );
}

#[test]
fn the_threshold_decides_by_the_high_time_alone() {
    // 16 MHz: 625 ns are exactly 10 ticks; a high of 10 ticks is not longer, 11 is.
    let rate = 16_000_000;
    let config = config();
    let mut waveform = Ws281xWaveform::new(&config, CHANNELS, BitTiming::typical(rate), Tick::ZERO);
    waveform.low(900);
    for i in 0..24 {
        // Lows as long or as short as the senders make them: only the high counts.
        let low = if i % 3 == 0 { 2 } else { 30 };
        if i % 2 == 0 {
            waveform.pulse(10, low);
        } else {
            waveform.pulse(11, low);
        }
    }
    // A pulse of one tick is a 0, one of 100 ticks a 1; then 22 more bits.
    waveform.pulse(1, 9);
    waveform.pulse(100, 9);
    waveform.bits(0x3f_ffff, 22);
    waveform.low(900);
    let block = waveform.finish();
    let leds: Vec<String> = decode_with(&config, rate, &block, &[])
        .into_iter()
        .filter(|line| line.starts_with("led "))
        .map(|line| line.split(' ').take(4).collect::<Vec<_>>().join(" "))
        .collect();
    assert_eq!(leds, vec!["led f1 #0 555555", "led f1 #1 7fffff"]);
}

#[test]
fn a_low_reaching_the_reset_time_ends_the_frame() {
    let config = config();
    for (low, frames) in [(RESET - 1, 1), (RESET, 2)] {
        let mut waveform = waveform(&config, 1000);
        waveform.bytes([0x00, 0x00]);
        // The 16th bit is a 0: high for 8 ticks, then low for `low` ticks before bit 17.
        waveform.bit(false);
        waveform.low(low - 17);
        waveform.bytes([0xf0; 6]);
        waveform.low(1100);
        let block = waveform.finish();
        let lines = decode(&config, &block);
        let begins = lines
            .iter()
            .filter(|line| line.starts_with("frame-begin"))
            .count();
        assert_eq!(begins, frames, "a low of {low} ticks: {lines:?}");
        if frames == 2 {
            // The first frame's 17 bits are no LED; it ends where the low reached the reset.
            let fall = 1000 + 16 * BIT + 8;
            assert_eq!(
                lines[1],
                format!(
                    "error f1 PartialLed 17 [1000,{})@{}",
                    1000 + 17 * BIT,
                    fall + RESET
                )
            );
            assert_eq!(lines[2], format!("frame-end f1 0 Reset @{}", fall + RESET));
            assert_eq!(
                lines[3],
                format!(
                    "frame-begin f2 [{},{})@{}",
                    fall + RESET,
                    fall + RESET + 1,
                    fall + RESET
                )
            );
        } else {
            // One frame of 65 bits: two LEDs, 17 bits left over.
            assert!(
                lines
                    .iter()
                    .any(|line| line.starts_with("error f1 PartialLed 17 ")),
                "{lines:?}"
            );
        }
    }
}

#[test]
fn bits_left_over_at_the_reset_are_reported() {
    let config = config();
    let mut waveform = waveform(&config, 1000);
    waveform.bytes([0x10, 0x20, 0x30]);
    waveform.bits(0b10110, 5);
    waveform.low(1200);
    let block = waveform.finish();
    let last_fall = fall(1000, 28, false);
    assert_eq!(
        decode(&config, &block),
        vec![
            "frame-begin f1 [1000,1001)@1000".to_owned(),
            led(1000, 0, 24, "102030", false),
            format!(
                "error f1 PartialLed 5 [{},{})@{}",
                rise(1000, 24),
                rise(1000, 29),
                last_fall + RESET
            ),
            format!("frame-end f1 1 Reset @{}", last_fall + RESET),
            format!("eof {}", block.end()),
        ]
    );
}

#[test]
fn the_end_of_the_samples_aborts_the_frame() {
    let config = config();
    let mut waveform = waveform(&config, 1000);
    waveform.bytes([0x01, 0x02, 0x03, 0x04, 0x05, 0x06]);
    waveform.bits(0x3ff, 10);
    waveform.low(400);
    let block = waveform.finish();
    let end = block.end().0;
    assert_eq!(
        decode(&config, &block),
        vec![
            "frame-begin f1 [1000,1001)@1000".to_owned(),
            led(1000, 0, 24, "010203", true),
            led(1000, 1, 24, "040506", false),
            format!("frame-end f1 2 Aborted @{end}"),
            format!("eof {end}"),
        ]
    );
    // Samples that reach the reset tick end the frame as usual, even with nothing after it.
    let reset = fall(1000, 47, false) + RESET;
    let mut whole = waveform_bytes(&config, 1000, &[0x01, 0x02, 0x03, 0x04, 0x05, 0x06]);
    whole.low(reset - whole.position().0);
    let block = whole.finish();
    let lines = decode(&config, &block);
    assert_eq!(lines[3], format!("frame-end f1 2 Reset @{reset}"));
    assert_eq!(lines[4], format!("eof {reset}"));
}

fn waveform_bytes(config: &Ws281xConfig, idle: u64, bytes: &[u8]) -> Ws281xWaveform {
    let mut waveform = waveform(config, idle);
    waveform.bytes(bytes.iter().copied());
    waveform
}

#[test]
fn a_gap_aborts_the_frame_and_the_next_one_waits_for_a_reset() {
    let config = config();
    let mut waveform = waveform(&config, 1000);
    waveform.bytes([0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09]);
    waveform.low(1200);
    let second = waveform.position().0;
    waveform.bytes([0x0a, 0x0b, 0x0c]);
    waveform.low(1200);
    let block = waveform.finish();
    // Inside LED 1: what follows the gap is the middle of the frame, which is skipped.
    let gap = TickRange::new(Tick(1000 + 30 * BIT + 3), Tick(1000 + 40 * BIT));
    let lines = decode_with(&config, RATE, &block, &[gap]);
    assert_eq!(
        lines,
        vec![
            "frame-begin f1 [1000,1001)@1000".to_owned(),
            led(1000, 0, 24, "010203", true),
            format!("frame-end f1 1 Aborted @{}", gap.start),
            format!("gap {gap}"),
            format!("frame-begin f2 [{second},{})@{second}", second + 1),
            format!(
                "led f2 #0 0a0b0c [{second},{})@{}",
                second + 24 * BIT,
                second + 23 * BIT + 8
            ),
            format!("frame-end f2 1 Reset @{}", second + 23 * BIT + 8 + RESET),
            format!("eof {}", block.end()),
        ]
    );
    // A gap inside the reset, after the low reached it, leaves the frame whole.
    let reset = fall(1000, 71, true) + RESET;
    let late = TickRange::new(Tick(reset), Tick(reset + 50));
    let lines = decode_with(&config, RATE, &block, &[late]);
    assert_eq!(lines[4], format!("frame-end f1 3 Reset @{reset}"));
    assert_eq!(lines[5], format!("gap {late}"));
    // One tick earlier, it cuts the reset short.
    let early = TickRange::new(Tick(reset - 1), Tick(reset + 50));
    let lines = decode_with(&config, RATE, &block, &[early]);
    assert_eq!(lines[4], format!("frame-end f1 3 Aborted @{}", reset - 1));
}

#[test]
fn a_frame_under_way_at_the_first_sample_is_skipped() {
    let config = config();
    let mut waveform = waveform(&config, 1000);
    waveform.bytes([0x01, 0x02, 0x03, 0x04, 0x05, 0x06]);
    waveform.low(1100);
    let second = waveform.position().0;
    waveform.bytes([0x0a, 0x0b, 0x0c]);
    waveform.low(1100);
    let block = waveform.finish();
    // From inside bit 3's high, and from inside bit 3's low: either way the first frame is
    // skipped and the second, after a reset, is read.
    for from in [1000 + 3 * BIT + 2, 1000 + 3 * BIT + 12] {
        let tail = block
            .slice(TickRange::new(Tick(from), block.end()))
            .expect("a slice")
            .rebased(Tick::ZERO);
        let lines = decode(&config, &tail);
        let start = second - from;
        assert_eq!(
            lines[0],
            format!("frame-begin f1 [{start},{})@{start}", start + 1)
        );
        assert!(lines[1].starts_with("led f1 #0 0a0b0c "), "{lines:?}");
        assert_eq!(lines.len(), 4, "{lines:?}");
    }
    // A line low from the first sample for less than the reset time does not count as one:
    // the frame that rises after 999 ticks is skipped too.
    let short = block
        .slice(TickRange::new(Tick(1), block.end()))
        .expect("a slice")
        .rebased(Tick::ZERO);
    let start = second - 1;
    assert_eq!(
        decode(&config, &short)[0],
        format!("frame-begin f1 [{start},{})@{start}", start + 1)
    );
    // Low for the reset time from the first sample, it does.
    let enough = block
        .slice(TickRange::new(Tick(0), block.end()))
        .expect("a slice");
    assert_eq!(
        decode(&config, &enough)[0],
        "frame-begin f1 [1000,1001)@1000"
    );
}

#[test]
fn an_led_shows_up_at_its_last_falling_edge() {
    let config = config();
    let mut waveform = waveform(&config, 1000);
    waveform.bytes([0x01, 0x02, 0x03, 0x04, 0x05, 0x06]);
    waveform.low(1200);
    let block = waveform.finish();
    let first_led_settled = fall(1000, 23, true);
    let mut harness = harness(&config, RATE, GraphConfig::default());
    let mut from = Tick::ZERO;
    for (cut, seen) in [(first_led_settled, 1), (first_led_settled + 1, 2)] {
        let piece = block
            .slice(TickRange::new(from, Tick(cut)))
            .expect("a slice");
        harness.feed(&piece, &[u64::MAX]);
        harness.collect();
        let lines = describe(&harness.without_progress());
        assert_eq!(lines.len(), seen, "up to {cut}: {lines:?}");
        from = Tick(cut);
    }
}

#[test]
fn progress_never_passes_an_led_under_way() {
    let config = config();
    let block = rich(&config);
    for sizes in [&[1u64][..], &[13], &[64, 7], &[500]] {
        let mut harness = harness(&config, RATE, GraphConfig::default());
        harness.feed_cut(&block, &[], sizes, &[1]);
        harness.end();
        let mut seen = Tick::ZERO;
        let mut progressed = false;
        for item in &harness.items {
            match item {
                Item::Progress(tick) => {
                    seen = seen.max(*tick);
                    progressed = true;
                }
                Item::Event(event) => {
                    assert!(event.span.start >= seen, "{event:?} after {seen}");
                }
                Item::Gap(_) | Item::End(_) => {}
            }
        }
        assert!(progressed, "blocks of {sizes:?}");
    }
}

#[test]
fn a_source_too_slow_for_the_threshold_is_refused() {
    let config = config();
    assert!(try_build(&config, 3_200_000, GraphConfig::default()).is_ok());
    let error = try_build(&config, 3_199_999, GraphConfig::default()).expect_err("refused");
    assert_eq!(
        error,
        GraphError::Requirement {
            from: "logic#0.samples".into(),
            to: "ws281x#0.logic".into(),
            key: "rateHz",
            requirement: ">= 3200000".into(),
            found: "3199999".into(),
        }
    );
    let wide = Ws281xConfig {
        data: CHANNELS,
        ..config
    };
    assert!(matches!(
        try_build(&wide, RATE, GraphConfig::default()).err(),
        Some(GraphError::Requirement {
            key: "channels",
            ..
        })
    ));
}

/// Frames of every kind on channel 1, channel 0 busy as well: RGB frames with varying lows,
/// a frame with bits left over, slow bits, and long idle times.
fn rich(config: &Ws281xConfig) -> LogicBlock {
    let mut waveform = waveform(config, 1100);
    for n in 0..6u8 {
        let leds = 1 + n % 3;
        for i in 0..leds * 3 {
            let byte = n.wrapping_mul(37).wrapping_add(i).rotate_left(u32::from(n));
            for shift in (0..8u8).rev() {
                let one = byte >> shift & 1 != 0;
                let (high, low) = if one { (16, 9) } else { (8, 17) };
                // Some bits take their time: a low of up to most of the reset.
                let low = match (i + shift) % 11 {
                    0 => low + 300,
                    5 => low + 40,
                    _ => low,
                };
                waveform.pulse(high, low);
            }
        }
        if n == 4 {
            waveform.bits(0b1011, 4);
        }
        waveform.low(RESET + 50 * u64::from(n));
    }
    let block = waveform.finish();
    // Noise on channel 0 changes nothing.
    let mut builder = uiharu_core::LogicBlockBuilder::new(block.start(), CHANNELS);
    for tick in block.start().0..block.end().0 {
        let levels = block.levels(Tick(tick)).expect("in the block");
        builder.push(uiharu_core::Levels(levels.0 | (tick / 7 % 2)));
    }
    builder.finish().expect("a block")
}

#[test]
fn rich_traffic_decodes_whole() {
    let config = config();
    let lines = decode(&config, &rich(&config));
    let ends: Vec<&String> = lines
        .iter()
        .filter(|line| line.starts_with("frame-end"))
        .collect();
    assert_eq!(ends.len(), 6, "{lines:?}");
    assert!(
        ends.iter().all(|line| line.contains(" Reset ")),
        "{lines:?}"
    );
    assert_eq!(
        lines
            .iter()
            .filter(|line| line.starts_with("error f5 PartialLed 4 "))
            .count(),
        1,
        "{lines:?}"
    );
}

#[test]
fn blocks_pauses_and_gaps_do_not_change_the_frames() {
    let config = config();
    let block = rich(&config);
    let len = block.len();
    // In the idle time before the first frame, inside a frame's bit, inside a reset, and from
    // a bit's high into the next.
    let gaps = [
        TickRange::new(Tick(300), Tick(340)),
        TickRange::new(Tick(len / 3), Tick(len / 3 + 7)),
        TickRange::new(Tick(len / 2), Tick(len / 2 + 900)),
        TickRange::new(Tick(4 * len / 5), Tick(4 * len / 5 + 13)),
    ];
    let reference = decode_with(&config, RATE, &block, &gaps);
    assert!(
        reference.iter().any(|line| line.contains(" Aborted ")),
        "{reference:?}"
    );
    assert_block_invariant(|graph| harness(&config, RATE, graph), &block, &gaps);
    let rgbw = rgbw();
    assert_block_invariant(|graph| harness(&rgbw, RATE, graph), &block, &gaps);
}

/// The bits of an LED under way in the reference model: value, rising and falling edge.
type ModelBits = Vec<(bool, u64, u64)>;

/// What the reference model is doing.
enum Model {
    /// Looking for a reset; the line has been low since this tick, if it is low.
    Hunting(Option<u64>),
    /// After a reset, waiting for a frame.
    Idle,
    /// In a frame.
    Frame {
        id: u64,
        leds: u32,
        bits: ModelBits,
        /// The rising edge of the bit whose high is under way.
        high_since: Option<u64>,
        last_fall: u64,
    },
}

/// The span of `bits` as the decoder's documentation defines it.
fn model_span(bits: &ModelBits) -> (u64, u64) {
    let first = bits[0].1;
    let (_, last_rise, last_fall) = bits[bits.len() - 1];
    let end = if bits.len() > 1 {
        let shortest = bits
            .windows(2)
            .map(|pair| pair[1].1 - pair[0].1)
            .min()
            .expect("two bits");
        (last_rise + shortest).max(last_fall + 1)
    } else {
        last_fall + 1
    };
    (first, end)
}

/// The decoder's rules, applied sample by sample: what it must report for `block` less `gaps`.
fn naive(config: &Ws281xConfig, rate: u64, block: &LogicBlock, gaps: &[TickRange]) -> Vec<String> {
    let one_from = u64::from(config.threshold_ns) * rate / 1_000_000_000 + 1;
    let reset = (u64::from(config.reset_us) * rate)
        .div_ceil(1_000_000)
        .max(1);
    let per_led = usize::from(config.bits_per_led);
    let mut lines = Vec::new();
    let mut model = Model::Hunting(None);
    let mut next_id = 1;
    // Whether a deadline has passed at `tick`, before the level there is looked at.
    let deadline = |model: &mut Model, tick: u64, lines: &mut Vec<String>| match model {
        Model::Hunting(Some(since)) if tick >= *since + reset => *model = Model::Idle,
        Model::Frame {
            id,
            leds,
            bits,
            high_since: None,
            last_fall,
        } if tick >= *last_fall + reset => {
            let at = *last_fall + reset;
            if !bits.is_empty() {
                let (start, end) = model_span(bits);
                lines.push(format!(
                    "error f{id} PartialLed {} [{start},{end})@{at}",
                    bits.len()
                ));
            }
            lines.push(format!("frame-end f{id} {leds} Reset @{at}"));
            *model = Model::Idle;
        }
        _ => {}
    };
    let mut previous: Option<bool> = None;
    let mut tick = block.start().0;
    let end = block.end().0;
    while tick < end {
        deadline(&mut model, tick, &mut lines);
        if let Some(gap) = gaps.iter().find(|gap| gap.contains(Tick(tick))) {
            if let Model::Frame { id, leds, .. } = model {
                lines.push(format!("frame-end f{id} {leds} Aborted @{tick}"));
            }
            lines.push(format!("gap {gap}"));
            model = Model::Hunting(None);
            previous = None;
            tick = gap.end.0;
            continue;
        }
        let high = block.level(config.data, Tick(tick)).expect("in the block");
        match previous {
            // The first sample, and the first after a gap, are no edge.
            None => {
                if let Model::Hunting(since) = &mut model {
                    *since = (!high).then_some(tick);
                }
            }
            Some(was) if was != high => match &mut model {
                Model::Hunting(since) => *since = (!high).then_some(tick),
                Model::Idle => {
                    lines.push(format!(
                        "frame-begin f{next_id} [{tick},{})@{tick}",
                        tick + 1
                    ));
                    model = Model::Frame {
                        id: next_id,
                        leds: 0,
                        bits: Vec::new(),
                        high_since: Some(tick),
                        last_fall: tick,
                    };
                    next_id += 1;
                }
                Model::Frame {
                    id,
                    leds,
                    bits,
                    high_since,
                    last_fall,
                } => {
                    if high {
                        *high_since = Some(tick);
                    } else if let Some(rise) = high_since.take() {
                        bits.push((tick - rise >= one_from, rise, tick));
                        *last_fall = tick;
                        if bits.len() == per_led {
                            let (start, end) = model_span(bits);
                            let bytes: String = bits
                                .chunks(8)
                                .map(|byte| {
                                    let value = byte
                                        .iter()
                                        .fold(0u8, |value, bit| (value << 1) | u8::from(bit.0));
                                    format!("{value:02x}")
                                })
                                .collect();
                            lines.push(format!("led f{id} #{leds} {bytes} [{start},{end})@{tick}"));
                            *leds += 1;
                            bits.clear();
                        }
                    }
                }
            },
            Some(_) => {}
        }
        previous = Some(high);
        tick += 1;
    }
    deadline(&mut model, end, &mut lines);
    if let Model::Frame { id, leds, .. } = model {
        lines.push(format!("frame-end f{id} {leds} Aborted @{end}"));
    }
    lines.push(format!("eof {end}"));
    lines
}

#[test]
fn the_reference_model_agrees_on_the_hand_made_cases() {
    let config = config();
    let block = rich(&config);
    let gaps = [
        TickRange::new(Tick(300), Tick(340)),
        TickRange::new(Tick(9000), Tick(9013)),
        TickRange::new(Tick(15000), Tick(15900)),
    ];
    assert_eq!(
        decode_with(&config, RATE, &block, &gaps),
        naive(&config, RATE, &block, &gaps)
    );
    assert_eq!(decode(&config, &block), naive(&config, RATE, &block, &[]));
    let rgbw = rgbw();
    assert_eq!(decode(&rgbw, &block), naive(&rgbw, RATE, &block, &[]));
}

/// A burst of bits on the line, each a high and a low in ticks, and the low after it.
#[derive(Clone, Debug)]
struct Burst {
    bits: Vec<(u64, u64)>,
    idle: u64,
}

fn burst() -> impl Strategy<Value = Burst> {
    let bit = (
        prop_oneof![4 => 6u64..11, 4 => 14u64..19, 1 => 1u64..30],
        prop_oneof![8 => 3u64..40, 1 => 990u64..1010, 1 => 40u64..900],
    );
    (
        prop::collection::vec(bit, 0..60),
        prop_oneof![2 => 1000u64..1500, 1 => 0u64..1000],
    )
        .prop_map(|(bits, idle)| Burst { bits, idle })
}

fn bursts_waveform(lead: (u64, u64), bursts: &[Burst]) -> LogicBlock {
    let config = config();
    let mut waveform = Ws281xWaveform::new(&config, CHANNELS, BitTiming::typical(RATE), Tick::ZERO);
    waveform.high(lead.0);
    waveform.low(lead.1.max(1));
    for burst in bursts {
        for (high, low) in &burst.bits {
            waveform.pulse(*high, *low);
        }
        waveform.low(burst.idle);
    }
    waveform.finish()
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 40, ..ProptestConfig::default() })]

    /// Random pulses read in blocks of any size, by a reader that pauses, through tiny queues
    /// and budgets, give what the sample-by-sample model gives.
    #[test]
    fn random_pulses_decode_as_the_model_says(
        lead in (0u64..30, 0u64..1500),
        bursts in prop::collection::vec(burst(), 1..6),
        rgbw in any::<bool>(),
        sizes in prop::collection::vec(1u64..400, 1..6),
        pauses in prop::collection::vec(0usize..12, 1..8),
        queue_capacity in 1usize..6,
        budget in 1u32..40,
        gap in prop::option::of((0.05f64..0.95, 1u64..1200)),
    ) {
        let config = Ws281xConfig {
            bits_per_led: if rgbw { 32 } else { 24 },
            ..config()
        };
        let block = bursts_waveform(lead, &bursts);
        let gaps: Vec<TickRange> = gap
            .and_then(|(at, len)| {
                let start = ((block.len() as f64) * at) as u64;
                let end = (start + len).min(block.len());
                (start < end).then(|| TickRange::new(Tick(start), Tick(end)))
            })
            .into_iter()
            .collect();
        let reference = decode_with(&config, RATE, &block, &gaps);
        prop_assert_eq!(&reference, &naive(&config, RATE, &block, &gaps));

        let mut harness = harness(&config, RATE, GraphConfig { queue_capacity, budget });
        harness.drain_on_full = 1;
        harness.feed_cut(&block, &gaps, &sizes, &pauses);
        harness.end();
        prop_assert_eq!(describe(&without_progress(&harness.items)), reference);
        let stats = harness.pipeline().stats();
        prop_assert!(stats.within_capacity(), "{:?}", stats);
    }
}
