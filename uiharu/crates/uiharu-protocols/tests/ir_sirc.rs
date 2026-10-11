// SPDX-License-Identifier: Apache-2.0

//! The SIRC infrared decoder on synthesized receiver output.

use proptest::prelude::*;
use uiharu_core::testing::{Harness, assert_block_invariant};
use uiharu_core::{GraphConfig, GraphError, Item, LogicBlock, Tick, TickRange, TimeBase};
use uiharu_protocols::ir_sirc::synth::{
    Element, REPEAT_PERIOD_NS, START_MARK_NS, SircWaveform, UNIT_NS, elements, frame_bits,
    frame_elements,
};
use uiharu_protocols::ir_sirc::{ErrorReason, Polarity, Sirc, SircConfig, SircEvent};

const CHANNELS: u8 = 2;
/// The rate of most tests: 40 kHz, 24 ticks to the 600 µs unit, so that every nominal length
/// is a whole number of ticks.
const RATE: u64 = 40_000;
/// A tick at [`RATE`], in nanoseconds.
const TICK_NS: u64 = 25_000;
/// Ticks to the unit at [`RATE`].
const UNIT: u64 = 24;

/// The receiver on channel 1, active low, 25% tolerance.
fn config() -> SircConfig {
    SircConfig {
        channel: 1,
        ..SircConfig::default()
    }
}

fn try_build(
    config: SircConfig,
    rate: u64,
    graph_config: GraphConfig,
) -> Result<Harness<SircEvent>, GraphError> {
    Harness::build(
        CHANNELS,
        TimeBase::new(rate),
        graph_config,
        |graph, source| {
            let sirc = graph.add(Sirc::new(config).expect("a valid configuration"));
            graph.connect(source.samples, Sirc::input(sirc))?;
            Ok(Sirc::events(sirc))
        },
    )
}

fn build(config: SircConfig, rate: u64, graph_config: GraphConfig) -> Harness<SircEvent> {
    try_build(config, rate, graph_config).expect("builds")
}

fn reason(reason: ErrorReason) -> &'static str {
    match reason {
        ErrorReason::Start => "start",
        ErrorReason::Bit => "bit",
        ErrorReason::Count => "count",
    }
}

/// One line per item, readable in assertions.
fn describe(items: &[Item<SircEvent>]) -> Vec<String> {
    items
        .iter()
        .map(|item| match item {
            Item::Event(event) => {
                let what = match event.payload {
                    SircEvent::Frame {
                        bits,
                        command,
                        address,
                        extended,
                    } => {
                        let extended = extended.map_or(String::new(), |value| format!(":{value}"));
                        format!("frame {bits} {command}:{address}{extended}")
                    }
                    SircEvent::Error {
                        reason: why,
                        bits,
                        raw,
                    } => format!("error {} {bits} {raw:x}", reason(why)),
                };
                format!(
                    "{what} [{},{})@{}",
                    event.span.start, event.span.end, event.settled
                )
            }
            Item::Gap(range) => format!("gap {range}"),
            Item::Progress(tick) => format!("progress {tick}"),
            Item::End(tick) => format!("eof {tick}"),
        })
        .collect()
}

/// Drops a progress line that repeats the one before it: whether two progress items fold in
/// the queue depends on when the reader ran.
fn folded(lines: Vec<String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for line in lines {
        if line.starts_with("progress ") && out.last() == Some(&line) {
            continue;
        }
        out.push(line);
    }
    out
}

/// Decodes `block` in one piece: the lines, progress left out.
fn decode(config: SircConfig, rate: u64, block: &LogicBlock) -> Vec<String> {
    let mut harness = build(config, rate, GraphConfig::default());
    harness.run_all(block);
    describe(&harness.without_progress())
}

/// The lines of `block` without their placement.
fn kinds(config: SircConfig, rate: u64, block: &LogicBlock) -> Vec<String> {
    decode(config, rate, block)
        .into_iter()
        .map(|line| line.split(" [").next().unwrap_or_default().to_owned())
        .collect()
}

fn mark(ticks: u64) -> Element {
    Element::mark(ticks * TICK_NS)
}

fn space(ticks: u64) -> Element {
    Element::space(ticks * TICK_NS)
}

