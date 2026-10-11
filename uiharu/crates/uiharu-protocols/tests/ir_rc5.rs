// SPDX-License-Identifier: Apache-2.0

//! The RC-5 infrared decoder on synthesized receiver output.

use proptest::prelude::*;
use uiharu_core::testing::{Harness, assert_block_invariant};
use uiharu_core::{GraphConfig, GraphError, Item, LogicBlock, Tick, TickRange, TimeBase};
use uiharu_protocols::ir_rc5::synth::{
    Element, HALF_BIT_NS, REPEAT_PERIOD_NS, Rc5Waveform, frame_elements,
};
use uiharu_protocols::ir_rc5::{ErrorReason, Polarity, Rc5, Rc5Config, Rc5Event};

const CHANNELS: u8 = 2;
/// The rate of most tests: 36 kHz, 32 ticks to the half bit. Durations of whole ticks are
/// given in nanoseconds rounded down, which the synthesizer places on the tick they fall short
/// of by a hair.
const RATE: u64 = 36_000;
/// Ticks to the half bit at [`RATE`].
const HALF: u64 = 32;

/// The receiver on channel 1, active low, 25% tolerance.
fn config() -> Rc5Config {
    Rc5Config {
        channel: 1,
        ..Rc5Config::default()
    }
}

fn try_build(
    config: Rc5Config,
    rate: u64,
    graph_config: GraphConfig,
) -> Result<Harness<Rc5Event>, GraphError> {
    Harness::build(
        CHANNELS,
        TimeBase::new(rate),
        graph_config,
        |graph, source| {
            let rc5 = graph.add(Rc5::new(config).expect("a valid configuration"));
            graph.connect(source.samples, Rc5::input(rc5))?;
            Ok(Rc5::events(rc5))
        },
    )
}

fn build(config: Rc5Config, rate: u64, graph_config: GraphConfig) -> Harness<Rc5Event> {
    try_build(config, rate, graph_config).expect("builds")
}

fn reason(reason: ErrorReason) -> &'static str {
    match reason {
        ErrorReason::Start => "start",
        ErrorReason::Toggle => "toggle",
        ErrorReason::Address => "address",
        ErrorReason::Command => "command",
    }
}

