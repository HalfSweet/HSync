// SPDX-License-Identifier: Apache-2.0

//! The Wiegand decoder on synthesized waveforms.

use std::sync::Arc;

use uiharu_core::testing::{Harness, assert_block_invariant, without_progress};
use uiharu_core::{
    GraphConfig, GraphError, Item, LogicBlock, SignalItem, Tick, TickRange, TimeBase,
};
use uiharu_protocols::wiegand::synth::{WiegandWaveform, bits_of, card26};
use uiharu_protocols::wiegand::{Wiegand, WiegandConfig, WiegandEvent};

const CHANNELS: u8 = 3;
/// 1 MHz: a tick is a microsecond.
const RATE: u64 = 1_000_000;
/// The default frame gap, in ticks.
const GAP: u64 = 10_000;
/// The default spacing of the synthesizer's bits, in ticks.
const BIT: u64 = 1_000;
/// The default width of its pulses, in ticks.
const PULSE: u64 = 50;

/// DATA0 on channel 1 and DATA1 on channel 2; channel 0 stays low.
fn config() -> WiegandConfig {
    WiegandConfig {
        d0: 1,
        d1: 2,
        ..WiegandConfig::default()
    }
}

fn build(config: WiegandConfig, rate: u64, graph: GraphConfig) -> Harness<WiegandEvent> {
    Harness::build(CHANNELS, TimeBase::new(rate), graph, |graph, source| {
        let wiegand = graph.add(Wiegand::new(config).expect("a valid configuration"));
        graph.connect(source.samples, Wiegand::input(wiegand))?;
        Ok(Wiegand::events(wiegand))
    })
    .expect("builds")
}

/// A waveform that starts with the lines idle long enough for the decoder to trust them.
fn new_waveform() -> WiegandWaveform {
    let mut waveform = WiegandWaveform::new(config(), CHANNELS, TimeBase::new(RATE), Tick::ZERO);
    waveform.idle(GAP + 2_000);
    waveform
}