/// The waveform of `config` at [`RATE`], idle for 10 ms: 400 ticks.
fn waveform(config: &SircConfig) -> SircWaveform {
    let mut waveform = SircWaveform::new(config, CHANNELS, RATE, Tick::ZERO);
    waveform.idle(10_000_000);
    waveform
}

/// Sends `elements` one by one: the tick each began at, and the tick after the last.
fn send(waveform: &mut SircWaveform, elements: &[Element]) -> Vec<u64> {
    let mut ticks = Vec::with_capacity(elements.len() + 1);
    for element in elements {
        ticks.push(waveform.position().0);
        waveform.send(std::slice::from_ref(element));
    }
    ticks.push(waveform.position().0);
    ticks
}

/// Where the last mark of a frame of `bits` whose start mark begins at `origin` (at
/// [`RATE`]) ends: after the start mark and its space, five units, and every bit but the last
/// space.
fn last_mark_end(origin: u64, bits: &[bool]) -> u64 {
    let marks: u64 = bits.iter().map(|&one| if one { 2 } else { 1 }).sum();
    origin + 5 * UNIT + (marks + bits.len() as u64 - 1) * UNIT
}

/// Where a frame of `bits` whose start mark begins at `origin` (at [`RATE`]) is placed and
/// settled: from the start mark to a unit after its last mark, decided 30 ticks after that
/// mark, the longest a bit's space may last.
fn placed(origin: u64, bits: &[bool]) -> String {
    let last_mark_end = last_mark_end(origin, bits);
    format!("[{origin},{})@{}", last_mark_end + UNIT, last_mark_end + 30)
}

#[test]
fn all_three_lengths_are_read() {
    let frames: [(u8, u8, u8, u8, &str); 3] = [
        // A television's volume up: command 18, address 1.
        (12, 18, 1, 0, "frame 12 18:1"),
        (15, 0x2a, 0x97, 0, "frame 15 42:151"),
        (20, 0x7f, 0x1a, 0xe2, "frame 20 127:26:226"),
    ];
    let mut waveform = waveform(&config());
    let mut wanted = Vec::new();
    for (length, command, address, extended, line) in frames {
        let origin = waveform.position().0;
        waveform.frame(length, command, address, extended);
        waveform.idle(20_000_000);
        let bits = frame_bits(length, command, address, extended);
        wanted.push(format!("{line} {}", placed(origin, &bits)));
    }
    let block = waveform.finish();
    wanted.push(format!("eof {}", block.end()));
    assert_eq!(decode(config(), RATE, &block), wanted);
    // Command 18 (0100100) and address 1 (10000), least significant bit first: three 1s and
    // nine 0s, the last mark ending 744 ticks after the start mark begins.
    assert_eq!(placed(400, &frame_bits(12, 18, 1, 0)), "[400,1168)@1174");
}

#[test]
fn a_frame_ends_once_its_silence_is_certain() {
    let mut waveform = waveform(&config());
    waveform.frame(12, 18, 1, 0);
    // The next frame begins 31 ticks after the last mark ends: one tick past the longest space
    // a bit may have, which the first frame's end takes.
    waveform.send(&[space(31 - UNIT)]);
    let second = waveform.position().0;
    waveform.frame(12, 19, 1, 0);
    waveform.idle(10_000_000);
    let block = waveform.finish();
    let first = placed(400, &frame_bits(12, 18, 1, 0));
    let wanted = vec![
        format!("frame 12 18:1 {first}"),
        format!(
            "frame 12 19:1 {}",
            placed(second, &frame_bits(12, 19, 1, 0))
        ),
        format!("eof {}", block.end()),
    ];
    assert_eq!(decode(config(), RATE, &block), wanted);
    // The first frame is decided by the sample 30 ticks after its last mark, whether the
    // samples come in one piece, end right before it (the decoder announcing progress there),
    // or end right after it.
    let last_mark_end = last_mark_end(400, &frame_bits(12, 18, 1, 0));
    for cut in [last_mark_end + 30, last_mark_end + 31, last_mark_end + 5] {
        let mut harness = build(config(), RATE, GraphConfig::default());
        let head = block
            .slice(TickRange::new(Tick::ZERO, Tick(cut)))
            .expect("a slice");
        harness.feed(&head, &[u64::MAX]);
        harness.collect();
        let seen = harness.events().len();
        assert_eq!(seen, usize::from(cut > last_mark_end + 30), "up to {cut}");
        let tail = block
            .slice(TickRange::new(Tick(cut), block.end()))
            .expect("a slice");
        harness.feed(&tail, &[u64::MAX]);
        harness.end();
        assert_eq!(
            describe(&harness.without_progress()),
            wanted,
            "cut at {cut}"
        );
    }
}

