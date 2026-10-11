// SPDX-License-Identifier: Apache-2.0

//! The MDIO decoder on synthesized waveforms.

use proptest::prelude::*;
use serde::Serialize;
use uiharu_core::testing::{Harness, assert_block_invariant};
use uiharu_core::{GraphConfig, Item, LogicBlock, Tick, TickRange, TimeBase};
use uiharu_protocols::mdio::synth::{Frame, MdioWaveform};
use uiharu_protocols::mdio::{Mdio, MdioConfig, MdioEvent};

/// MDC and MDIO on channels 0 and 1.
const CHANNELS: u8 = 2;
/// Samples per half MDC period: a bit takes four samples and rises at its third.
const HALF: u64 = 2;
/// Samples per bit.
const BIT: u64 = 2 * HALF;
/// Samples before the first bit.
const LEAD: u64 = 6;

fn harness(graph: GraphConfig) -> Harness<MdioEvent> {
    Harness::build(
        CHANNELS,
        TimeBase::new(1_000_000),
        graph,
        |graph, source| {
            let mdio = graph.add(Mdio::new(MdioConfig::default()).expect("a valid configuration"));
            graph.connect(source.samples, Mdio::input(mdio))?;
            Ok(Mdio::events(mdio))
        },
    )
    .expect("builds")
}

fn waveform() -> MdioWaveform {
    let mut waveform = MdioWaveform::new(MdioConfig::default(), CHANNELS, HALF, Tick::ZERO);
    waveform.idle(LEAD);
    waveform
}

fn decode(block: &LogicBlock) -> Vec<String> {
    let mut harness = harness(GraphConfig::default());
    harness.run_all(block);
    describe(&harness.without_progress())
}

fn decode_with_gaps(block: &LogicBlock, gaps: &[TickRange]) -> Vec<String> {
    let mut harness = harness(GraphConfig::default());
    harness.feed_with_gaps(block, gaps, &[61]);
    harness.end();
    describe(&harness.without_progress())
}

fn name(value: &impl Serialize) -> String {
    serde_json::to_value(value)
        .expect("serializes")
        .as_str()
        .expect("a name")
        .to_owned()
}

fn at(span: TickRange, settled: Tick) -> String {
    if span.is_empty() {
        format!("@{settled}")
    } else {
        format!("[{},{})@{settled}", span.start, span.end)
    }
}

/// One line per item, readable in assertions.
fn describe(items: &[Item<MdioEvent>]) -> Vec<String> {
    items
        .iter()
        .map(|item| match item {
            Item::Event(event) => {
                let what = match event.payload {
                    MdioEvent::Frame {
                        clause,
                        op,
                        phy,
                        reg,
                        turnaround,
                        data,
                        preamble_bits,
                    } => format!(
                        "frame c{} {} phy{phy} reg{reg} ta{turnaround:02b} data{data:04x} \
                         pre{preamble_bits}",
                        clause.number(),
                        name(&op)
                    ),
                    MdioEvent::Error { reason } => format!("error {}", name(&reason)),
                };
                format!("{what} {}", at(event.span, event.settled))
            }
            Item::Gap(range) => format!("gap {range}"),
            Item::Progress(tick) => format!("progress {tick}"),
            Item::End(tick) => format!("eof {tick}"),
        })
        .collect()
}

/// The tick at which bit `n` of what was sent from `from` on rises.
fn rise(from: Tick, n: u64) -> u64 {
    from.get() + n * BIT + HALF
}

/// The line of a frame sent from `from` after `preamble` ones, `what` describing its fields:
/// at most 32 ones are counted, and the frame spans from the first of them.
fn frame_line(what: &str, from: Tick, preamble: u64) -> String {
    let counted = preamble.min(32);
    let start = rise(from, preamble - counted);
    let last = rise(from, preamble + 31);
    format!("{what} pre{counted} [{start},{})@{last}", last + 1)
}

/// An error covering bits `first..=last` of what was sent from `from` on.
fn error_line(reason: &str, from: Tick, first: u64, last: u64) -> String {
    let last = rise(from, last);
    format!("error {reason} [{},{})@{last}", rise(from, first), last + 1)
}