/// One line per item, readable in assertions.
fn describe(items: &[Item<Rc5Event>]) -> Vec<String> {
    items
        .iter()
        .map(|item| match item {
            Item::Event(event) => {
                let what = match event.payload {
                    Rc5Event::Frame {
                        toggle,
                        address,
                        command,
                        extended,
                    } => {
                        let extended = if extended { " extended" } else { "" };
                        format!("frame t{} {address}:{command}{extended}", u8::from(toggle))
                    }
                    Rc5Event::Error {
                        reason: why,
                        bits,
                        raw,
                    } => format!("error {} {bits} {raw:b}", reason(why)),
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
fn decode(config: Rc5Config, rate: u64, block: &LogicBlock) -> Vec<String> {
    let mut harness = build(config, rate, GraphConfig::default());
    harness.run_all(block);
    describe(&harness.without_progress())
}

/// `ticks` ticks at [`RATE`] in nanoseconds, rounded down.
fn ns(ticks: u64) -> u64 {
    ticks * 1_000_000_000 / RATE
}

fn mark(ticks: u64) -> Element {
    Element::mark(ns(ticks))
}

fn space(ticks: u64) -> Element {
    Element::space(ns(ticks))
}

/// The waveform of `config` at [`RATE`], idle for 20 ms: 720 ticks.
fn waveform(config: &Rc5Config) -> Rc5Waveform {
    let mut waveform = Rc5Waveform::new(config, CHANNELS, RATE, Tick::ZERO);
    waveform.idle(20_000_000);
    waveform
}

/// Where a frame whose first half bit begins at `origin` (at [`RATE`]) is placed and settled,
/// given its last bit.
fn placed(origin: u64, last_bit: bool) -> String {
    // The middle of bit 13 lies 27 half bits after the origin; a final mark ends a half bit
    // later, a final space is certain after the shortest half bit, 24 ticks.
    let middle = origin + 27 * HALF;
    let settled = if last_bit { middle + HALF } else { middle + 23 };
    format!("[{origin},{})@{settled}", middle + HALF)
}

#[test]
fn a_known_frame_is_read_as_the_format_says() {
    // Address 0 (a television), command 12 (standby), then command 13 (mute) whose last bit,
    // a 1, ends with a mark.
    let mut waveform = waveform(&config());
    waveform.frame(false, 0, 12);
    waveform.idle(30_000_000);
    let second = waveform.position().0;
    waveform.frame(false, 0, 13);
    waveform.idle(30_000_000);
    let block = waveform.finish();
    assert_eq!(second, 720 + 28 * HALF + 1080);
    assert_eq!(
        decode(config(), RATE, &block),
        vec![
            format!("frame t0 0:12 {}", placed(720, false)),
            format!("frame t0 0:13 {}", placed(second, true)),
            format!("eof {}", block.end()),
        ]
    );
    // At 1 MHz a half bit is 888.9 ticks: the first edge, the middle of the first start bit,
    // falls on 20889, and the frame is placed 888 ticks before it.
    let mut waveform = Rc5Waveform::new(&config(), CHANNELS, 1_000_000, Tick::ZERO);
    waveform.idle(20_000_000);
    waveform.frame(false, 0, 12);
    waveform.idle(30_000_000);
    let block = waveform.finish();
    // The last middle is 27 half bits in, at 43999.976 µs: tick 44000; a half bit's window
    // begins at 667 ticks.
    assert_eq!(
        decode(config(), 1_000_000, &block),
        vec![
            "frame t0 0:12 [20001,44888)@44666".to_owned(),
            format!("eof {}", block.end()),
        ]
    );
}

#[test]
fn the_toggle_bit_tells_key_presses_apart() {
    let mut waveform = waveform(&config());
    let first = waveform.elapsed_ns();
    // A key held for three frames, then pressed again.
    for (n, toggle) in [false, false, false, true].into_iter().enumerate() {
        waveform.idle_until(first + n as u64 * REPEAT_PERIOD_NS);
        waveform.frame(toggle, 20, 53);
    }
    waveform.idle(20_000_000);
    let block = waveform.finish();
    let lines = decode(config(), RATE, &block);
    let frames: Vec<&str> = lines
        .iter()
        .map(|line| line.split(" [").next().unwrap_or_default())
        .collect();
    assert_eq!(
        frames,
        vec![
            "frame t0 20:53",
            "frame t0 20:53",
            "frame t0 20:53",
            "frame t1 20:53",
            &format!("eof {}", block.end()),
        ]
    );
}

#[test]
fn rc5x_commands_fold_the_second_start_bit_in() {
    let mut waveform = waveform(&config());
    for command in [63, 64, 100, 127, 0] {
        waveform.frame(true, 31, command);
        waveform.idle(5_000_000);
    }
    let block = waveform.finish();
    let lines = decode(config(), RATE, &block);
    let frames: Vec<&str> = lines
        .iter()
        .map(|line| line.split(" [").next().unwrap_or_default())
        .collect();
    assert_eq!(
        frames,
        vec![
            "frame t1 31:63",
            "frame t1 31:64 extended",
            "frame t1 31:100 extended",
            "frame t1 31:127 extended",
            "frame t1 31:0",
            &format!("eof {}", block.end()),
        ]
    );
}

/// Sends `elements` one by one: the tick each began at, and the tick after the last.
fn send(waveform: &mut Rc5Waveform, elements: &[Element]) -> Vec<u64> {
    let mut ticks = Vec::with_capacity(elements.len() + 1);
    for element in elements {
        ticks.push(waveform.position().0);
        waveform.send(std::slice::from_ref(element));
    }
    ticks.push(waveform.position().0);
    ticks
}

/// The half bits of a frame with half bit `index` replaced by `element`.
fn bent(toggle: bool, address: u8, command: u8, index: usize, element: Element) -> Vec<Element> {
    let mut elements = frame_elements(toggle, address, command);
    elements[index] = element;
    elements
}

#[test]
fn every_error_names_its_field_and_is_placed_where_it_was_decided() {
    // At 36 kHz and 25% one half bit is read from 24 to 40 ticks and two from 48 to 80. Half
    // bit `2 n` is the first half of bit `n`, `2 n + 1` its second; `t[i]` is where half bit
    // `i` begins, and the frame is placed from `t[0]`, where its first half bit begins.
    type Expect = fn(&[u64]) -> String;
    let cases: Vec<(Vec<Element>, Expect)> = vec![
        // The first mark, 44 ticks: neither one half bit nor two.
        (bent(false, 0, 1, 1, mark(44)), |t| {
            format!("error start 1 1 [{},{})@{}", t[0], t[2], t[2])
        }),
        // The first mark, the first start bit's second half and the second start bit's first
        // half (a 0, command 64 and up), stretched to 90 ticks: too long by its 81st.
        (bent(false, 0, 70, 2, mark(58)), |t| {
            format!("error start 1 1 [{},{})@{}", t[0], t[1] + 81, t[1] + 80)
        }),
        // Toggle 1 after a second start bit 1: an edge between the two, then the toggle's
        // middle 45 ticks later.
        (bent(true, 0, 1, 4, space(45)), |t| {
            format!("error toggle 2 11 [{},{})@{}", t[0], t[4] + 41, t[4] + 40)
        }),
        // Toggle 0 after a 1: the middle two half bits on, 32 + 50 ticks: too long.
        (bent(false, 0, 1, 4, mark(50)), |t| {
            format!("error toggle 2 11 [{},{})@{}", t[0], t[3] + 81, t[3] + 80)
        }),
        // Address 00110: the third address bit's second half, a mark before an edge between
        // two 1s, 15 ticks: too short.
        (bent(false, 0b00110, 1, 2 * 5 + 1, mark(15)), |t| {
            format!("error address 6 110001 [{},{})@{}", t[0], t[12], t[12])
        }),
        // The fourth command bit's first half, 20 ticks after an edge between two equal bits.
        (bent(false, 3, 0b000000, 2 * 11, mark(20)), |t| {
            format!(
                "error command 11 11000011000 [{},{})@{}",
                t[0], t[23], t[23]
            )
        }),
        // The last bit a 1, its final mark 45 ticks: too long by its 41st.
        (bent(false, 0, 1, 27, mark(45)), |t| {
            format!(
                "error command 14 11000000000001 [{},{})@{}",
                t[0],
                t[27] + 41,
                t[27] + 40
            )
        }),
        // The last bit a 0, its final space cut short by a mark 20 ticks in.
        (
            bent(false, 0, 2, 27, space(20))
                .into_iter()
                .chain([mark(HALF)])
                .collect(),
            |t| {
                format!(
                    "error command 14 11000000000010 [{},{})@{}",
                    t[0], t[28], t[28]
                )
            },
        ),
        // A frame that stops after the middle of its tenth bit, a 0: its second half is a
        // space, and no edge follows it within two half bits.
        (
            frame_elements(false, 0b00010, 0)
                .into_iter()
                .take(20)
                .collect(),
            |t| {
                format!(
                    "error command 10 1100001000 [{},{})@{}",
                    t[0],
                    t[19] + 81,
                    t[19] + 80
                )
            },
        ),
    ];
    let mut waveform = waveform(&config());
    let mut wanted = Vec::new();
    for (elements, expect) in &cases {
        let ticks = send(&mut waveform, elements);
        wanted.push(expect(&ticks));
        waveform.idle(10_000_000);
        // A good frame after each: nothing of the bad one lingers.
        let origin = waveform.position().0;
        waveform.frame(true, 7, 7);
        wanted.push(format!("frame t1 7:7 {}", placed(origin, true)));
        waveform.idle(10_000_000);
    }
    let block = waveform.finish();
    wanted.push(format!("eof {}", block.end()));
    assert_eq!(decode(config(), RATE, &block), wanted);
}

#[test]
fn glitches_on_the_idle_line_are_passed_over() {
    let mut waveform = waveform(&config());
    for ticks in [1, 5, 23] {
        send(&mut waveform, &[mark(ticks), space(200)]);
    }
    // A glitch just before a frame does not hide it.
    send(&mut waveform, &[mark(3), space(40)]);
    let origin = waveform.position().0;
    waveform.frame(false, 9, 9);
    waveform.idle(10_000_000);
    let block = waveform.finish();
    assert_eq!(
        decode(config(), RATE, &block),
        vec![
            format!("frame t0 9:9 {}", placed(origin, true)),
            format!("eof {}", block.end()),
        ]
    );
}

#[test]
fn polarity_decides_which_level_is_a_mark() {
    for polarity in [Polarity::ActiveLow, Polarity::ActiveHigh] {
        let config = Rc5Config {
            polarity,
            ..config()
        };
        let mut waveform = waveform(&config);
        waveform.frame(true, 17, 34);
        waveform.idle(20_000_000);
        let block = waveform.finish();
        assert_eq!(
            decode(config, RATE, &block),
            vec![
                format!("frame t1 17:34 {}", placed(720, false)),
                format!("eof {}", block.end()),
            ],
            "{polarity:?}"
        );
        let wrong = Rc5Config {
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
        let config = Rc5Config {
            tolerance,
            ..config()
        };
        let mut waveform = waveform(&config);
        waveform.send(elements);
        waveform.idle(10_000_000);
        let lines = decode(config, RATE, &waveform.finish());
        lines[0].split(" [").next().unwrap_or_default().to_owned()
    };
    // A remote control whose clock is 25% slow or fast: every interval at a bound.
    for percent in [75, 125] {
        let elements: Vec<Element> = frame_elements(false, 21, 42)
            .into_iter()
            .map(|half| half.scaled(percent))
            .collect();
        assert_eq!(run(&elements, 25), "frame t0 21:42", "{percent}%");
        assert!(run(&elements, 24).starts_with("error"), "{percent}%");
    }
    // One half bit at the bounds of its window, 24 and 40 ticks, and one tick beyond. Bit 3 is
    // the first address bit, a 1 after a toggle of 1: its first half is a space between two
    // edges.
    let half = |ticks: u64| bent(true, 0b10000, 0, 6, space(ticks));
    assert_eq!(run(&half(24), 25), "frame t1 16:0");
    assert_eq!(run(&half(40), 25), "frame t1 16:0");
    assert!(run(&half(23), 25).starts_with("error address 3 "));
    assert!(run(&half(41), 25).starts_with("error address 3 "));
    // At 10% a half bit is read from 29 to 35 ticks.
    assert_eq!(run(&half(29), 10), "frame t1 16:0");
    assert!(run(&half(28), 10).starts_with("error address 3 "));
    assert!(run(&half(36), 10).starts_with("error address 3 "));
}

/// Two frames with `between_ns` of idle line between them: the waveform, where the first
/// begins and where the second begins.
fn two_frames(between_ns: u64) -> (LogicBlock, u64, u64) {
    let mut waveform = waveform(&config());
    let first = waveform.position().0;
    waveform.frame(false, 1, 2);
    waveform.idle(between_ns);
    let second = waveform.position().0;
    waveform.frame(true, 3, 5);
    waveform.idle(20_000_000);
    (waveform.finish(), first, second)
}

#[test]
fn a_frame_cut_off_by_the_end_of_the_samples_is_not_reported() {
    let (block, first, second) = two_frames(40_000_000);
    // From the first edge of the second frame to the last tick before its final mark ends.
    for cut in [second + HALF + 1, second + 300, second + 28 * HALF - 1] {
        let head = block
            .slice(TickRange::new(Tick::ZERO, Tick(cut)))
            .expect("a slice");
        let mut harness = build(config(), RATE, GraphConfig::default());
        harness.run_all(&head);
        // Progress goes no further than where the cut frame is placed.
        assert_eq!(
            folded(describe(&harness.items)),
            vec![
                "progress 0".to_owned(),
                format!("frame t0 1:2 {}", placed(first, false)),
                format!("progress {second}"),
                format!("eof {cut}"),
            ],
            "cut at {cut}"
        );
    }
}

#[test]
fn a_frame_shows_up_as_soon_as_its_last_half_bit_is_certain() {
    let (block, first, second) = two_frames(40_000_000);
    let mut harness = build(config(), RATE, GraphConfig::default());
    // The first frame ends in a space, certain 24 ticks after its last middle; the second in
    // a mark, certain at its end.
    let cuts = [
        (first + 27 * HALF + 23, 0),
        (first + 27 * HALF + 24, 1),
        (second + 28 * HALF, 1),
        (second + 28 * HALF + 1, 2),
    ];
    let mut from = Tick::ZERO;
    for (cut, seen) in cuts {
        let piece = block
            .slice(TickRange::new(from, Tick(cut)))
            .expect("a slice");
        harness.feed(&piece, &[u64::MAX]);
        harness.collect();
        assert_eq!(harness.events().len(), seen, "up to {cut}");
        from = Tick(cut);
    }
}

#[test]
fn a_gap_drops_the_frame_it_cuts_and_decoding_resumes_once_the_line_idles() {
    let (block, first, second) = two_frames(40_000_000);
    let wanted_second = format!("frame t1 3:5 {}", placed(second, true));
    let mut harness = build(config(), RATE, GraphConfig::default());
    let gap = TickRange::new(Tick(first + 300), Tick(first + 340));
    harness.feed_with_gaps(&block, &[gap], &[u64::MAX]);
    harness.end();
    assert_eq!(
        describe(&harness.without_progress()),
        vec![
            format!("gap {gap}"),
            wanted_second.clone(),
            format!("eof {}", block.end()),
        ]
    );
    // A gap on the idle line: the second frame's first edge comes 81 ticks after it ends, the
    // line idle long enough to be aligned; 80 are too few.
    let first_edge = second + HALF;
    for (gap_end, decoded) in [(first_edge - 81, true), (first_edge - 80, false)] {
        let gap = TickRange::new(Tick(first_edge - 400), Tick(gap_end));
        let mut harness = build(config(), RATE, GraphConfig::default());
        harness.feed_with_gaps(&block, &[gap], &[u64::MAX]);
        harness.end();
        let mut wanted = vec![
            format!("frame t0 1:2 {}", placed(first, false)),
            format!("gap {gap}"),
        ];
        if decoded {
            wanted.push(wanted_second.clone());
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
    let (block, first, second) = two_frames(40_000_000);
    for from in [first + 10, first + 40, first + 400, first + 28 * HALF - 5] {
        let tail = block
            .slice(TickRange::new(Tick(from), block.end()))
            .expect("a slice");
        assert_eq!(
            decode(config(), RATE, &tail),
            vec![
                format!("frame t1 3:5 {}", placed(second, true)),
                format!("eof {}", block.end()),
            ],
            "from {from}"
        );
    }
}

/// Frames of every field, RC-5X commands, glitches and errors; and gaps that cut a frame, the
/// idle line before another and the first start bit of a third.
fn rich(config: &Rc5Config) -> (LogicBlock, [TickRange; 3]) {
    let mut waveform = waveform(config);
    waveform.frame(false, 0, 12);
    waveform.idle(5_000_000);
    let cut = waveform.position().0;
    waveform.frame(true, 31, 127);
    waveform.idle(5_000_000);
    waveform.frame(false, 10, 64);
    waveform.idle(6_000_000);
    let after_gap = waveform.position().0;
    waveform.frame(true, 5, 33);
    waveform.idle(3_000_000);
    send(&mut waveform, &[mark(3), space(100), mark(20), space(200)]);
    let jittered: Vec<Element> = frame_elements(false, 21, 99)
        .into_iter()
        .enumerate()
        .map(|(index, half)| half.scaled([84, 116, 100, 91, 109][index % 5]))
        .collect();
    waveform.send(&jittered);
    waveform.idle(4_000_000);
    for elements in [
        bent(false, 0, 1, 1, mark(44)),
        bent(true, 0, 1, 4, space(45)),
        bent(false, 0b00110, 1, 11, mark(15)),
        frame_elements(false, 0b00010, 0)
            .into_iter()
            .take(20)
            .collect(),
    ] {
        waveform.send(&elements);
        waveform.idle(4_000_000);
    }
    let started = waveform.position().0;
    waveform.frame(false, 2, 2);
    waveform.idle(4_000_000);
    waveform.frame(true, 3, 3);
    waveform.idle(3_000_000);
    let gaps = [
        TickRange::new(Tick(cut + 200), Tick(cut + 230)),
        TickRange::new(Tick(after_gap - 150), Tick(after_gap - 120)),
        TickRange::new(Tick(started + 10), Tick(started + 20)),
    ];
    (waveform.finish(), gaps)
}

#[test]
fn every_cut_and_pause_yields_the_same_events() {
    for polarity in [Polarity::ActiveLow, Polarity::ActiveHigh] {
        let config = Rc5Config {
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
            "frame t0 0:12 ",
            "frame t0 10:64 extended ",
            "frame t1 5:33 ",
            "frame t0 21:99 extended ",
            "frame t1 3:3 ",
            "error start ",
            "error toggle ",
            "error address ",
            "error command ",
            "gap ",
        ] {
            assert!(
                reference.iter().any(|line| line.starts_with(wanted)),
                "{wanted} in {reference:?}"
            );
        }
        // The frames the first and third gaps cut are lost.
        for lost in ["frame t1 31:127", "frame t0 2:2"] {
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
    let error = try_build(config(), 4499, GraphConfig::default()).expect_err("refused");
    assert_eq!(
        error,
        GraphError::Requirement {
            from: "logic#0.samples".into(),
            to: "ir-rc5#0.logic".into(),
            key: "rateHz",
            requirement: ">= 4500".into(),
            found: "4499".into(),
        }
    );
    // At the slowest rate taken, a half bit is four ticks, and frames are still read.
    let mut waveform = Rc5Waveform::new(&config(), CHANNELS, 4500, Tick::ZERO);
    waveform.idle(20_000_000);
    waveform.frame(true, 6, 66);
    waveform.idle(20_000_000);
    let lines = decode(config(), 4500, &waveform.finish());
    assert!(lines[0].starts_with("frame t1 6:66 extended "), "{lines:?}");
    assert_eq!(lines.len(), 2);
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 32, ..ProptestConfig::default() })]

    /// Whatever the fields, the jitter within the tolerance and the cuts, every frame sent is
    /// read back.
    #[test]
    fn random_frames_are_read_back(
        frames in prop::collection::vec(
            (any::<bool>(), 0u8..32, 0u8..128, prop::collection::vec(86u64..=114, 28), 0u64..20),
            1..10,
        ),
        sizes in prop::collection::vec(1u64..800, 1..4),
        active_high in any::<bool>(),
    ) {
        let config = Rc5Config {
            polarity: if active_high { Polarity::ActiveHigh } else { Polarity::ActiveLow },
            tolerance: 20,
            ..config()
        };
        // 144 kHz: 128 ticks to the half bit.
        let rate = 144_000;
        let mut waveform = Rc5Waveform::new(&config, CHANNELS, rate, Tick::ZERO);
        waveform.idle(5_000_000);
        let mut wanted = Vec::new();
        for (toggle, address, command, jitter, idle_ms) in &frames {
            let elements: Vec<Element> = frame_elements(*toggle, *address, *command)
                .iter()
                .zip(jitter)
                .map(|(half, percent)| half.scaled(*percent))
                .collect();
            waveform.send(&elements);
            waveform.idle(3 * HALF_BIT_NS + idle_ms * 1_000_000);
            wanted.push((*toggle, *address, *command));
        }
        let block = waveform.finish();
        let mut harness = build(config, rate, GraphConfig::default());
        harness.feed(&block, &sizes);
        harness.end();
        let read: Vec<(bool, u8, u8)> = harness
            .events()
            .into_iter()
            .map(|event| match event.payload {
                Rc5Event::Frame { toggle, address, command, extended } => {
                    assert_eq!(extended, command >= 64);
                    Some((toggle, address, command))
                }
                Rc5Event::Error { .. } => None,
            })
            .collect::<Option<Vec<_>>>()
            .expect("no errors");
        prop_assert_eq!(read, wanted);
    }
}