#[test]
fn frames_repeat_while_a_key_is_held() {
    let mut waveform = waveform(&config());
    let first = waveform.elapsed_ns();
    for n in 0..4 {
        waveform.idle_until(first + n * REPEAT_PERIOD_NS);
        waveform.frame(20, 0x31, 0x1a, 0x01);
    }
    waveform.idle(10_000_000);
    let block = waveform.finish();
    let lines = kinds(config(), RATE, &block);
    assert_eq!(
        lines,
        vec![
            "frame 20 49:26:1",
            "frame 20 49:26:1",
            "frame 20 49:26:1",
            "frame 20 49:26:1",
            &format!("eof {}", block.end()),
        ]
    );
}

/// The elements of a frame with element `index` replaced by `element`: element 0 is the start
/// mark, 1 its space, `2 + 2 n` the mark of bit `n` and `3 + 2 n` its space.
fn bent(length: u8, command: u8, index: usize, element: Element) -> Vec<Element> {
    let mut elements = frame_elements(length, command, 0, 0);
    elements[index] = element;
    elements
}

#[test]
fn every_error_is_named_and_placed_where_it_was_decided() {
    // At 40 kHz and 25% a unit is read from 18 to 30 ticks, two from 36 to 60 and the start
    // mark of four from 72 to 120. `t[i]` is where element `i` begins.
    type Expect = fn(&[u64]) -> String;
    let cases: Vec<(Vec<Element>, Expect)> = vec![
        // A start mark of 130 ticks is too long at its 121st.
        (bent(12, 0, 0, mark(130)), |t| {
            format!("error start 0 0 [{},{})@{}", t[0], t[0] + 121, t[0] + 120)
        }),
        // The start's space too short, and too long.
        (bent(12, 0, 1, space(10)), |t| {
            format!("error start 0 0 [{},{})@{}", t[0], t[2], t[2])
        }),
        (bent(12, 0, 1, space(35)), |t| {
            format!("error start 0 0 [{},{})@{}", t[0], t[1] + 31, t[1] + 30)
        }),
        // Command 5 (1010000): the mark of bit 1 too short, of bit 2 between a 0's and a
        // 1's, of bit 3 too long.
        (bent(12, 5, 4, mark(10)), |t| {
            format!("error bit 1 1 [{},{})@{}", t[0], t[5], t[5])
        }),
        (bent(12, 5, 6, mark(33)), |t| {
            format!("error bit 2 1 [{},{})@{}", t[0], t[7], t[7])
        }),
        (bent(12, 5, 8, mark(65)), |t| {
            format!("error bit 3 5 [{},{})@{}", t[0], t[8] + 61, t[8] + 60)
        }),
        // The space of bit 4 too short: bit 4 was read.
        (bent(12, 5, 11, space(10)), |t| {
            format!("error bit 5 5 [{},{})@{}", t[0], t[12], t[12])
        }),
        // Thirteen bits and eleven, then silence.
        (elements(&frame_bits(13, 5, 0, 0xff)), |t| {
            format!(
                "error count 13 1005 [{},{})@{}",
                t[0],
                t[27] + 31,
                t[27] + 30
            )
        }),
        (elements(&frame_bits(11, 5, 0, 0)), |t| {
            format!("error count 11 5 [{},{})@{}", t[0], t[23] + 31, t[23] + 30)
        }),
        // A 21st bit begins.
        (elements(&frame_bits(21, 5, 0, 0)), |t| {
            format!("error count 20 5 [{},{})@{}", t[0], t[42], t[42])
        }),
    ];
    let mut waveform = waveform(&config());
    let mut wanted = Vec::new();
    for (elements, expect) in &cases {
        let ticks = send(&mut waveform, elements);
        wanted.push(expect(&ticks));
        waveform.idle(10_000_000);
        // A good frame after each: nothing of the bad one lingers.
        let origin = waveform.position().0;
        waveform.frame(15, 0x55, 0xaa, 0);
        let bits = frame_bits(15, 0x55, 0xaa, 0);
        wanted.push(format!("frame 15 85:170 {}", placed(origin, &bits)));
        waveform.idle(10_000_000);
    }
    let block = waveform.finish();
    wanted.push(format!("eof {}", block.end()));
    assert_eq!(decode(config(), RATE, &block), wanted);
}