#[test]
fn clause_22_reads_and_writes_cover_their_preamble_and_bits() {
    let mut waveform = waveform();
    let read_at = waveform.position();
    waveform.frame(32, &Frame::c22_read(1, 2, 0x1234));
    waveform.idle(10);
    let write_at = waveform.position();
    waveform.frame(32, &Frame::c22_write(0x1f, 0x10, 0xbeef));
    waveform.idle(10);
    let block = waveform.finish();
    let lines = decode(&block);
    assert_eq!(
        lines,
        [
            frame_line("frame c22 read phy1 reg2 ta10 data1234", read_at, 32),
            frame_line("frame c22 write phy31 reg16 ta10 databeef", write_at, 32),
            format!("eof {}", block.end()),
        ]
    );
    // The first preamble bit rises two samples after the lead, the last data bit 63 bits
    // later.
    assert_eq!(
        lines[0],
        "frame c22 read phy1 reg2 ta10 data1234 pre32 [8,261)@260"
    );
}

#[test]
fn clause_45_frames_name_their_operation() {
    let mut waveform = waveform();
    let frames = [
        Frame::c45_address(3, 1, 0x0007),
        Frame::c45_write(3, 1, 0x00ff),
        Frame::c45_read(3, 1, 0xa5a5),
        Frame::c45_read_increment(0x1f, 0x1e, 0x5a5a),
    ];
    let mut starts = Vec::new();
    for frame in &frames {
        starts.push(waveform.position());
        waveform.frame(32, frame);
        waveform.idle(3);
    }
    let block = waveform.finish();
    assert_eq!(
        decode(&block),
        [
            frame_line("frame c45 address phy3 reg1 ta10 data0007", starts[0], 32),
            frame_line("frame c45 write phy3 reg1 ta10 data00ff", starts[1], 32),
            frame_line("frame c45 read phy3 reg1 ta10 dataa5a5", starts[2], 32),
            frame_line(
                "frame c45 post-read-increment phy31 reg30 ta10 data5a5a",
                starts[3],
                32
            ),
            format!("eof {}", block.end()),
        ]
    );
}

#[test]
fn a_suppressed_preamble_is_taken_once_in_sync() {
    let mut waveform = waveform();
    let first = waveform.position();
    waveform.frame(32, &Frame::c22_read(1, 1, 0x796d));
    // MDC stops between frames, and the next one starts with ST.
    waveform.idle(20);
    let second = waveform.position();
    waveform.frame(0, &Frame::c22_read(1, 2, 0x0141));
    // Straight after it, with MDC running: no idle bit at all.
    let third = waveform.position();
    waveform.frame(0, &Frame::c45_read(2, 1, 0x0003));
    // One idle bit, then five.
    let fourth = waveform.position();
    waveform.frame(1, &Frame::c22_write(1, 0, 0x8000));
    let fifth = waveform.position();
    waveform.frame(5, &Frame::c22_read(1, 0, 0x1140));
    waveform.idle(8);
    let block = waveform.finish();
    assert_eq!(
        decode(&block),
        [
            frame_line("frame c22 read phy1 reg1 ta10 data796d", first, 32),
            frame_line("frame c22 read phy1 reg2 ta10 data0141", second, 0),
            frame_line("frame c45 read phy2 reg1 ta10 data0003", third, 0),
            frame_line("frame c22 write phy1 reg0 ta10 data8000", fourth, 1),
            frame_line("frame c22 read phy1 reg0 ta10 data1140", fifth, 5),
            format!("eof {}", block.end()),
        ]
    );
}

#[test]
fn ones_beyond_a_preamble_are_idle() {
    let mut waveform = waveform();
    let at = waveform.position();
    // MDC runs through a long idle: 100 ones, then the frame.
    waveform.frame(100, &Frame::c22_read(4, 5, 0x0bad));
    waveform.idle(4);
    let block = waveform.finish();
    let lines = decode(&block);
    assert_eq!(
        lines[0],
        frame_line("frame c22 read phy4 reg5 ta10 data0bad", at, 100)
    );
    // The span starts at the 69th one, 32 before ST.
    assert!(
        lines[0].contains(&format!("[{},", rise(at, 68))),
        "{lines:?}"
    );
}

