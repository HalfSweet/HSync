// SPDX-License-Identifier: Apache-2.0

//! The RC-6 infrared decoder on synthesized receiver output.

use proptest::prelude::*;
use uiharu_core::testing::{Harness, assert_block_invariant};
use uiharu_core::{GraphConfig, GraphError, Item, LogicBlock, Tick, TickRange, TimeBase};
use uiharu_protocols::ir_rc6::synth::{
    Element, LEADER_MARK_NS, Rc6Waveform, SIGNAL_FREE_NS, frame_elements,
};
use uiharu_protocols::ir_rc6::{ErrorReason, Polarity, Rc6, Rc6Config, Rc6Event};

const CHANNELS: u8 = 2;
/// The rate of most tests: 72 kHz, 32 ticks to the unit. Durations of whole ticks are given
/// in nanoseconds rounded down, which the synthesizer places on the tick they fall short of by
/// a hair.
const RATE: u64 = 72_000;
/// Ticks to the unit at [`RATE`].
const UNIT: u64 = 32;

/// The receiver on channel 1, active low, 25% tolerance.
fn config() -> Rc6Config {
    Rc6Config {
        channel: 1,
        ..Rc6Config::default()
    }
}

fn try_build(
    config: Rc6Config,
    rate: u64,
    graph_config: GraphConfig,
) -> Result<Harness<Rc6Event>, GraphError> {
    Harness::build(
        CHANNELS,
        TimeBase::new(rate),
        graph_config,
        |graph, source| {
            let rc6 = graph.add(Rc6::new(config).expect("a valid configuration"));
            graph.connect(source.samples, Rc6::input(rc6))?;
            Ok(Rc6::events(rc6))
        },
    )
}

fn build(config: Rc6Config, rate: u64, graph_config: GraphConfig) -> Harness<Rc6Event> {
    try_build(config, rate, graph_config).expect("builds")
}

fn reason(reason: ErrorReason) -> &'static str {
    match reason {
        ErrorReason::Leader => "leader",
        ErrorReason::Start => "start",
        ErrorReason::Mode => "mode",
        ErrorReason::Toggle => "toggle",
        ErrorReason::Address => "address",
        ErrorReason::Command => "command",
        ErrorReason::UnsupportedMode => "unsupported-mode",
    }
}