#[test]
fn marks_too_short_for_a_start_are_passed_over() {
    let mut waveform = waveform(&config());
    // Glitches, a 1's mark, and an NEC bit.
    for ticks in [1, 48, 22, 71] {
        send(&mut waveform, &[mark(ticks), space(100)]);
    }
    send(&mut waveform, &[mark(3), space(10)]);
    let origin = waveform.position().0;
    waveform.frame(12, 0x10, 0x04, 0);
    waveform.idle(5_000_000);
    let block = waveform.finish();
    assert_eq!(
        decode(config(), RATE, &block),
        vec![
            format!(
                "frame 12 16:4 {}",
                placed(origin, &frame_bits(12, 0x10, 0x04, 0))
            ),
            format!("eof {}", block.end()),
        ]
    );
}

#[test]
fn polarity_decides_which_level_is_a_mark() {
    for polarity in [Polarity::ActiveLow, Polarity::ActiveHigh] {
        let config = SircConfig {
            polarity,
            ..config()
        };
        let mut waveform = waveform(&config);
        waveform.frame(15, 0x3c, 0xc3, 0);
        waveform.idle(10_000_000);
        let block = waveform.finish();
        assert_eq!(
            decode(config, RATE, &block),
            vec![
                format!(
                    "frame 15 60:195 {}",
                    placed(400, &frame_bits(15, 0x3c, 0xc3, 0))
                ),
                format!("eof {}", block.end()),
            ],
            "{polarity:?}"
        );
        let wrong = SircConfig {
            polarity: match polarity {
                Polarity::ActiveLow => Polarity::ActiveHigh,
                Polarity::ActiveHigh => Polarity::ActiveLow,
            },
            ..config
        };
        assert!(
            !decode(wrong, RATE, &block)
                .iter()
                .any(|line| line.starts_with("frame")),
            "{polarity:?}"
        );
    }
}

#[test]
fn tolerance_bounds_are_inclusive() {
    let run = |elements: &[Element], tolerance: u8| -> String {
        let config = SircConfig {
            tolerance,
            ..config()
        };
        let mut waveform = waveform(&config);
        waveform.send(elements);
        waveform.idle(5_000_000);
        kinds(config, RATE, &waveform.finish())[0].clone()
    };
    // Every mark and space at the bounds of its window: a 0's mark and a space from 18 to 30
    // ticks, a 1's mark from 36 to 60, the start mark from 72 to 120.
    let scaled = |zero: u64, one: u64, gap: u64, start: u64| -> Vec<Element> {
        let bits = frame_bits(12, 0x55, 0x0a, 0);
        let mut elements = vec![mark(start), space(gap)];
        for one_bit in bits {
            elements.push(mark(if one_bit { one } else { zero }));
            elements.push(space(gap));
        }
        elements
    };
    assert_eq!(run(&scaled(18, 36, 18, 72), 25), "frame 12 85:10");
    assert_eq!(run(&scaled(30, 60, 30, 120), 25), "frame 12 85:10");
    assert_eq!(run(&scaled(17, 36, 18, 72), 25), "error bit 1 1");
    assert_eq!(run(&scaled(31, 60, 30, 120), 25), "error bit 1 1");
    assert_eq!(run(&scaled(30, 35, 30, 120), 25), "error bit 0 0");
    assert_eq!(run(&scaled(30, 61, 30, 120), 25), "error bit 0 0");
    assert_eq!(run(&scaled(18, 36, 17, 72), 25), "error start 0 0");
    assert_eq!(run(&scaled(24, 48, 24, 121), 25), "error start 0 0");
    // A space of 31 ticks after the first bit is the silence that ends a frame of one bit.
    let one_bit = [
        mark(96),
        space(24),
        mark(48),
        space(31),
        mark(24),
        space(24),
    ];
    assert_eq!(run(&one_bit, 25), "error count 1 1");
    // At 10% a unit is read from 22 to 26 ticks.
    assert_eq!(run(&scaled(22, 48, 26, 96), 10), "frame 12 85:10");
    assert_eq!(run(&scaled(21, 48, 24, 96), 10), "error bit 1 1");
    assert_eq!(run(&scaled(27, 48, 24, 96), 10), "error bit 1 1");
}