#[test]
fn a_turnaround_that_breaks_its_rule_is_reported_and_the_frame_goes_on() {
    let mut waveform = waveform();
    let write_at = waveform.position();
    waveform.frame(32, &Frame::c22_write(1, 4, 0x01e1).with_turnaround(0b11));
    // Nobody answers the read: the line stays high through the turnaround and the data.
    let read_at = waveform.position();
    waveform.frame(32, &Frame::c22_read(9, 2, 0xffff).with_turnaround(0b11));
    // A device that drives already in the first bit: a read's first bit is nobody's.
    let early_at = waveform.position();
    waveform.frame(32, &Frame::c45_read(1, 1, 0x1234).with_turnaround(0b00));
    let address_at = waveform.position();
    waveform.frame(0, &Frame::c45_address(1, 1, 0x0007).with_turnaround(0b01));
    waveform.idle(4);
    let block = waveform.finish();
    assert_eq!(
        decode(&block),
        [
            error_line("turnaround", write_at, 32 + 14, 32 + 15),
            frame_line("frame c22 write phy1 reg4 ta11 data01e1", write_at, 32),
            error_line("turnaround", read_at, 32 + 14, 32 + 15),
            frame_line("frame c22 read phy9 reg2 ta11 dataffff", read_at, 32),
            frame_line("frame c45 read phy1 reg1 ta00 data1234", early_at, 32),
            error_line("turnaround", address_at, 14, 15),
            frame_line("frame c45 address phy1 reg1 ta01 data0007", address_at, 0),
            format!("eof {}", block.end()),
        ]
    );
}

#[test]
fn a_clause_22_start_without_a_clause_22_operation_is_no_frame() {
    let mut waveform = waveform();
    let bad_at = waveform.position();
    waveform.frame(
        32,
        &Frame {
            op: 0b11,
            ..Frame::c22_read(1, 2, 0x0000)
        },
    );
    // Out of sync now: a frame without a preamble is not decoded, one with a preamble is.
    waveform.frame(0, &Frame::c22_read(1, 3, 0x1234));
    let good_at = waveform.position();
    waveform.frame(32, &Frame::c22_read(1, 4, 0x5678));
    let other_at = waveform.position();
    waveform.frame(
        32,
        &Frame {
            op: 0b00,
            ..Frame::c22_write(1, 2, 0x0000)
        },
    );
    let last_at = waveform.position();
    waveform.frame(32, &Frame::c22_write(1, 5, 0x9abc));
    waveform.idle(4);
    let block = waveform.finish();
    assert_eq!(
        decode(&block),
        [
            error_line("start", bad_at, 32, 35),
            frame_line("frame c22 read phy1 reg4 ta10 data5678", good_at, 32),
            error_line("start", other_at, 32, 35),
            frame_line("frame c22 write phy1 reg5 ta10 data9abc", last_at, 32),
            format!("eof {}", block.end()),
        ]
    );
}

#[test]
fn a_frame_cut_by_the_end_of_the_samples_is_aborted() {
    let mut waveform = waveform();
    let at = waveform.position();
    waveform.ones(32);
    let bits = Frame::c22_read(1, 2, 0xabcd).bits();
    waveform.bits(bits[..20].iter().copied());
    let block = waveform.finish();
    let end = block.end().get();
    assert_eq!(
        decode(&block),
        [
            format!("error aborted [{},{})@{end}", rise(at, 0), rise(at, 51) + 1),
            format!("eof {end}"),
        ]
    );
}

#[test]
fn a_gap_aborts_the_frame_and_sync_waits_for_a_preamble() {
    let mut waveform = waveform();
    let first = waveform.position();
    waveform.frame(32, &Frame::c22_read(1, 2, 0xabcd));
    let second = waveform.position();
    waveform.frame(0, &Frame::c22_read(1, 3, 0x1111));
    // After the gap, a frame without a preamble is not decoded.
    waveform.frame(0, &Frame::c22_read(1, 4, 0x2222));
    let fourth = waveform.position();
    waveform.frame(32, &Frame::c22_read(1, 5, 0x3333));
    waveform.idle(4);
    let block = waveform.finish();
    // Lose bits 10 to 12 of the second frame.
    let gap = TickRange::new(Tick(rise(second, 10) - 1), Tick(rise(second, 13) - 1));
    assert_eq!(
        decode_with_gaps(&block, &[gap]),
        [
            frame_line("frame c22 read phy1 reg2 ta10 dataabcd", first, 32),
            format!(
                "error aborted [{},{})@{}",
                rise(second, 0),
                rise(second, 9) + 1,
                gap.start
            ),
            format!("gap {gap}"),
            frame_line("frame c22 read phy1 reg5 ta10 data3333", fourth, 32),
            format!("eof {}", block.end()),
        ]
    );
}