/// One line per item, readable in assertions.
fn describe(items: &[Item<Rc6Event>]) -> Vec<String> {
    items
        .iter()
        .map(|item| match item {
            Item::Event(event) => {
                let what = match event.payload {
                    Rc6Event::Frame {
                        mode,
                        toggle,
                        address,
                        command,
                    } => format!(
                        "frame m{mode} t{} {address:02x}:{command:02x}",
                        u8::from(toggle)
                    ),
                    Rc6Event::Error {
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
fn decode(config: Rc6Config, rate: u64, block: &LogicBlock) -> Vec<String> {
    let mut harness = build(config, rate, GraphConfig::default());
    harness.run_all(block);
    describe(&harness.without_progress())
}

/// The lines of `block` without their placement.
fn kinds(config: Rc6Config, rate: u64, block: &LogicBlock) -> Vec<String> {
    decode(config, rate, block)
        .into_iter()
        .map(|line| line.split(" [").next().unwrap_or_default().to_owned())
        .collect()
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

/// The waveform of `config` at [`RATE`], idle for 10 ms: 720 ticks.
fn waveform(config: &Rc6Config) -> Rc6Waveform {
    let mut waveform = Rc6Waveform::new(config, CHANNELS, RATE, Tick::ZERO);
    waveform.idle(10_000_000);
    waveform
}

/// Where a frame whose leader begins at `origin` (at [`RATE`]) is placed and settled, given
/// its last bit.
fn placed(origin: u64, last_bit: bool) -> String {
    // The middle of the last bit lies 51 units after the origin: a final mark (a last 0) ends
    // one unit later, a final space (a last 1) is certain after the shortest unit, 24 ticks.
    let middle = origin + 51 * UNIT;
    let settled = if last_bit { middle + 23 } else { middle + UNIT };
    format!("[{origin},{})@{settled}", middle + UNIT)
}

/// Sends `elements` one by one: the tick each began at, and the tick after the last.
fn send(waveform: &mut Rc6Waveform, elements: &[Element]) -> Vec<u64> {
    let mut ticks = Vec::with_capacity(elements.len() + 1);
    for element in elements {
        ticks.push(waveform.position().0);
        waveform.send(std::slice::from_ref(element));
    }
    ticks.push(waveform.position().0);
    ticks
}

/// The elements of a frame of `mode` with element `index` replaced by `element`: element 0 is
/// the leader mark, 1 the leader space, `2 + 2 n` the first half of bit `n` and `3 + 2 n` its
/// second half.
fn bent(
    mode: u8,
    toggle: bool,
    address: u8,
    command: u8,
    index: usize,
    element: Element,
) -> Vec<Element> {
    let mut elements = frame_elements(mode, toggle, address, command);
    elements[index] = element;
    elements
}

#[test]
fn a_known_mode_0_frame_is_read_as_the_format_says() {
    // A television (address 0), standby (command 0x0c, a last bit of 0), then mute (0x0d, a
    // last bit of 1).
    let mut waveform = waveform(&config());
    waveform.frame(false, 0x00, 0x0c);
    waveform.idle(SIGNAL_FREE_NS);
    let second = waveform.position().0;
    waveform.frame(true, 0x00, 0x0d);
    waveform.idle(10_000_000);
    let block = waveform.finish();
    // A frame is 52 units; the signal-free time 6 units.
    assert_eq!(second, 720 + 52 * UNIT + 6 * UNIT);
    assert_eq!(
        decode(config(), RATE, &block),
        vec![
            format!("frame m0 t0 00:0c {}", placed(720, false)),
            format!("frame m0 t1 00:0d {}", placed(second, true)),
            format!("eof {}", block.end()),
        ]
    );
    // At 1 MHz a unit is 444.4 ticks: the frame ends 52 units, 23111.1 µs, after it begins.
    let mut waveform = Rc6Waveform::new(&config(), CHANNELS, 1_000_000, Tick::ZERO);
    waveform.idle(10_000_000);
    waveform.frame(false, 0x00, 0x0c);
    waveform.idle(10_000_000);
    let block = waveform.finish();
    assert_eq!(
        decode(config(), 1_000_000, &block),
        vec![
            "frame m0 t0 00:0c [10000,33112)@33112".to_owned(),
            format!("eof {}", block.end()),
        ]
    );
}

#[test]
fn the_toggle_follows_the_trailer_bit() {
    let mut waveform = waveform(&config());
    for toggle in [false, false, true, true, false] {
        waveform.frame(toggle, 0x04, 0x10);
        waveform.idle(20_000_000);
    }
    let block = waveform.finish();
    assert_eq!(
        kinds(config(), RATE, &block),
        vec![
            "frame m0 t0 04:10",
            "frame m0 t0 04:10",
            "frame m0 t1 04:10",
            "frame m0 t1 04:10",
            "frame m0 t0 04:10",
            &format!("eof {}", block.end()),
        ]
    );
}

#[test]
fn the_double_length_trailer_bit_is_read_in_every_neighbourhood() {
    // The trailer bit next to the last mode bit (a 0) and to the first address bit, of either
    // value: its halves stand alone between two edges or merge with a neighbour's half into
    // three units.
    {
        let mut waveform = waveform(&config());
        let mut wanted = Vec::new();
        for toggle in [false, true] {
            for address in [0x00, 0x80, 0x7f, 0xff] {
                waveform.frame(toggle, address, 0xa5);
                waveform.idle(5_000_000);
                wanted.push(format!("frame m0 t{} {address:02x}:a5", u8::from(toggle)));
            }
        }
        let block = waveform.finish();
        wanted.push(format!("eof {}", block.end()));
        assert_eq!(kinds(config(), RATE, &block), wanted);
    }

    let run = |elements: &[Element]| -> String {
        let mut waveform = waveform(&config());
        waveform.send(elements);
        waveform.idle(5_000_000);
        kinds(config(), RATE, &waveform.finish())[0].clone()
    };
    // Toggle 0 after a mode bit of 0: its first half, a space of two units, stands between an
    // edge and the trailer bit's middle and is read from 48 to 80 ticks.
    let alone = |ticks: u64| bent(0, false, 0x00, 0x00, 10, space(ticks));
    assert_eq!(run(&alone(48)), "frame m0 t0 00:00");
    assert_eq!(run(&alone(80)), "frame m0 t0 00:00");
    assert_eq!(run(&alone(47)), "error toggle 4 1000");
    assert_eq!(run(&alone(81)), "error toggle 4 1000");
    // Toggle 1 after a mode bit of 0: the last mode bit's second half and the trailer bit's
    // first make one mark of three units, read up to 120 ticks.
    let merged = |ticks: u64| bent(0, true, 0x00, 0x00, 10, mark(ticks));
    assert_eq!(run(&merged(88)), "frame m0 t1 00:00");
    assert_eq!(run(&merged(89)), "error toggle 4 1000");
    assert_eq!(run(&merged(20)), "error toggle 4 1000");
    // Toggle 0 before an address bit of 1: the trailer bit's second half and the address
    // bit's first make one mark of three units. Where the windows of two and three units
    // overlap, 72 to 80 ticks, the nearer wins: 81 ticks are three units, 80 as near to two,
    // which is then taken, and the frame no longer fits.
    let after = |ticks: u64| bent(0, false, 0x80, 0x00, 11, mark(ticks));
    assert_eq!(run(&after(49)), "frame m0 t0 80:00");
    assert_eq!(run(&after(88)), "frame m0 t0 80:00");
    assert!(run(&after(48)).starts_with("error address"));
}

#[test]
fn every_error_names_its_field_and_is_placed_where_it_was_decided() {
    // At 72 kHz and 25% a unit is read from 24 to 40 ticks, two from 48 to 80, three from 72
    // to 120 and the leader mark of six from 144 to 240. `t[i]` is where element `i` begins.
    type Expect = fn(&[u64]) -> String;
    let cases: Vec<(Vec<Element>, Expect)> = vec![
        // A leader mark of 250 ticks is too long at its 241st.
        (bent(0, false, 0, 0, 0, mark(250)), |t| {
            format!("error leader 0 0 [{},{})@{}", t[0], t[0] + 241, t[0] + 240)
        }),
        // Leader spaces too short and too long.
        (bent(0, false, 0, 0, 1, space(40)), |t| {
            format!("error leader 0 0 [{},{})@{}", t[0], t[2], t[2])
        }),
        (bent(0, false, 0, 0, 1, space(90)), |t| {
            format!("error leader 0 0 [{},{})@{}", t[0], t[1] + 81, t[1] + 80)
        }),
        // The start bit's mark, 45 ticks.
        (bent(0, false, 0, 0, 2, mark(45)), |t| {
            format!("error start 0 0 [{},{})@{}", t[0], t[2] + 41, t[2] + 40)
        }),
        // The second mode bit's first half, after an edge between two 0s, 15 ticks.
        (bent(0, false, 0, 0, 6, space(15)), |t| {
            format!("error mode 2 10 [{},{})@{}", t[0], t[7], t[7])
        }),
        // Mode 6: decided at the middle of the last mode bit.
        (frame_elements(6, false, 0, 0), |t| {
            format!("error unsupported-mode 4 1110 [{},{})@{}", t[0], t[9], t[9])
        }),
        // The trailer bit's first half, alone after an edge, 85 ticks.
        (bent(0, false, 0, 0, 10, space(85)), |t| {
            format!(
                "error toggle 4 1000 [{},{})@{}",
                t[0],
                t[10] + 81,
                t[10] + 80
            )
        }),
        // The trailer bit's first half merged with the last mode bit's second, 32 + 20 ticks.
        (bent(0, true, 0, 0, 10, mark(20)), |t| {
            format!("error toggle 4 1000 [{},{})@{}", t[0], t[11], t[11])
        }),
        // The first address bit's first half merged with the trailer bit's second, 64 + 60.
        (bent(0, false, 0x80, 0, 12, mark(60)), |t| {
            format!(
                "error address 5 10000 [{},{})@{}",
                t[0],
                t[11] + 121,
                t[11] + 120
            )
        }),
        // The fifth command bit's first half, alone after an edge, 45 ticks.
        (bent(0, false, 0, 0, 2 + 2 * 17, space(45)), |t| {
            format!(
                "error command 17 10000000000000000 [{},{})@{}",
                t[0],
                t[36] + 41,
                t[36] + 40
            )
        }),
        // The last bit a 0, its final mark 45 ticks.
        (bent(0, false, 0, 0, 43, mark(45)), |t| {
            format!(
                "error command 21 100000000000000000000 [{},{})@{}",
                t[0],
                t[43] + 41,
                t[43] + 40
            )
        }),
        // The last bit a 1, its final space cut short by a mark 20 ticks in.
        (
            bent(0, false, 0, 1, 43, space(20))
                .into_iter()
                .chain([mark(UNIT)])
                .collect(),
            |t| {
                format!(
                    "error command 21 100000000000000000001 [{},{})@{}",
                    t[0], t[44], t[44]
                )
            },
        ),
    ];
    let mut waveform = waveform(&config());
    let mut wanted = Vec::new();
    for (elements, expect) in &cases {
        let ticks = send(&mut waveform, elements);
        wanted.push(expect(&ticks));
        waveform.idle(5_000_000);
        // A good frame after each: nothing of the bad one lingers.
        let origin = waveform.position().0;
        waveform.frame(true, 0x5a, 0x3c);
        wanted.push(format!("frame m0 t1 5a:3c {}", placed(origin, false)));
        waveform.idle(5_000_000);
    }
    let block = waveform.finish();
    wanted.push(format!("eof {}", block.end()));
    assert_eq!(decode(config(), RATE, &block), wanted);
}

#[test]
fn marks_too_short_for_a_leader_are_passed_over() {
    let mut waveform = waveform(&config());
    // Glitches, an RC-5 bit, and a leader mark of 100 ticks whose frame follows.
    for ticks in [2, 30, 64] {
        send(&mut waveform, &[mark(ticks), space(300)]);
    }
    waveform.send(&bent(0, false, 1, 1, 0, mark(100)));
    waveform.idle(5_000_000);
    send(&mut waveform, &[mark(5), space(20)]);
    let origin = waveform.position().0;
    waveform.frame(false, 0x11, 0x22);
    waveform.idle(5_000_000);
    let block = waveform.finish();
    assert_eq!(
        decode(config(), RATE, &block),
        vec![
            format!("frame m0 t0 11:22 {}", placed(origin, false)),
            format!("eof {}", block.end()),
        ]
    );
}

#[test]
fn polarity_decides_which_level_is_a_mark() {
    for polarity in [Polarity::ActiveLow, Polarity::ActiveHigh] {
        let config = Rc6Config {
            polarity,
            ..config()
        };
        let mut waveform = waveform(&config);
        waveform.frame(true, 0x40, 0x81);
        waveform.idle(10_000_000);
        let block = waveform.finish();
        assert_eq!(
            decode(config, RATE, &block),
            vec![
                format!("frame m0 t1 40:81 {}", placed(720, true)),
                format!("eof {}", block.end()),
            ],
            "{polarity:?}"
        );
        let wrong = Rc6Config {
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
fn a_remote_control_off_by_a_clock_error_is_followed() {
    let run = |percent: u64, tolerance: u8| -> String {
        let config = Rc6Config {
            tolerance,
            ..config()
        };
        let elements: Vec<Element> = frame_elements(0, false, 0x80, 0x6d)
            .into_iter()
            .map(|element| element.scaled(percent))
            .collect();
        let mut waveform = waveform(&config);
        waveform.send(&elements);
        waveform.idle(5_000_000);
        kinds(config, RATE, &waveform.finish())[0].clone()
    };
    // Every length scaled alike: up to 25% slow, and fast as long as three units stay nearer
    // to three than to two after the trailer bit.
    for percent in [86, 90, 110, 125] {
        assert_eq!(run(percent, 25), "frame m0 t0 80:6d", "{percent}%");
    }
    assert!(run(126, 25).starts_with("error"));
    assert_eq!(run(108, 10), "frame m0 t0 80:6d");
    assert!(run(112, 10).starts_with("error"));
}

/// Two frames with `between_ns` of idle line between them: the waveform, where the first
/// begins and where the second begins.
fn two_frames(between_ns: u64) -> (LogicBlock, u64, u64) {
    let mut waveform = waveform(&config());
    let first = waveform.position().0;
    waveform.frame(false, 0x01, 0x02);
    waveform.idle(between_ns);
    let second = waveform.position().0;
    waveform.frame(true, 0x03, 0x05);
    waveform.idle(10_000_000);
    (waveform.finish(), first, second)
}

#[test]
fn a_frame_cut_off_by_the_end_of_the_samples_is_not_reported() {
    let (block, first, second) = two_frames(20_000_000);
    for cut in [second + 1, second + 600, second + 52 * UNIT - 9] {
        let head = block
            .slice(TickRange::new(Tick::ZERO, Tick(cut)))
            .expect("a slice");
        let mut harness = build(config(), RATE, GraphConfig::default());
        harness.run_all(&head);
        assert_eq!(
            folded(describe(&harness.items)),
            vec![
                "progress 0".to_owned(),
                format!("frame m0 t0 01:02 {}", placed(first, false)),
                format!("progress {second}"),
                format!("eof {cut}"),
            ],
            "cut at {cut}"
        );
    }
}

#[test]
fn a_frame_shows_up_as_soon_as_its_last_half_bit_is_certain() {
    let (block, first, second) = two_frames(20_000_000);
    let mut harness = build(config(), RATE, GraphConfig::default());
    // The first frame ends in a mark, certain at its end; the second in a space, certain 24
    // ticks after its last middle.
    let cuts = [
        (first + 52 * UNIT, 0),
        (first + 52 * UNIT + 1, 1),
        (second + 51 * UNIT + 23, 1),
        (second + 51 * UNIT + 24, 2),
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
    let (block, first, second) = two_frames(20_000_000);
    let wanted_second = format!("frame m0 t1 03:05 {}", placed(second, true));
    let gap = TickRange::new(Tick(first + 900), Tick(first + 950));
    let mut harness = build(config(), RATE, GraphConfig::default());
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
    // A gap on the idle line: the second frame's leader comes 121 ticks after it ends, the
    // line idle long enough to be aligned; 120 are too few.
    for (gap_end, decoded) in [(second - 121, true), (second - 120, false)] {
        let gap = TickRange::new(Tick(second - 500), Tick(gap_end));
        let mut harness = build(config(), RATE, GraphConfig::default());
        harness.feed_with_gaps(&block, &[gap], &[u64::MAX]);
        harness.end();
        let mut wanted = vec![
            format!("frame m0 t0 01:02 {}", placed(first, false)),
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
    let (block, first, second) = two_frames(20_000_000);
    // In the leader mark, the leader space, the trailer bit and the data.
    for from in [first + 20, first + 200, first + 560, first + 1200] {
        let tail = block
            .slice(TickRange::new(Tick(from), block.end()))
            .expect("a slice");
        assert_eq!(
            decode(config(), RATE, &tail),
            vec![
                format!("frame m0 t1 03:05 {}", placed(second, true)),
                format!("eof {}", block.end()),
            ],
            "from {from}"
        );
    }
}

/// Frames of every neighbourhood of the trailer bit, glitches, a mode 6 frame and errors; and
/// gaps that cut a frame, the idle line before another and a leader mark.
fn rich(config: &Rc6Config) -> (LogicBlock, [TickRange; 3]) {
    let mut waveform = waveform(config);
    waveform.frame(false, 0x00, 0x0c);
    waveform.idle(4_000_000);
    let cut = waveform.position().0;
    waveform.frame(true, 0xff, 0xff);
    waveform.idle(4_000_000);
    waveform.frame(true, 0x80, 0x01);
    waveform.idle(5_000_000);
    let after_gap = waveform.position().0;
    waveform.frame(false, 0x7f, 0x80);
    waveform.idle(3_000_000);
    send(&mut waveform, &[mark(3), space(100), mark(60), space(300)]);
    let jittered: Vec<Element> = frame_elements(0, true, 0x24, 0x9b)
        .into_iter()
        .enumerate()
        .map(|(index, element)| element.scaled([92, 108, 100, 95, 105][index % 5]))
        .collect();
    waveform.send(&jittered);
    waveform.idle(3_000_000);
    for elements in [
        bent(0, false, 0, 0, 1, space(40)),
        frame_elements(6, true, 0x0f, 0x04),
        bent(0, false, 0, 0, 6, space(15)),
        bent(0, false, 0x80, 0, 12, mark(60)),
        bent(0, false, 0, 0, 43, mark(45)),
    ] {
        waveform.send(&elements);
        waveform.idle(3_000_000);
    }
    waveform.send(&[Element::mark(LEADER_MARK_NS * 2)]);
    waveform.idle(3_000_000);
    let leader = waveform.position().0;
    waveform.frame(false, 0x12, 0x34);
    waveform.idle(3_000_000);
    waveform.frame(true, 0x56, 0x78);
    waveform.idle(2_000_000);
    let gaps = [
        TickRange::new(Tick(cut + 700), Tick(cut + 730)),
        TickRange::new(Tick(after_gap - 200), Tick(after_gap - 150)),
        TickRange::new(Tick(leader + 50), Tick(leader + 60)),
    ];
    (waveform.finish(), gaps)
}

#[test]
fn every_cut_and_pause_yields_the_same_events() {
    for polarity in [Polarity::ActiveLow, Polarity::ActiveHigh] {
        let config = Rc6Config {
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
            "frame m0 t0 00:0c ",
            "frame m0 t1 80:01 ",
            "frame m0 t0 7f:80 ",
            "frame m0 t1 24:9b ",
            "frame m0 t1 56:78 ",
            "error leader ",
            "error unsupported-mode 4 1110 ",
            "error mode ",
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
        for lost in ["frame m0 t1 ff:ff", "frame m0 t0 12:34"] {
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
    let error = try_build(config(), 8999, GraphConfig::default()).expect_err("refused");
    assert_eq!(
        error,
        GraphError::Requirement {
            from: "logic#0.samples".into(),
            to: "ir-rc6#0.logic".into(),
            key: "rateHz",
            requirement: ">= 9000".into(),
            found: "8999".into(),
        }
    );
    // At the slowest rate taken, a unit is four ticks, and frames are still read.
    let mut waveform = Rc6Waveform::new(&config(), CHANNELS, 9000, Tick::ZERO);
    waveform.idle(10_000_000);
    waveform.frame(true, 0x0c, 0xc0);
    waveform.idle(10_000_000);
    assert_eq!(
        kinds(config(), 9000, &waveform.finish()),
        vec!["frame m0 t1 0c:c0", "eof 388"]
    );
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 32, ..ProptestConfig::default() })]

    /// Whatever the fields, the jitter within the tolerance and the cuts, every frame sent is
    /// read back.
    #[test]
    fn random_frames_are_read_back(
        frames in prop::collection::vec(
            (any::<bool>(), any::<u8>(), any::<u8>(), prop::collection::vec(88u64..=112, 44), 0u64..10),
            1..10,
        ),
        sizes in prop::collection::vec(1u64..1500, 1..4),
        active_high in any::<bool>(),
    ) {
        let config = Rc6Config {
            polarity: if active_high { Polarity::ActiveHigh } else { Polarity::ActiveLow },
            ..config()
        };
        // 144 kHz: 64 ticks to the unit.
        let rate = 144_000;
        let mut waveform = Rc6Waveform::new(&config, CHANNELS, rate, Tick::ZERO);
        waveform.idle(5_000_000);
        let mut wanted = Vec::new();
        for (toggle, address, command, jitter, idle_ms) in &frames {
            let elements: Vec<Element> = frame_elements(0, *toggle, *address, *command)
                .iter()
                .zip(jitter)
                .map(|(element, percent)| element.scaled(*percent))
                .collect();
            waveform.send(&elements);
            waveform.idle(SIGNAL_FREE_NS + idle_ms * 1_000_000);
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
                Rc6Event::Frame { mode: 0, toggle, address, command } => {
                    Some((toggle, address, command))
                }
                Rc6Event::Frame { .. } | Rc6Event::Error { .. } => None,
            })
            .collect::<Option<Vec<_>>>()
            .expect("no errors");
        prop_assert_eq!(read, wanted);
    }
}