/// Two frames with `between_ns` of idle line between them: the waveform, where the first
/// begins and where the second begins.
fn two_frames(between_ns: u64) -> (LogicBlock, u64, u64) {
    let mut waveform = waveform(&config());
    let first = waveform.position().0;
    waveform.frame(12, 0x01, 0x02, 0);
    waveform.idle(between_ns);
    let second = waveform.position().0;
    waveform.frame(20, 0x03, 0x05, 0x07);
    waveform.idle(10_000_000);
    (waveform.finish(), first, second)
}

#[test]
fn a_frame_whose_silence_was_not_seen_is_not_reported() {
    let (block, first, second) = two_frames(20_000_000);
    let first_line = format!(
        "frame 12 1:2 {}",
        placed(first, &frame_bits(12, 0x01, 0x02, 0))
    );
    let last_mark_end = last_mark_end(second, &frame_bits(20, 0x03, 0x05, 0x07));
    // Inside the frame, right after its last mark, and on the last tick before its silence
    // is certain.
    for cut in [
        second + 1,
        second + 300,
        last_mark_end + 1,
        last_mark_end + 30,
    ] {
        let head = block
            .slice(TickRange::new(Tick::ZERO, Tick(cut)))
            .expect("a slice");
        let mut harness = build(config(), RATE, GraphConfig::default());
        harness.run_all(&head);
        assert_eq!(
            folded(describe(&harness.items)),
            vec![
                "progress 0".to_owned(),
                first_line.clone(),
                format!("progress {second}"),
                format!("eof {cut}"),
            ],
            "cut at {cut}"
        );
    }
    // One tick more and the silence is seen.
    let head = block
        .slice(TickRange::new(Tick::ZERO, Tick(last_mark_end + 31)))
        .expect("a slice");
    let lines = kinds(config(), RATE, &head);
    assert_eq!(lines[1], "frame 20 3:5:7");
}

#[test]
fn a_gap_drops_the_frame_it_cuts_and_decoding_resumes_once_the_line_idles() {
    let (block, first, second) = two_frames(20_000_000);
    let first_bits = frame_bits(12, 0x01, 0x02, 0);
    let second_line = format!(
        "frame 20 3:5:7 {}",
        placed(second, &frame_bits(20, 0x03, 0x05, 0x07))
    );
    let last_mark_end = last_mark_end(first, &first_bits);
    // A gap inside the first frame, and one inside its closing silence before it is certain:
    // either way the first frame is lost.
    for gap in [
        TickRange::new(Tick(first + 300), Tick(first + 320)),
        TickRange::new(Tick(last_mark_end + 10), Tick(last_mark_end + 20)),
    ] {
        let mut harness = build(config(), RATE, GraphConfig::default());
        harness.feed_with_gaps(&block, &[gap], &[u64::MAX]);
        harness.end();
        assert_eq!(
            describe(&harness.without_progress()),
            vec![
                format!("gap {gap}"),
                second_line.clone(),
                format!("eof {}", block.end()),
            ],
            "gap {gap}"
        );
    }
    // A gap on the idle line: the second frame begins 31 ticks after it ends, the line idle
    // long enough to be aligned; 30 are too few.
    for (gap_end, decoded) in [(second - 31, true), (second - 30, false)] {
        let gap = TickRange::new(Tick(second - 300), Tick(gap_end));
        let mut harness = build(config(), RATE, GraphConfig::default());
        harness.feed_with_gaps(&block, &[gap], &[u64::MAX]);
        harness.end();
        let mut wanted = vec![
            format!("frame 12 1:2 {}", placed(first, &first_bits)),
            format!("gap {gap}"),
        ];
        if decoded {
            wanted.push(second_line.clone());
        }
        wanted.push(format!("eof {}", block.end()));
        assert_eq!(
            describe(&harness.without_progress()),
            wanted,
            "gap ending at {gap_end}"
        );
    }
}