#[test]
fn a_gap_in_the_preamble_loses_the_ones_before_it() {
    let mut waveform = waveform();
    let at = waveform.position();
    waveform.frame(32, &Frame::c22_read(1, 2, 0x4321));
    waveform.idle(4);
    let again = waveform.position();
    waveform.frame(40, &Frame::c22_read(1, 2, 0x4321));
    waveform.idle(4);
    let block = waveform.finish();
    // A gap that swallows the 21st one of the first preamble leaves 11 after it, too few for
    // sync, and the 20 before it do not count. One that swallows the 7th one of the second
    // leaves 33.
    let gaps = [
        TickRange::new(Tick(rise(at, 19) + 1), Tick(rise(at, 20) + 1)),
        TickRange::new(Tick(rise(again, 5) + 1), Tick(rise(again, 6) + 1)),
    ];
    assert_eq!(
        decode_with_gaps(&block, &gaps),
        [
            format!("gap {}", gaps[0]),
            format!("gap {}", gaps[1]),
            frame_line("frame c22 read phy1 reg2 ta10 data4321", again, 40),
            format!("eof {}", block.end()),
        ]
    );
}

#[test]
fn a_frame_goes_out_with_its_last_bit_and_a_turnaround_error_before() {
    let mut waveform = waveform();
    let at = waveform.position();
    waveform.frame(32, &Frame::c22_write(2, 3, 0x4444).with_turnaround(0b01));
    waveform.idle(20);
    let block = waveform.finish();
    let piece = |from: Tick, to: Tick| block.slice(TickRange::new(from, to)).expect("a slice");
    let mut harness = harness(GraphConfig::default());
    // Into the data: the broken turnaround is out, the frame is not.
    let middle = Tick(rise(at, 32 + 20));
    harness.feed(&piece(Tick::ZERO, middle), &[13]);
    harness.collect();
    let turnaround = error_line("turnaround", at, 32 + 14, 32 + 15);
    assert_eq!(
        describe(&harness.without_progress()),
        std::slice::from_ref(&turnaround)
    );
    // Through the last bit's rising edge: the frame is out, before the samples end.
    let last = Tick(rise(at, 63) + 1);
    harness.feed(&piece(middle, last), &[13]);
    harness.collect();
    assert_eq!(
        describe(&harness.without_progress()),
        [
            turnaround,
            frame_line("frame c22 write phy2 reg3 ta01 data4444", at, 32),
        ]
    );
}

#[test]
fn a_frame_under_way_at_the_first_sample_is_skipped() {
    let mut waveform = MdioWaveform::new(MdioConfig::default(), CHANNELS, HALF, Tick::ZERO);
    // The tail of a write whose data is all ones, then a frame without a preamble: sixteen
    // ones are no preamble.
    let tail = Frame::c22_write(1, 2, 0xffff).bits();
    waveform.bits(tail[12..].iter().copied());
    waveform.frame(0, &Frame::c22_read(1, 3, 0x0001));
    let at = waveform.position();
    waveform.frame(32, &Frame::c22_read(1, 4, 0x0002));
    waveform.idle(4);
    let block = waveform.finish();
    assert_eq!(
        decode(&block),
        [
            frame_line("frame c22 read phy1 reg4 ta10 data0002", at, 32),
            format!("eof {}", block.end()),
        ]
    );
}

/// Two rounds of frames of every kind and preamble length, idle time with and without MDC,
/// broken turnarounds and starts; with the tick each frame is sent from, preamble included.
fn rich_waveform() -> (LogicBlock, Vec<Tick>) {
    let mut waveform = waveform();
    let frames = [
        (32, Frame::c22_read(0, 1, 0x796d)),
        (0, Frame::c22_read(0, 2, 0x0141)),
        (1, Frame::c22_write(0, 0, 0x9140)),
        (64, Frame::c45_address(2, 3, 0x0010)),
        (0, Frame::c45_write(2, 3, 0x1234)),
        (3, Frame::c45_read(2, 3, 0xffff)),
        (0, Frame::c45_read_increment(2, 3, 0x0000)),
        (32, Frame::c22_write(5, 9, 0x0f0f).with_turnaround(0b00)),
        (
            32,
            Frame {
                op: 0b11,
                ..Frame::c22_read(1, 1, 0x5555)
            },
        ),
        (0, Frame::c22_read(1, 1, 0xaaaa)),
        (33, Frame::c22_read(31, 31, 0xffff).with_turnaround(0b11)),
        (2, Frame::c45_address(7, 1, 0xfffe)),
    ];
    let mut marks = Vec::new();
    for round in 0..2u64 {
        for (n, (preamble, frame)) in frames.iter().enumerate() {
            marks.push(waveform.position());
            waveform.frame(*preamble, frame);
            if n % 3 == 0 {
                waveform.idle(5 + n as u64 + round);
            }
        }
    }
    waveform.ones(7);
    waveform.idle(9);
    (waveform.finish(), marks)
}

