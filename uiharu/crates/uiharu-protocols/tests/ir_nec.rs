// SPDX-License-Identifier: Apache-2.0

//! The NEC infrared decoder on synthesized receiver output.

use proptest::prelude::*;
use uiharu_core::testing::{Harness, assert_block_invariant};
use uiharu_core::{GraphConfig, GraphError, Item, LogicBlock, Tick, TickRange, TimeBase};
use uiharu_protocols::ir_nec::synth::{
    Element, LEADER_MARK_NS, NecWaveform, REPEAT_PERIOD_NS, UNIT_NS, frame_elements,
    repeat_elements,
};
use uiharu_protocols::ir_nec::{ErrorReason, Nec, NecConfig, NecEvent, Polarity};

const CHANNELS: u8 = 2;
/// The rate of most tests: 16 kHz, nine ticks to the 562.5 µs unit, so that every nominal
/// length is a whole number of ticks.
const RATE: u64 = 16_000;
/// A tick at [`RATE`], in nanoseconds.
const TICK_NS: u64 = 62_500;

/// The receiver on channel 1, active low, 25% tolerance.
fn config() -> NecConfig {
    NecConfig {
        channel: 1,
        ..NecConfig::default()
    }
}

fn try_build(
    config: NecConfig,
    rate: u64,
    graph_config: GraphConfig,
) -> Result<Harness<NecEvent>, GraphError> {
    Harness::build(
        CHANNELS,
        TimeBase::new(rate),
        graph_config,
        |graph, source| {
            let nec = graph.add(Nec::new(config).expect("a valid configuration"));
            graph.connect(source.samples, Nec::input(nec))?;
            Ok(Nec::events(nec))
        },
    )
}

fn build(config: NecConfig, rate: u64, graph_config: GraphConfig) -> Harness<NecEvent> {
    try_build(config, rate, graph_config).expect("builds")
}

fn reason(reason: ErrorReason) -> &'static str {
    match reason {
        ErrorReason::Leader => "leader",
        ErrorReason::Bit => "bit",
        ErrorReason::Count => "count",
        ErrorReason::Trailer => "trailer",
    }
}