#[test]
fn a_frame_under_way_at_the_first_sample_is_passed_over() {
    let (block, first, second) = two_frames(20_000_000);
    // In the start mark, its space, a bit's mark and a bit's space.
    for from in [first + 50, first + 100, first + 130, first + 500] {
        let tail = block
            .slice(TickRange::new(Tick(from), block.end()))
            .expect("a slice");
        assert_eq!(
            decode(config(), RATE, &tail),
            vec![
                format!(
                    "frame 20 3:5:7 {}",
                    placed(second, &frame_bits(20, 0x03, 0x05, 0x07))
                ),
                format!("eof {}", block.end()),
            ],
            "from {from}"
        );
    }
}

/// Frames of every length, short marks and errors; and gaps that cut a frame, the idle line
/// before another and the silence after a third.
fn rich(config: &SircConfig) -> (LogicBlock, [TickRange; 3]) {
    let mut waveform = waveform(config);
    waveform.frame(12, 18, 1, 0);
    waveform.idle(3_000_000);
    let cut = waveform.position().0;
    waveform.frame(15, 0x2a, 0x97, 0);
    waveform.idle(3_000_000);
    waveform.frame(20, 0x7f, 0x1a, 0xe2);
    waveform.idle(4_000_000);
    let after_gap = waveform.position().0;
    waveform.frame(12, 0x40, 0x10, 0);
    // Back to back with the next frame, just past the silence that ends it.
    waveform.send(&[space(31 - UNIT)]);
    waveform.frame(15, 0x01, 0xfe, 0);
    waveform.idle(2_000_000);
    send(&mut waveform, &[mark(2), space(50), mark(40), space(100)]);
    let jittered: Vec<Element> = frame_elements(20, 0x5a, 0x15, 0xa5)
        .into_iter()
        .enumerate()
        .map(|(index, element)| element.scaled([85, 115, 100, 92, 108][index % 5]))
        .collect();
    waveform.send(&jittered);
    waveform.idle(2_000_000);
    for elements in [
        bent(12, 0, 1, space(10)),
        bent(12, 5, 6, mark(33)),
        bent(12, 5, 11, space(10)),
        elements(&frame_bits(13, 5, 0, 0xff)),
        elements(&frame_bits(21, 5, 0, 0)),
    ] {
        waveform.send(&elements);
        waveform.idle(2_000_000);
    }
    waveform.send(&[Element::mark(START_MARK_NS * 2)]);
    waveform.idle(2_000_000);
    let silent = waveform.position().0;
    waveform.frame(12, 0x0f, 0x0f, 0);
    let silence = waveform.position().0 - UNIT;
    waveform.idle(3_000_000);
    waveform.frame(15, 0x70, 0x07, 0);
    waveform.idle(1_000_000);
    let gaps = [
        TickRange::new(Tick(cut + 200), Tick(cut + 210)),
        TickRange::new(Tick(after_gap - 80), Tick(after_gap - 40)),
        TickRange::new(Tick(silence + 5), Tick(silence + 8)),
    ];
    assert!(silent < silence);
    (waveform.finish(), gaps)
}

#[test]
fn every_cut_and_pause_yields_the_same_events() {
    for polarity in [Polarity::ActiveLow, Polarity::ActiveHigh] {
        let config = SircConfig {
            polarity,
            ..config()
        };
        let (block, gaps) = rich(&config);
        let reference = {
            let mut harness = build(config, RATE, GraphConfig::default());
            harness.feed_with_gaps(&block, &gaps, &[u64::MAX]);
            harness.end();
            describe(&harness.without_progress())
        };
        for wanted in [
            "frame 12 18:1 ",
            "frame 20 127:26:226 ",
            "frame 12 64:16 ",
            "frame 15 1:254 ",
            "frame 20 90:21:165 ",
            "frame 15 112:7 ",
            "error start ",
            "error bit ",
            "error count 13 ",
            "error count 20 ",
            "gap ",
        ] {
            assert!(
                reference.iter().any(|line| line.starts_with(wanted)),
                "{wanted} in {reference:?}"
            );
        }
        // The frames the first and third gaps cut are lost.
        for lost in ["frame 15 42:151", "frame 12 15:15"] {
            assert!(
                !reference.iter().any(|line| line.starts_with(lost)),
                "{lost}"
            );
        }
        assert_block_invariant(|graph| build(config, RATE, graph), &block, &gaps);
    }
}