#[test]
fn every_cut_and_pause_yields_the_same_events() {
    let (block, marks) = rich_waveform();
    let gaps = [
        // Inside the data of a frame without a preamble: the next frame with one resyncs.
        TickRange::new(Tick(rise(marks[1], 20) - 1), Tick(rise(marks[1], 22))),
        // Inside a long preamble, which still has more than 32 ones after the gap.
        TickRange::new(Tick(rise(marks[3], 10) + 1), Tick(rise(marks[3], 12) + 1)),
        // In idle time with MDC stopped, before a frame with a preamble.
        TickRange::new(Tick(marks[7].get() - 9), Tick(marks[7].get() - 3)),
        // From the second round's broken start into the frame after it.
        TickRange::new(Tick(rise(marks[20], 50)), Tick(rise(marks[21], 3))),
        // In the last frame's data.
        TickRange::new(Tick(rise(marks[23], 30) + 1), Tick(rise(marks[23], 31) + 2)),
    ];
    let reference = decode_with_gaps(&block, &gaps);
    assert!(
        reference
            .iter()
            .filter(|line| line.starts_with("frame "))
            .count()
            >= 12,
        "{reference:?}"
    );
    for what in ["error turnaround", "error start", "error aborted", "gap "] {
        assert!(
            reference.iter().any(|line| line.starts_with(what)),
            "{what}: {reference:?}"
        );
    }
    assert_block_invariant(harness, &block, &gaps);
}

#[test]
fn progress_never_passes_a_frame_under_way() {
    let (block, _) = rich_waveform();
    for sizes in [&[1u64][..], &[3], &[7, 64], &[97]] {
        let mut harness = harness(GraphConfig::default());
        harness.feed_cut(&block, &[], sizes, &[1]);
        harness.end();
        let mut seen = Tick::ZERO;
        let mut progress = 0;
        for item in &harness.items {
            match item {
                Item::Progress(tick) => {
                    seen = seen.max(*tick);
                    progress += 1;
                }
                Item::Event(event) => {
                    assert!(event.span.start >= seen, "{event:?} after {seen}");
                }
                Item::Gap(range) => assert!(range.start >= seen, "{range} after {seen}"),
                Item::End(tick) => assert!(*tick >= seen),
            }
        }
        assert!(progress > 10, "blocks of {sizes:?}");
    }
}

/// One frame of a random scenario and the time after it.
#[derive(Clone, Debug)]
struct Sent {
    preamble: u32,
    frame: Frame,
    /// Samples with MDC stopped after the frame.
    idle: u64,
}

/// The six starts that make a frame, as ST and OP codes, with the line the decoder writes for
/// them and whether the device drives the data.
const STARTS: [(u8, u8, &str, bool); 6] = [
    (0b01, 0b10, "c22 read", true),
    (0b01, 0b01, "c22 write", false),
    (0b00, 0b00, "c45 address", false),
    (0b00, 0b01, "c45 write", false),
    (0b00, 0b11, "c45 read", true),
    (0b00, 0b10, "c45 post-read-increment", true),
];

/// A frame of any codes, the starts of no frame among them, after any preamble.
fn any_sent() -> impl Strategy<Value = Sent> {
    (
        prop_oneof![3 => Just(32u32), 3 => 0u32..4, 1 => 0u32..80],
        0u8..4,
        0u8..4,
        any::<u8>(),
        any::<u8>(),
        prop_oneof![3 => Just(0b10u8), 1 => 0u8..4],
        any::<u16>(),
        0u64..20,
    )
        .prop_map(
            |(preamble, st, op, phy, reg, turnaround, data, idle)| Sent {
                preamble,
                frame: Frame {
                    st,
                    op,
                    phy,
                    reg,
                    turnaround,
                    data,
                },
                idle,
            },
        )
}