fn at(span: TickRange, settled: Tick) -> String {
    if span.is_empty() {
        format!("@{settled}")
    } else {
        format!("[{},{})@{settled}", span.start, span.end)
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// One line per item, progress left out: it depends on how the samples were cut.
fn describe(items: &[Item<WiegandEvent>]) -> Vec<String> {
    without_progress(items)
        .iter()
        .map(|item| match item {
            Item::Event(event) => {
                let what = match &event.payload {
                    WiegandEvent::Frame {
                        frame,
                        bits,
                        data,
                        parity_ok,
                    } => {
                        let parity = match parity_ok {
                            Some(true) => " parity ok",
                            Some(false) => " parity bad",
                            None => "",
                        };
                        format!("frame f{} {bits} {}{parity}", frame.0, hex(data))
                    }
                    WiegandEvent::Error { reason } => format!("error {reason:?}"),
                };
                format!("{what} {}", at(event.span, event.settled))
            }
            Item::Gap(range) => format!("gap {range}"),
            Item::Progress(tick) => format!("progress {tick}"),
            Item::End(tick) => format!("eof {tick}"),
        })
        .collect()
}

fn decode(block: &LogicBlock) -> Vec<String> {
    let mut harness = build(config(), RATE, GraphConfig::default());
    harness.run_all(block);
    describe(&harness.items)
}

/// The bits written out as `0` and `1`, for the frames' expected data.
fn from_text(text: &str) -> Vec<bool> {
    text.chars().map(|c| c == '1').collect()
}

#[test]
fn a_26_bit_frame_and_its_parities() {
    // Facility 123 is 0111 1011 and card 12345 is 0011 0000 0011 1001. The first twelve of
    // those bits hold eight ones, so the even parity bit in front is 0; the last twelve hold
    // four, so the odd parity bit behind is 1. Packed from the top: 0011 1101, 1001 1000,
    // 0001 1100, 11 and six zeros.
    let card = from_text("0 01111011 0011000000111001 1".replace(' ', "").as_str());
    assert_eq!(card, card26(123, 12_345), "the synthesizer agrees");
    let mut waveform = new_waveform();
    let start = waveform.position().get();
    waveform.bits(card.iter().copied());
    waveform.idle(GAP + 5_000);
    let block = waveform.finish();
    let release = start + 25 * BIT + PULSE;
    assert_eq!(
        decode(&block),
        vec![
            format!(
                "frame f1 26 3d981cc0 parity ok [{start},{})@{}",
                release + 1,
                release + GAP
            ),
            format!("eof {}", block.end()),
        ]
    );

    // The odd parity bit flipped, then the even one.
    for flip in [25, 0] {
        let mut bad = card.clone();
        bad[flip] = !bad[flip];
        let mut waveform = new_waveform();
        waveform.bits(bad);
        waveform.idle(GAP + 5_000);
        let lines = decode(&waveform.finish());
        assert!(lines[0].contains(" parity bad "), "{lines:?}");
    }
}

#[test]
fn a_34_bit_frame_is_reported_as_it_is() {
    let value = 0x2_d1b2_c3e5u64;
    let mut waveform = new_waveform();
    let start = waveform.position().get();
    waveform.bits(bits_of(value, 34));
    waveform.idle(GAP + 1_000);
    let block = waveform.finish();
    let release = start + 33 * BIT + PULSE;
    // 34 bits from the top: 1011 0100 0110 1100 1011 0000 1111 1001 01 and six zeros.
    assert_eq!(
        decode(&block)[0],
        format!(
            "frame f1 34 b46cb0f940 [{start},{})@{}",
            release + 1,
            release + GAP
        )
    );
}

#[test]
fn frames_end_at_a_silence_longer_than_the_gap() {
    let mut waveform = new_waveform();
    let first = waveform.position().get();
    waveform.bits(bits_of(0b101, 3));
    // From the release of the last pulse: 950 ticks to the end of its bit time, then a silence
    // that ends exactly at the frame gap. The next pulse falls frame_gap ticks after the
    // release: the same frame.
    let release = first + 2 * BIT + PULSE;
    waveform.idle(GAP - (BIT - PULSE));
    assert_eq!(waveform.position().get(), release + GAP);
    waveform.bits(bits_of(0b01, 2));
    // One tick more and it is the next frame.
    let release = release + GAP + BIT + PULSE;
    waveform.idle(GAP + 1 - (BIT - PULSE));
    let second = waveform.position().get();
    assert_eq!(second, release + GAP + 1);
    waveform.bits(bits_of(0b1, 1));
    waveform.idle(GAP + 1_000);
    let block = waveform.finish();
    let lines = decode(&block);
    assert_eq!(
        lines,
        vec![
            // 1 0 1 0 1, five bits: 1010 1000.
            format!("frame f1 5 a8 [{first},{})@{}", release + 1, release + GAP),
            format!(
                "frame f2 1 80 [{second},{})@{}",
                second + PULSE + 1,
                second + PULSE + GAP
            ),
            format!("eof {}", block.end()),
        ]
    );
}

/// Two 26-bit frames 15 ms apart.
fn two_frames() -> (LogicBlock, Tick) {
    let mut waveform = new_waveform();
    waveform.bits(card26(1, 2));
    waveform.idle(15_000);
    let second = waveform.position();
    waveform.bits(card26(3, 4));
    waveform.idle(GAP + 2_000);
    (waveform.finish(), second)
}

#[test]
fn a_frame_ends_when_its_silence_passes_whatever_tells_the_decoder() {
    let (block, second) = two_frames();
    let reference = decode(&block);
    assert_eq!(reference.len(), 3, "{reference:?}");
    // Up to just before the second frame: the first frame's silence has passed in the samples
    // already, before any pulse of the next one.
    let head = block
        .slice(TickRange::new(Tick::ZERO, second))
        .expect("a slice");
    let mut harness = build(config(), RATE, GraphConfig::default());
    harness.feed(&head, &[700]);
    harness.collect();
    assert_eq!(describe(&harness.items), reference[..1]);
    let tail = block
        .slice(TickRange::new(second, block.end()))
        .expect("a slice");
    harness.feed(&tail, &[700]);
    harness.end();
    assert_eq!(describe(&harness.items), reference);
    // Up to one tick short of the silence: the frame is not over yet.
    let silence = reference[0]
        .rsplit('@')
        .next()
        .and_then(|tick| tick.parse::<u64>().ok())
        .expect("a settled tick");
    let short = block
        .slice(TickRange::new(Tick::ZERO, Tick(silence)))
        .expect("a slice");
    let mut harness = build(config(), RATE, GraphConfig::default());
    harness.feed(&short, &[333]);
    harness.collect();
    assert!(describe(&harness.items).is_empty());
    // One more sample decides it.
    harness.push(SignalItem::Data(Arc::new(
        block
            .slice(TickRange::new(Tick(silence), Tick(silence + 1)))
            .expect("a slice"),
    )));
    harness.collect();
    assert_eq!(describe(&harness.items), reference[..1]);
    // The source announcing that its clock has passed the silence decides it too.
    let mut harness = build(config(), RATE, GraphConfig::default());
    harness.feed(&short, &[333]);
    harness.push(SignalItem::Progress(Tick(silence + 50)));
    harness.collect();
    assert_eq!(describe(&harness.items), reference[..1]);
    // However the samples are cut and read.
    assert_block_invariant(|graph| build(config(), RATE, graph), &block, &[]);
}

#[test]
fn a_pulse_on_both_lines_aborts_the_frame() {
    let mut waveform = new_waveform();
    let start = waveform.position().get();
    waveform.bits(bits_of(0b1100_1010, 8));
    let both = waveform.position().get();
    waveform.pulse(true, true, PULSE);
    waveform.idle(BIT - PULSE);
    // The rest of the broken frame is skipped up to the silence.
    waveform.bits(bits_of(0b1011, 4));
    waveform.idle(GAP + 500);
    let next = waveform.position().get();
    waveform.bits(card26(7, 77));
    waveform.idle(GAP + 500);
    let lines = decode(&waveform.finish());
    assert_eq!(
        lines[0],
        format!("error BothLines [{start},{})@{both}", both + 1)
    );
    assert!(lines[1].starts_with("frame f1 26 "), "{lines:?}");
    assert!(lines[1].contains(&format!(" [{next},")), "{lines:?}");
    assert_eq!(lines.len(), 3, "{lines:?}");

    // A pulse on one line that the other joins.
    let mut waveform = new_waveform();
    let start = waveform.position().get();
    waveform.pulse(true, false, 20);
    waveform.pulse(true, true, 20);
    waveform.idle(GAP + 500);
    let lines = decode(&waveform.finish());
    assert_eq!(
        lines[0],
        format!("error BothLines [{start},{})@{}", start + 21, start + 20)
    );
    assert_eq!(lines.len(), 2, "{lines:?}");
}

#[test]
fn pulses_outside_the_width_window_are_errors() {
    // 19 µs is too short; 20 µs is not.
    let mut waveform = new_waveform();
    let start = waveform.position().get();
    waveform.bits(bits_of(0b10, 2));
    let short = waveform.position().get();
    waveform.pulse(false, true, 19);
    waveform.idle(GAP + 100);
    let ok = waveform.position().get();
    waveform.pulse(false, true, 20);
    waveform.idle(GAP + 100);
    let lines = decode(&waveform.finish());
    assert_eq!(
        lines[..2],
        [
            format!("error ShortPulse [{start},{})@{}", short + 20, short + 19),
            format!("frame f1 1 80 [{ok},{})@{}", ok + 21, ok + 20 + GAP),
        ]
    );

    // A pulse of 101 µs is too long, decided once it outlasts 100 µs whatever follows; one of
    // 100 µs is not.
    let mut waveform = new_waveform();
    let long = waveform.position().get();
    waveform.pulse(true, false, 101);
    waveform.idle(GAP + 100);
    let ok = waveform.position().get();
    waveform.pulse(true, false, 100);
    waveform.idle(GAP + 100);
    let block = waveform.finish();
    let lines = decode(&block);
    assert_eq!(
        lines[..2],
        [
            format!("error LongPulse [{long},{})@{}", long + 101, long + 100),
            format!("frame f1 1 00 [{ok},{})@{}", ok + 101, ok + 100 + GAP),
        ]
    );
    // The long pulse is decided before it ends.
    let head = block
        .slice(TickRange::new(Tick::ZERO, Tick(long + 101)))
        .expect("a slice");
    let mut harness = build(config(), RATE, GraphConfig::default());
    harness.feed(&head, &[64]);
    harness.collect();
    assert_eq!(describe(&harness.items), lines[..1]);
}

#[test]
fn a_frame_longer_than_64_bits_is_an_error() {
    let mut waveform = new_waveform();
    let start = waveform.position().get();
    waveform.bits((0..70).map(|bit| bit % 3 == 0));
    waveform.idle(GAP + 100);
    let next = waveform.position().get();
    waveform.bits(bits_of(0b11, 2));
    waveform.idle(GAP + 100);
    let lines = decode(&waveform.finish());
    let release = start + 64 * BIT + PULSE;
    assert_eq!(
        lines[0],
        format!("error TooLong [{start},{})@{release}", release + 1)
    );
    // The six bits after it belong to the broken frame.
    assert!(
        lines[1].starts_with(&format!("frame f1 2 c0 [{next},")),
        "{lines:?}"
    );

    // 64 bits are a frame.
    let mut waveform = new_waveform();
    waveform.bits(bits_of(0x8000_0000_0000_0001, 64));
    waveform.idle(GAP + 100);
    let lines = decode(&waveform.finish());
    assert!(
        lines[0].starts_with("frame f1 64 8000000000000001 "),
        "{lines:?}"
    );
}

#[test]
fn the_end_of_the_samples_before_the_silence_aborts_the_frame() {
    let mut waveform = new_waveform();
    let start = waveform.position().get();
    waveform.bits(card26(9, 999));
    // The samples stop 5 ms after the last pulse.
    waveform.idle(5_000 - (BIT - PULSE));
    let block = waveform.finish();
    let release = start + 25 * BIT + PULSE;
    let end = block.end().get();
    assert_eq!(
        decode(&block),
        vec![
            format!("error Aborted [{start},{})@{end}", release + 1),
            format!("eof {end}"),
        ]
    );
    // Cut inside a pulse.
    let cut = block
        .slice(TickRange::new(Tick::ZERO, Tick(start + 3 * BIT + 10)))
        .expect("a slice");
    assert_eq!(
        decode(&cut)[0],
        format!(
            "error Aborted [{start},{})@{}",
            start + 3 * BIT + 1,
            start + 3 * BIT + 10
        )
    );
}

#[test]
fn a_gap_aborts_the_frame_and_the_lines_must_idle_again() {
    let mut waveform = new_waveform();
    let start = waveform.position().get();
    waveform.bits(card26(5, 55));
    waveform.idle(GAP + 3_000);
    let next = waveform.position().get();
    waveform.bits(card26(6, 66));
    waveform.idle(GAP + 3_000);
    let block = waveform.finish();
    // The gap swallows bits 10 and 11 of the first frame; bits 12 to 25 come after it, and are
    // skipped: the lines have not been idle since.
    let gap = TickRange::new(Tick(start + 9 * BIT + 500), Tick(start + 11 * BIT + 500));
    let mut harness = build(config(), RATE, GraphConfig::default());
    harness.feed_with_gaps(&block, &[gap], &[1_000]);
    harness.end();
    let lines = describe(&harness.items);
    assert_eq!(
        lines[..2],
        [
            format!(
                "error Aborted [{start},{})@{}",
                start + 9 * BIT + PULSE + 1,
                gap.start
            ),
            format!("gap {gap}"),
        ]
    );
    assert!(
        lines[2].starts_with("frame f1 26 ") && lines[2].contains(&format!("[{next},")),
        "{lines:?}"
    );
    assert_eq!(lines.len(), 4, "{lines:?}");
}

#[test]
fn a_frame_under_way_when_the_samples_begin_is_skipped() {
    let mut waveform = new_waveform();
    let start = waveform.position().get();
    waveform.bits(card26(10, 1000));
    waveform.idle(GAP + 500);
    let next = waveform.position().get();
    waveform.bits(card26(11, 1100));
    waveform.idle(GAP + 500);
    let block = waveform.finish();
    for from in [start + 7 * BIT + 20, start + 7 * BIT + 500] {
        let tail = block
            .slice(TickRange::new(Tick(from), block.end()))
            .expect("a slice");
        let lines = decode(&tail);
        assert_eq!(lines.len(), 2, "{lines:?}");
        assert!(
            lines[0].starts_with("frame f1 26 ") && lines[0].contains(&format!("[{next},")),
            "{lines:?}"
        );
    }
    // Lines seen idle for less than the frame gap before a frame are not trusted either.
    let tail = block
        .slice(TickRange::new(Tick(start - 5_000), block.end()))
        .expect("a slice");
    let lines = decode(&tail);
    assert_eq!(lines.len(), 2, "{lines:?}");
    assert!(lines[0].contains(&format!("[{next},")), "{lines:?}");
}

/// Frames of every kind, good and broken, and where each 26-bit frame begins.
fn busy_line() -> (LogicBlock, Vec<u64>) {
    let mut waveform = new_waveform();
    let mut starts = Vec::new();
    for n in 0..6u16 {
        starts.push(waveform.position().get());
        waveform.bits(card26(n as u8 * 40, n * 1_111));
        waveform.idle(GAP + 300 * u64::from(n) + 7);
        match n % 3 {
            0 => waveform.pulse(true, true, 40),
            1 => waveform.pulse(false, true, 5 + u64::from(n)),
            _ => waveform.pulse(true, false, 150),
        }
        waveform.idle(GAP + 50);
        waveform.bits(bits_of(u64::from(n) * 0x1234_5679, 34));
        waveform.idle(GAP + 200);
    }
    (waveform.finish(), starts)
}

#[test]
fn every_cut_and_pause_yields_the_same_events() {
    let (block, starts) = busy_line();
    let reference = decode(&block);
    for kind in [
        "frame f",
        "error BothLines",
        "error ShortPulse",
        "error LongPulse",
    ] {
        assert!(
            reference.iter().any(|line| line.starts_with(kind)),
            "{kind} in {reference:?}"
        );
    }
    let release = |n: usize| starts[n] + 25 * BIT + PULSE;
    let gaps = [
        // Inside frame 1, in frame 2's silence before it passed, and in frame 3's after.
        TickRange::new(Tick(starts[1] + 5_500), Tick(starts[1] + 7_200)),
        TickRange::new(Tick(release(2) + 3_000), Tick(release(2) + 4_000)),
        TickRange::new(Tick(release(3) + GAP + 100), Tick(release(3) + GAP + 200)),
    ];
    let mut harness = build(config(), RATE, GraphConfig::default());
    harness.feed_with_gaps(&block, &gaps, &[u64::MAX]);
    harness.end();
    let gapped = describe(&harness.items);
    let aborted: Vec<&String> = gapped
        .iter()
        .filter(|line| line.starts_with("error Aborted"))
        .collect();
    assert_eq!(aborted.len(), 2, "{gapped:?}");
    assert!(
        gapped
            .iter()
            .any(|line| line.contains(&format!("[{},", starts[3]))),
        "frame 3 is whole: {gapped:?}"
    );
    assert_block_invariant(|graph| build(config(), RATE, graph), &block, &gaps);
}

#[test]
fn progress_never_passes_a_frame_not_yet_reported() {
    let (block, _) = busy_line();
    for sizes in [&[1u64][..], &[613], &[64, 7_001]] {
        let mut harness = build(config(), RATE, GraphConfig::default());
        harness.feed_cut(&block, &[], sizes, &[1]);
        harness.end();
        let mut seen = Tick::ZERO;
        let mut progressed = false;
        for item in &harness.items {
            match item {
                Item::Progress(tick) => {
                    progressed |= *tick > Tick::ZERO;
                    seen = seen.max(*tick);
                }
                Item::Event(event) => {
                    assert!(event.span.start >= seen, "{event:?} after {seen}");
                }
                Item::Gap(_) | Item::End(_) => {}
            }
        }
        assert!(progressed, "blocks of {sizes:?} announce progress");
    }
}

#[test]
fn a_source_too_slow_for_the_shortest_pulse_is_refused() {
    let result = Harness::build(
        CHANNELS,
        TimeBase::new(99_999),
        GraphConfig::default(),
        |graph, source| {
            let wiegand = graph.add(Wiegand::new(config()).expect("a valid configuration"));
            graph.connect(source.samples, Wiegand::input(wiegand))?;
            Ok(Wiegand::events(wiegand))
        },
    );
    match result {
        Err(GraphError::Requirement { key, .. }) => assert_eq!(key, "rateHz"),
        other => panic!("expected the rate to be refused, got {other:?}"),
    }
}

#[test]
fn other_rates_and_timings_decode_alike() {
    // 250 kHz, 30 µs pulses 2 ms apart, a 5 ms frame gap.
    let config = WiegandConfig {
        frame_gap_us: 5_000,
        ..config()
    };
    let rate = 250_000;
    let mut waveform = WiegandWaveform::new(config, CHANNELS, TimeBase::new(rate), Tick::ZERO);
    waveform.set_timing(uiharu_protocols::wiegand::synth::Timing {
        pulse: 30_000,
        interval: 2_000_000,
    });
    waveform.idle_us(6_000);
    let start = waveform.position().get();
    waveform.bits(card26(123, 12_345));
    waveform.idle_us(8_000);
    let block = waveform.finish();
    let mut harness = build(config, rate, GraphConfig::default());
    harness.run_all(&block);
    // A tick is 4 µs: the last pulse falls 25 × 500 ticks after the first and lasts 7 ticks
    // (30 µs rounded down); the silence is 1250 ticks.
    let release = start + 25 * 500 + 7;
    assert_eq!(
        describe(&harness.items)[0],
        format!(
            "frame f1 26 3d981cc0 parity ok [{start},{})@{}",
            release + 1,
            release + 1_250
        )
    );
}