#[test]
fn progress_never_passes_a_frame_still_being_read() {
    let (block, gaps) = rich(&config());
    for sizes in [&[1u64][..], &[7, 64], &[97], &[500]] {
        let mut harness = build(config(), RATE, GraphConfig::default());
        harness.feed_cut(&block, &gaps, sizes, &[1]);
        harness.end();
        let mut seen = Tick::ZERO;
        let mut progress = 0;
        for item in &harness.items {
            match item {
                Item::Progress(tick) => {
                    assert!(*tick >= seen, "progress went back to {tick} from {seen}");
                    seen = *tick;
                    progress += 1;
                }
                Item::Event(event) => assert!(event.span.start >= seen, "{event:?} after {seen}"),
                Item::Gap(range) => assert!(range.start >= seen, "{range} after {seen}"),
                Item::End(tick) => assert!(*tick >= seen),
            }
        }
        assert!(progress > 10, "blocks of {sizes:?}");
    }
}

#[test]
fn a_source_too_slow_for_the_tolerance_is_refused() {
    let error = try_build(config(), 6666, GraphConfig::default()).expect_err("refused");
    assert_eq!(
        error,
        GraphError::Requirement {
            from: "logic#0.samples".into(),
            to: "ir-sirc#0.logic".into(),
            key: "rateHz",
            requirement: ">= 6667".into(),
            found: "6666".into(),
        }
    );
    // At the slowest rate taken, a unit is four ticks, and frames are still read.
    let mut waveform = SircWaveform::new(&config(), CHANNELS, 6667, Tick::ZERO);
    waveform.idle(10_000_000);
    waveform.frame(20, 0x22, 0x11, 0x88);
    waveform.idle(10_000_000);
    let lines = kinds(config(), 6667, &waveform.finish());
    assert_eq!(lines[0], "frame 20 34:17:136");
    assert_eq!(lines.len(), 2);
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 32, ..ProptestConfig::default() })]

    /// Whatever the length, the fields, the jitter within the tolerance and the cuts, every
    /// frame sent is read back.
    #[test]
    fn random_frames_are_read_back(
        frames in prop::collection::vec(
            (
                prop_oneof![Just(12u8), Just(15u8), Just(20u8)],
                any::<u8>(),
                any::<u8>(),
                any::<u8>(),
                prop::collection::vec(85u64..=115, 42),
                0u64..20,
            ),
            1..10,
        ),
        sizes in prop::collection::vec(1u64..1500, 1..4),
        active_high in any::<bool>(),
    ) {
        let config = SircConfig {
            polarity: if active_high { Polarity::ActiveHigh } else { Polarity::ActiveLow },
            ..config()
        };
        // 100 kHz: 60 ticks to the unit.
        let rate = 100_000;
        let mut waveform = SircWaveform::new(&config, CHANNELS, rate, Tick::ZERO);
        waveform.idle(5_000_000);
        let mut wanted = Vec::new();
        for (length, command, address, extended, jitter, idle_ms) in &frames {
            let elements: Vec<Element> = frame_elements(*length, *command, *address, *extended)
                .iter()
                .zip(jitter)
                .map(|(element, percent)| element.scaled(*percent))
                .collect();
            waveform.send(&elements);
            waveform.idle(2 * UNIT_NS + idle_ms * 1_000_000);
            let address = if *length == 15 { *address } else { address & 0x1f };
            let extended = (*length == 20).then_some(*extended);
            wanted.push((*length, command & 0x7f, address, extended));
        }
        let block = waveform.finish();
        let mut harness = build(config, rate, GraphConfig::default());
        harness.feed(&block, &sizes);
        harness.end();
        let read: Vec<(u8, u8, u8, Option<u8>)> = harness
            .events()
            .into_iter()
            .map(|event| match event.payload {
                SircEvent::Frame { bits, command, address, extended } => {
                    Some((bits, command, address, extended))
                }
                SircEvent::Error { .. } => None,
            })
            .collect::<Option<Vec<_>>>()
            .expect("no errors");
        prop_assert_eq!(read, wanted);
    }
}