/// A well-formed frame after a preamble of up to 40 ones.
fn good_sent() -> impl Strategy<Value = (usize, Sent)> {
    (
        0usize..STARTS.len(),
        0u32..41,
        0u8..32,
        0u8..32,
        prop_oneof![3 => Just(0b10u8), 1 => 0u8..4],
        any::<u16>(),
        0u64..20,
    )
        .prop_map(|(start, preamble, phy, reg, turnaround, data, idle)| {
            let (st, op, _, _) = STARTS[start];
            let frame = Frame {
                st,
                op,
                phy,
                reg,
                turnaround,
                data,
            };
            (
                start,
                Sent {
                    preamble,
                    frame,
                    idle,
                },
            )
        })
}

fn scenario_waveform(sent: &[Sent]) -> LogicBlock {
    let mut waveform = waveform();
    for sent in sent {
        waveform.frame(sent.preamble, &sent.frame);
        waveform.idle(sent.idle);
    }
    waveform.ones(3);
    waveform.idle(5);
    waveform.finish()
}

proptest! {
    #![proptest_config(ProptestConfig {
        // The default is enough for every run; PROPTEST_CASES raises it for a stress run.
        cases: std::env::var("PROPTEST_CASES").ok().and_then(|cases| cases.parse().ok()).unwrap_or(64),
        ..ProptestConfig::default()
    })]

    /// Random traffic cut into blocks of any size, read with random pauses through tiny
    /// queues and budgets, with or without a gap, yields the events of one uninterrupted run.
    #[test]
    fn blocks_pauses_queues_and_budgets_do_not_change_the_events(
        sent in prop::collection::vec(any_sent(), 1..8),
        sizes in prop::collection::vec(1u64..300, 1..6),
        pauses in prop::collection::vec(0usize..12, 1..8),
        queue_capacity in 1usize..6,
        budget in 1u32..40,
        gap in prop::option::of((0.05f64..0.95, 1u64..200)),
    ) {
        let block = scenario_waveform(&sent);
        let gaps: Vec<TickRange> = gap
            .map(|(at, len)| {
                let start = ((block.len() as f64) * at) as u64;
                TickRange::new(Tick(start), Tick((start + len).min(block.len())))
            })
            .into_iter()
            .filter(|gap| !gap.is_empty())
            .collect();
        let reference = {
            let mut harness = harness(GraphConfig::default());
            harness.feed_with_gaps(&block, &gaps, &[u64::MAX]);
            harness.end();
            harness.without_progress()
        };
        let mut harness = harness(GraphConfig { queue_capacity, budget });
        harness.drain_on_full = 1;
        harness.feed_cut(&block, &gaps, &sizes, &pauses);
        harness.end();
        prop_assert_eq!(harness.without_progress(), reference);
    }

    /// Well-formed frames after a first full preamble are decoded as they were sent, whatever
    /// preamble each of the others has, with an error for every turnaround that breaks its
    /// rule.
    #[test]
    fn frames_in_sync_are_decoded_as_sent(sent in prop::collection::vec(good_sent(), 1..10)) {
        let mut frames: Vec<Sent> = sent.iter().map(|(_, sent)| sent.clone()).collect();
        frames[0].preamble = frames[0].preamble.max(32);
        let block = scenario_waveform(&frames);
        let mut expected = Vec::new();
        for ((start, _), sent) in sent.iter().zip(&frames) {
            let (_, _, name, device_drives) = STARTS[*start];
            let frame = sent.frame;
            let turnaround_ok = if device_drives {
                frame.turnaround & 1 == 0
            } else {
                frame.turnaround == 0b10
            };
            if !turnaround_ok {
                expected.push("error turnaround".to_owned());
            }
            expected.push(format!(
                "frame {name} phy{} reg{} ta{:02b} data{:04x} pre{}",
                frame.phy,
                frame.reg,
                frame.turnaround,
                frame.data,
                sent.preamble.min(32)
            ));
        }
        let lines: Vec<String> = decode(&block)
            .into_iter()
            .filter(|line| !line.starts_with("eof "))
            .map(|line| {
                line.rsplit_once(' ')
                    .map_or(line.as_str(), |(what, _)| what)
                    .to_owned()
            })
            .collect();
        prop_assert_eq!(lines, expected);
    }
}