/// One line per item, readable in assertions.
fn describe(items: &[Item<NecEvent>]) -> Vec<String> {
    items
        .iter()
        .map(|item| match item {
            Item::Event(event) => {
                let what = match event.payload {
                    NecEvent::Frame {
                        address,
                        command,
                        extended,
                        inverse_ok,
                        raw,
                    } => {
                        let extended = if extended { " extended" } else { "" };
                        let inverse = if inverse_ok { "" } else { " bad-inverse" };
                        format!("frame {address:04x}:{command:02x} {raw:08x}{extended}{inverse}")
                    }
                    NecEvent::Repeat => "repeat".to_owned(),
                    NecEvent::Error {
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
fn decode(config: NecConfig, rate: u64, block: &LogicBlock) -> Vec<String> {
    let mut harness = build(config, rate, GraphConfig::default());
    harness.run_all(block);
    describe(&harness.without_progress())
}

/// A mark of `ticks` ticks at [`RATE`].
fn mark(ticks: u64) -> Element {
    Element::mark(ticks * TICK_NS)
}

/// A space of `ticks` ticks at [`RATE`].
fn space(ticks: u64) -> Element {
    Element::space(ticks * TICK_NS)
}

/// Sends `elements` one by one: the tick each began at, and the tick after the last.
fn send(waveform: &mut NecWaveform, elements: &[Element]) -> Vec<u64> {
    let mut ticks = Vec::with_capacity(elements.len() + 1);
    for element in elements {
        ticks.push(waveform.position().0);
        waveform.send(std::slice::from_ref(element));
    }
    ticks.push(waveform.position().0);
    ticks
}

/// The waveform of `config` at [`RATE`], idle for 20 ms.
fn waveform(config: &NecConfig) -> NecWaveform {
    let mut waveform = NecWaveform::new(config, CHANNELS, RATE, Tick::ZERO);
    waveform.idle(20_000_000);
    waveform
}

#[test]
fn a_known_key_is_read_as_the_format_says() {
    // A remote control's key with address 0x00 and command 0x45, sampled at 1 MHz: the bits
    // from the first are 0x00, its inverse 0xff, 0x45 and its inverse 0xba, each least
    // significant bit first.
    let mut waveform = NecWaveform::new(&config(), CHANNELS, 1_000_000, Tick::ZERO);
    waveform.idle(20_000_000);
    waveform.frame(0x00, 0x45);
    waveform.idle(50_000_000);
    let block = waveform.finish();
    // Sixteen 1s and sixteen 0s: 67.5 ms of leader and data, then the final mark, ending at
    // 20 + 68.0625 ms.
    assert_eq!(
        decode(config(), 1_000_000, &block),
        vec!["frame 0000:45 ba45ff00 [20000,88063)@88063", "eof 138063"]
    );
    let mut harness = build(config(), 1_000_000, GraphConfig::default());
    harness.run_all(&block);
    assert_eq!(
        harness.events()[0].payload,
        NecEvent::Frame {
            address: 0x00,
            command: 0x45,
            extended: false,
            inverse_ok: true,
            raw: 0xba45_ff00,
        }
    );
}

#[test]
fn extended_addresses_and_wrong_inverses_are_reported_as_received() {
    let mut waveform = waveform(&config());
    let mut starts = Vec::new();
    for bytes in [
        [0x34, 0x12, 0x56, 0xa9],
        [0x19, 0xe6, 0x56, 0x01],
        [0xee, 0x87, 0x05, 0x4d],
    ] {
        starts.push(waveform.position().0);
        waveform.bytes(bytes);
        waveform.idle(20_000_000);
    }
    let block = waveform.finish();
    let lines = decode(config(), RATE, &block);
    assert_eq!(lines.len(), 4, "{lines:?}");
    assert!(
        lines[0].starts_with(&format!("frame 1234:56 a9561234 extended [{},", starts[0])),
        "{lines:?}"
    );
    assert!(
        lines[1].starts_with(&format!(
            "frame 0019:56 0156e619 bad-inverse [{},",
            starts[1]
        )),
        "{lines:?}"
    );
    // A 16-bit address whose command is followed by something else than its inverse.
    assert!(
        lines[2].starts_with(&format!(
            "frame 87ee:05 4d0587ee extended bad-inverse [{},",
            starts[2]
        )),
        "{lines:?}"
    );
}

#[test]
fn repeat_codes_follow_a_held_key() {
    let mut waveform = waveform(&config());
    let first = waveform.elapsed_ns();
    let mut starts = vec![waveform.position().0];
    waveform.frame(0x04, 0x08);
    for n in 1..=3 {
        waveform.idle_until(first + n * REPEAT_PERIOD_NS);
        starts.push(waveform.position().0);
        waveform.repeat();
    }
    waveform.idle(20_000_000);
    let block = waveform.finish();
    // 108 ms is 1728 ticks; a repeat code is 144 + 36 + 9 ticks.
    assert_eq!(starts, vec![320, 2048, 3776, 5504]);
    assert_eq!(
        decode(config(), RATE, &block),
        vec![
            "frame 0004:08 f708fb04 [320,1409)@1409",
            "repeat [2048,2237)@2237",
            "repeat [3776,3965)@3965",
            "repeat [5504,5693)@5693",
            "eof 6013",
        ]
    );
}

/// A frame of 0x00 0xff 0x45 0xba with element `index` replaced.
fn bent(index: usize, element: Element) -> Vec<Element> {
    let mut elements = frame_elements([0x00, 0xff, 0x45, 0xba]);
    elements[index] = element;
    elements
}

#[test]
fn every_error_is_named_and_placed_where_it_was_decided() {
    // At 16 kHz and 25%: a unit of 9 ticks is read from 7 to 11, a 1's space of 27 from 21 to
    // 33, the leader mark of 144 from 108 to 180, a frame's leader space of 72 from 54 to 90 and
    // a repeat code's of 36 from 27 to 45.
    let short_frame: Vec<Element> = frame_elements([0x00, 0xff, 0x45, 0xba])
        .into_iter()
        .take(2 + 2 * 16)
        .chain([mark(9)])
        .collect();
    let mut repeat_short = repeat_elements().to_vec();
    repeat_short[2] = mark(4);
    let mut repeat_long = repeat_elements().to_vec();
    repeat_long[2] = mark(15);
    // Each case: its elements, and the line it gives from the ticks its elements begin at.
    type Expect = fn(&[u64]) -> String;
    let cases: Vec<(Vec<Element>, Expect)> = vec![
        // A leader mark of 200 ticks is too long at its 181st.
        (bent(0, mark(200)), |t| {
            format!("error leader 0 0 [{},{})@{}", t[0], t[0] + 181, t[0] + 180)
        }),
        // Leader spaces too short, between the two windows and too long.
        (bent(1, space(20)), |t| {
            format!("error leader 0 0 [{},{})@{}", t[0], t[2], t[2])
        }),
        (bent(1, space(50)), |t| {
            format!("error leader 0 0 [{},{})@{}", t[0], t[2], t[2])
        }),
        (bent(1, space(100)), |t| {
            format!("error leader 0 0 [{},{})@{}", t[0], t[1] + 91, t[1] + 90)
        }),
        // The mark of bit 5 too short, of bit 7 too long.
        (bent(2 + 2 * 5, mark(5)), |t| {
            format!("error bit 5 0 [{},{})@{}", t[0], t[13], t[13])
        }),
        (bent(2 + 2 * 7, mark(15)), |t| {
            format!("error bit 7 0 [{},{})@{}", t[0], t[16] + 12, t[16] + 11)
        }),
        // The space of bit 3 too short, of bit 12 between a 0's and a 1's (bits 8 to 11 are 1s).
        (bent(3 + 2 * 3, space(5)), |t| {
            format!("error bit 3 0 [{},{})@{}", t[0], t[10], t[10])
        }),
        (bent(3 + 2 * 12, space(15)), |t| {
            format!("error bit 12 f00 [{},{})@{}", t[0], t[28], t[28])
        }),
        // A space longer than a 1's after bit 20's mark: the frame stopped short.
        (bent(3 + 2 * 20, space(40)), |t| {
            format!(
                "error count 20 5ff00 [{},{})@{}",
                t[0],
                t[43] + 34,
                t[43] + 33
            )
        }),
        // Sixteen bits and the final mark: the space after it is the idle line.
        (short_frame, |t| {
            format!(
                "error count 16 ff00 [{},{})@{}",
                t[0],
                t[35] + 34,
                t[35] + 33
            )
        }),
        // The final mark too short and too long.
        (bent(66, mark(5)), |t| {
            format!("error trailer 32 ba45ff00 [{},{})@{}", t[0], t[67], t[67])
        }),
        (bent(66, mark(14)), |t| {
            format!(
                "error trailer 32 ba45ff00 [{},{})@{}",
                t[0],
                t[66] + 12,
                t[66] + 11
            )
        }),
        // The final mark of a repeat code too short and too long.
        (repeat_short, |t| {
            format!("error trailer 0 0 [{},{})@{}", t[0], t[3], t[3])
        }),
        (repeat_long, |t| {
            format!("error trailer 0 0 [{},{})@{}", t[0], t[2] + 12, t[2] + 11)
        }),
    ];
    let mut waveform = waveform(&config());
    let mut wanted = Vec::new();
    for (elements, expect) in &cases {
        let ticks = send(&mut waveform, elements);
        wanted.push(expect(&ticks));
        waveform.idle(30_000_000);
        // A good frame after each: nothing of the bad one lingers.
        let start = waveform.position().0;
        waveform.frame(0x01, 0x02);
        wanted.push(format!(
            "frame 0001:02 fd02fe01 [{start},{})@{}",
            waveform.position().0,
            waveform.position().0
        ));
        waveform.idle(30_000_000);
    }
    let block = waveform.finish();
    wanted.push(format!("eof {}", block.end()));
    assert_eq!(decode(config(), RATE, &block), wanted);
}

#[test]
fn polarity_decides_which_level_is_a_mark() {
    for polarity in [Polarity::ActiveLow, Polarity::ActiveHigh] {
        let config = NecConfig {
            polarity,
            ..config()
        };
        let mut waveform = waveform(&config);
        waveform.frame(0x10, 0x20);
        waveform.idle(20_000_000);
        let block = waveform.finish();
        assert_eq!(
            decode(config, RATE, &block),
            vec!["frame 0010:20 df20ef10 [320,1409)@1409", "eof 1729"],
            "{polarity:?}"
        );
        // Read the other way round, the idle line is one long mark: no frame, and the mark
        // after the frame is too long for a leader.
        let wrong = NecConfig {
            polarity: match polarity {
                Polarity::ActiveLow => Polarity::ActiveHigh,
                Polarity::ActiveHigh => Polarity::ActiveLow,
            },
            ..config
        };
        assert_eq!(
            decode(wrong, RATE, &block),
            vec!["error leader 0 0 [1409,1590)@1589", "eof 1729"],
            "{polarity:?}"
        );
    }
}

#[test]
fn tolerance_bounds_are_inclusive() {
    // At 1 MHz and 25% a unit is read from 422 to 703 ticks, a 1's space from 1266 to 2109,
    // the leader mark from 6750 to 11250 and a frame's leader space from 3375 to 5625.
    let tick = |ticks: u64| ticks * 1000;
    let scaled = |short: u64, one: u64, leader: u64, space: u64| -> Vec<Element> {
        frame_elements([0x00, 0xff, 0x45, 0xba])
            .into_iter()
            .enumerate()
            .map(|(index, element)| match index {
                0 => Element::mark(tick(leader)),
                1 => Element::space(tick(space)),
                _ if element.ns == UNIT_NS => Element {
                    ns: tick(short),
                    ..element
                },
                _ => Element {
                    ns: tick(one),
                    ..element
                },
            })
            .collect()
    };
    let run = |elements: &[Element], tolerance: u8| -> String {
        let config = NecConfig {
            tolerance,
            ..config()
        };
        let mut waveform = NecWaveform::new(&config, CHANNELS, 1_000_000, Tick::ZERO);
        waveform.idle(20_000_000);
        waveform.send(elements);
        waveform.idle(20_000_000);
        let lines = decode(config, 1_000_000, &waveform.finish());
        lines[0].split(" [").next().unwrap_or_default().to_owned()
    };
    let good = "frame 0000:45 ba45ff00";
    assert_eq!(run(&scaled(422, 1266, 6750, 3375), 25), good);
    assert_eq!(run(&scaled(703, 2109, 11_250, 5625), 25), good);
    assert_eq!(run(&scaled(421, 1266, 6750, 3375), 25), "error bit 0 0");
    assert_eq!(run(&scaled(422, 1265, 6750, 3375), 25), "error bit 8 0");
    assert_eq!(run(&scaled(704, 2109, 11_250, 5625), 25), "error bit 0 0");
    assert_eq!(run(&scaled(703, 2110, 11_250, 5625), 25), "error count 8 0");
    assert_eq!(
        run(&scaled(562, 1687, 11_251, 4500), 25),
        "error leader 0 0"
    );
    assert_eq!(run(&scaled(562, 1687, 9000, 5626), 25), "error leader 0 0");
    // A leader mark shorter than its window is no leader at all.
    assert!(run(&scaled(562, 1687, 6749, 4500), 25).starts_with("eof "));
    // At 10% a unit is read from 507 to 618 ticks.
    assert_eq!(run(&scaled(507, 1688, 9000, 4500), 10), good);
    assert_eq!(run(&scaled(618, 1688, 9000, 4500), 10), good);
    assert_eq!(run(&scaled(619, 1688, 9000, 4500), 10), "error bit 0 0");
    assert_eq!(run(&scaled(646, 1940, 9000, 4500), 25), good);
}

#[test]
fn short_marks_on_the_idle_line_are_no_leader() {
    let mut waveform = waveform(&config());
    // Glitches and the marks of other formats: a third of a unit, an RC-5 bit, a SIRC start.
    for ticks in [3, 28, 38, 107] {
        send(&mut waveform, &[mark(ticks), space(200)]);
    }
    // A glitch right before a frame does not hide it.
    send(&mut waveform, &[mark(2), space(20)]);
    let start = waveform.position().0;
    waveform.frame(0xaa, 0x55);
    let end = waveform.position().0;
    waveform.idle(10_000_000);
    let block = waveform.finish();
    assert_eq!(
        decode(config(), RATE, &block),
        vec![
            format!("frame 00aa:55 aa5555aa [{start},{end})@{end}"),
            format!("eof {}", block.end()),
        ]
    );
}

/// Two frames with `between_ns` of idle line between them: the waveform, where the first ends
/// and where the second begins.
fn two_frames(between_ns: u64) -> (LogicBlock, u64, u64) {
    let mut waveform = waveform(&config());
    waveform.frame(0x01, 0x11);
    let first_end = waveform.position().0;
    waveform.idle(between_ns);
    let second = waveform.position().0;
    waveform.frame(0x02, 0x22);
    waveform.idle(20_000_000);
    (waveform.finish(), first_end, second)
}

#[test]
fn a_frame_cut_off_by_the_end_of_the_samples_is_not_reported() {
    let (block, first_end, second) = two_frames(40_000_000);
    for cut in [second + 1, second + 200, second + 1088] {
        let head = block
            .slice(TickRange::new(Tick::ZERO, Tick(cut)))
            .expect("a slice");
        let mut harness = build(config(), RATE, GraphConfig::default());
        harness.run_all(&head);
        // Progress goes no further than the start of the frame that was cut off.
        assert_eq!(
            folded(describe(&harness.items)),
            vec![
                "progress 0".to_owned(),
                format!("frame 0001:11 ee11fe01 [320,{first_end})@{first_end}"),
                format!("progress {second}"),
                format!("eof {cut}"),
            ],
            "cut at {cut}"
        );
    }
}

#[test]
fn a_frame_shows_up_at_the_end_of_its_final_mark() {
    let (block, first_end, _) = two_frames(40_000_000);
    let mut harness = build(config(), RATE, GraphConfig::default());
    let piece = block
        .slice(TickRange::new(Tick::ZERO, Tick(first_end)))
        .expect("a slice");
    harness.feed(&piece, &[u64::MAX]);
    harness.collect();
    assert!(harness.events().is_empty());
    let next = block
        .slice(TickRange::new(Tick(first_end), Tick(first_end + 1)))
        .expect("a slice");
    harness.feed(&next, &[u64::MAX]);
    harness.collect();
    assert_eq!(
        describe(&harness.without_progress()),
        vec![format!(
            "frame 0001:11 ee11fe01 [320,{first_end})@{first_end}"
        )]
    );
}

#[test]
fn a_gap_drops_the_frame_it_cuts_and_decoding_resumes_once_the_line_idles() {
    let (block, first_end, second) = two_frames(40_000_000);
    let second_end = second + 1089;
    let wanted_second = format!("frame 0002:22 dd22fd02 [{second},{second_end})@{second_end}");
    // A gap inside the first frame drops it.
    let lines = {
        let mut harness = build(config(), RATE, GraphConfig::default());
        harness.feed_with_gaps(&block, &[TickRange::new(Tick(700), Tick(750))], &[u64::MAX]);
        harness.end();
        folded(describe(&harness.items))
    };
    // Progress at the first sample and at the leader fold into one: nothing came between them.
    assert_eq!(
        lines,
        vec![
            "progress 320".to_owned(),
            "gap 700..750".to_owned(),
            wanted_second.clone(),
            format!("progress {}", block.end()),
            format!("eof {}", block.end()),
        ]
    );
    // A gap on the idle line: the second frame starts 91 ticks after it ends, idle long
    // enough to be aligned; 90 are too few.
    for (gap_end, decoded) in [(second - 91, true), (second - 90, false)] {
        let gap = TickRange::new(Tick(first_end + 100), Tick(gap_end));
        let mut harness = build(config(), RATE, GraphConfig::default());
        harness.feed_with_gaps(&block, &[gap], &[u64::MAX]);
        harness.end();
        let mut wanted = vec![
            format!("frame 0001:11 ee11fe01 [320,{first_end})@{first_end}"),
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
    let (block, _, second) = two_frames(40_000_000);
    // From inside the leader mark, inside the leader space, inside the data and on the final
    // mark: whatever is left of the first frame is passed over, the second is read.
    for from in [330, 470, 900, 1400] {
        let tail = block
            .slice(TickRange::new(Tick(from), block.end()))
            .expect("a slice");
        assert_eq!(
            decode(config(), RATE, &tail),
            vec![
                format!(
                    "frame 0002:22 dd22fd02 [{second},{})@{}",
                    second + 1089,
                    second + 1089
                ),
                format!("eof {}", block.end()),
            ],
            "from {from}"
        );
    }
}

/// Frames, repeat codes, extended frames, glitches and errors of every kind, with the
/// elements of one frame jittered within the tolerance; and gaps that cut a repeat code's
/// leader mark, the idle line before the extended frame and the data of a frame.
fn rich(config: &NecConfig) -> (LogicBlock, [TickRange; 3]) {
    let mut waveform = waveform(config);
    let first = waveform.elapsed_ns();
    waveform.frame(0x00, 0x45);
    waveform.idle_until(first + REPEAT_PERIOD_NS);
    let repeat = waveform.position().0;
    waveform.repeat();
    waveform.idle(30_000_000);
    waveform.repeat();
    waveform.idle(15_000_000);
    let extended = waveform.position().0;
    waveform.extended(0xbeef, 0x01);
    waveform.idle(12_000_000);
    send(&mut waveform, &[mark(30), space(40), mark(2), space(400)]);
    let jittered: Vec<Element> = frame_elements([0x5a, 0xa5, 0x3c, 0xc3])
        .into_iter()
        .enumerate()
        .map(|(index, element)| element.scaled([82, 118, 95, 107, 100][index % 5]))
        .collect();
    waveform.send(&jittered);
    waveform.idle(9_000_000);
    for elements in [
        bent(1, space(50)),
        bent(2 + 2 * 9, mark(15)),
        bent(3 + 2 * 20, space(40)),
        bent(66, mark(14)),
        repeat_elements().to_vec(),
    ] {
        waveform.send(&elements);
        waveform.idle(7_000_000);
    }
    let cut = waveform.position().0;
    waveform.frame(0x33, 0x44);
    waveform.idle(7_000_000);
    waveform.bytes([0x19, 0xe6, 0x56, 0x01]);
    waveform.idle(7_000_000);
    waveform.send(&[Element::mark(LEADER_MARK_NS * 2)]);
    waveform.idle(8_000_000);
    waveform.frame(0xff, 0x00);
    waveform.idle(6_000_000);
    let gaps = [
        TickRange::new(Tick(repeat + 50), Tick(repeat + 60)),
        TickRange::new(Tick(extended - 200), Tick(extended - 190)),
        TickRange::new(Tick(cut + 400), Tick(cut + 420)),
    ];
    (waveform.finish(), gaps)
}

#[test]
fn every_cut_and_pause_yields_the_same_events() {
    for polarity in [Polarity::ActiveLow, Polarity::ActiveHigh] {
        let config = NecConfig {
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
            "frame 0000:45 ",
            "frame beef:01 ",
            "frame 005a:3c ",
            "frame 0019:56 ",
            "frame 00ff:00 ",
            "repeat ",
            "error leader ",
            "error bit ",
            "error count ",
            "error trailer ",
            "gap ",
        ] {
            assert!(
                reference.iter().any(|line| line.starts_with(wanted)),
                "{wanted} in {reference:?}"
            );
        }
        // The frame the third gap cuts is lost, and so is the repeat code the first one cuts.
        assert!(!reference.iter().any(|line| line.starts_with("frame 0033")));
        assert_eq!(
            reference
                .iter()
                .filter(|line| line.starts_with("repeat "))
                .count(),
            2,
            "{reference:?}"
        );
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
    let error = try_build(config(), 7111, GraphConfig::default()).expect_err("refused");
    assert_eq!(
        error,
        GraphError::Requirement {
            from: "logic#0.samples".into(),
            to: "ir-nec#0.logic".into(),
            key: "rateHz",
            requirement: ">= 7112".into(),
            found: "7111".into(),
        }
    );
    // At the slowest rate taken, a unit is four ticks, and frames are still read.
    let mut waveform = NecWaveform::new(&config(), CHANNELS, 7112, Tick::ZERO);
    waveform.idle(20_000_000);
    waveform.frame(0x12, 0x34);
    waveform.idle(20_000_000);
    let lines = decode(config(), 7112, &waveform.finish());
    assert!(lines[0].starts_with("frame 0012:34 cb34ed12 "), "{lines:?}");
    assert_eq!(lines.len(), 2);
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 32, ..ProptestConfig::default() })]

    /// Whatever the bytes, the jitter within the tolerance and the cuts, every frame and repeat
    /// code sent is read back.
    #[test]
    fn random_frames_are_read_back(
        sends in prop::collection::vec(
            (any::<[u8; 4]>(), any::<bool>(), prop::collection::vec(85u64..=115, 67), 0u64..30),
            1..8,
        ),
        sizes in prop::collection::vec(1u64..2000, 1..4),
        active_high in any::<bool>(),
    ) {
        let config = NecConfig {
            polarity: if active_high { Polarity::ActiveHigh } else { Polarity::ActiveLow },
            tolerance: 20,
            ..config()
        };
        // 64 kHz: 36 ticks a unit.
        let rate = 64_000;
        let mut waveform = NecWaveform::new(&config, CHANNELS, rate, Tick::ZERO);
        waveform.idle(10_000_000);
        let mut wanted = Vec::new();
        for (bytes, repeat, jitter, idle_ms) in &sends {
            let elements: Vec<Element> = if *repeat {
                repeat_elements().to_vec()
            } else {
                frame_elements(*bytes)
            };
            let elements: Vec<Element> = elements
                .iter()
                .zip(jitter)
                .map(|(element, percent)| element.scaled(*percent))
                .collect();
            waveform.send(&elements);
            waveform.idle((6 + idle_ms) * 1_000_000);
            wanted.push(if *repeat { None } else { Some(u32::from_le_bytes(*bytes)) });
        }
        let block = waveform.finish();
        let mut harness = build(config, rate, GraphConfig::default());
        harness.feed(&block, &sizes);
        harness.end();
        let read: Vec<Option<u32>> = harness
            .events()
            .into_iter()
            .map(|event| match event.payload {
                NecEvent::Frame { raw, .. } => Some(Some(raw)),
                NecEvent::Repeat => Some(None),
                NecEvent::Error { .. } => None,
            })
            .collect::<Option<Vec<_>>>()
            .expect("no errors");
        prop_assert_eq!(read, wanted);
    }
}
