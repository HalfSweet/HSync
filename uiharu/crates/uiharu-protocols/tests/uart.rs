// SPDX-License-Identifier: Apache-2.0

//! The UART decoder on synthesized waveforms.

use proptest::prelude::*;
use uiharu_core::testing::{Harness, assert_block_invariant};
use uiharu_core::{GraphConfig, GraphError, Item, LogicBlock, Tick, TickRange, TimeBase};
use uiharu_protocols::uart::synth::{Faults, UartWaveform};
use uiharu_protocols::uart::{
    BitOrder, Parity, ParityCheck, StopBits, Uart, UartConfig, UartEvent,
};

const CHANNELS: u8 = 2;
/// Samples per bit in most tests: 100 000 baud sampled at 1 MHz.
const BIT: u64 = 10;
const BAUD: u32 = 100_000;
const RATE: u64 = 1_000_000;

/// 8N1 at 100 000 baud on channel 1.
fn config() -> UartConfig {
    UartConfig {
        line: 1,
        baud: BAUD,
        ..UartConfig::default()
    }
}

/// Source → UART on a source of `rate` samples per second.
fn try_build(
    config: UartConfig,
    rate: u64,
    graph_config: GraphConfig,
) -> Result<Harness<UartEvent>, GraphError> {
    Harness::build(
        CHANNELS,
        TimeBase::new(rate),
        graph_config,
        |graph, source| {
            let uart = graph.add(Uart::new(config).expect("a valid configuration"));
            graph.connect(source.samples, Uart::input(uart))?;
            Ok(Uart::events(uart))
        },
    )
}

fn build(config: UartConfig, rate: u64, graph_config: GraphConfig) -> Harness<UartEvent> {
    try_build(config, rate, graph_config).expect("builds")
}

fn frame_text(value: u16, parity: Option<ParityCheck>, framing_ok: bool) -> String {
    let parity = match parity {
        None => "",
        Some(ParityCheck::Ok) => " parity-ok",
        Some(ParityCheck::Error) => " parity-error",
    };
    let framing = if framing_ok { "" } else { " framing-error" };
    format!("frame {value:#x}{parity}{framing}")
}

/// One line per item, readable in assertions.
/// Drops a progress line that repeats the one before it: whether two progress items fold in
/// the queue depends on when the reader ran, which is not what these tests check.
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

fn describe(items: &[Item<UartEvent>]) -> Vec<String> {
    items
        .iter()
        .map(|item| match item {
            Item::Event(event) => {
                let what = match event.payload {
                    UartEvent::Frame {
                        value,
                        parity,
                        framing_ok,
                    } => frame_text(value, parity, framing_ok),
                    UartEvent::Break { ticks } => format!("break {ticks}"),
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

/// Decodes `block` in one piece: the lines, progress left out.
fn decode(config: UartConfig, rate: u64, block: &LogicBlock) -> Vec<String> {
    let mut harness = build(config, rate, GraphConfig::default());
    harness.run_all(block);
    describe(&harness.without_progress())
}

/// Where a character is read and placed, by the rules of the decoder's documentation.
struct Expect {
    config: UartConfig,
    rate: u64,
}

impl Expect {
    /// The tick `two_hundredths` two-hundredths of a bit after `start`, rounded down.
    fn at(&self, start: u64, two_hundredths: u64) -> u64 {
        start + self.rate * two_hundredths / (200 * u64::from(self.config.baud))
    }

    /// The line of a character whose start edge is at `start`.
    fn frame(&self, start: u64, value: u16, parity: Option<ParityCheck>, framing: bool) -> String {
        let sample_point = u64::from(self.config.sample_point);
        let whole =
            1 + u64::from(self.config.data_bits) + u64::from(self.config.parity != Parity::None);
        let settled = match self.config.stop_bits {
            StopBits::One => self.at(start, 200 * whole + 2 * sample_point),
            StopBits::OneAndAHalf => self.at(start, 200 * (whole + 1) + sample_point),
            StopBits::Two => self.at(start, 200 * (whole + 1) + 2 * sample_point),
        };
        let stop = 100 * u64::from(self.config.stop_bits.half_bits());
        let end = self.at(start, 200 * whole + stop);
        format!(
            "{} [{start},{end})@{settled}",
            frame_text(value, parity, framing)
        )
    }
}

#[test]
fn a_character_is_spanned_and_settled_as_documented() {
    let mut waveform = UartWaveform::new(config(), CHANNELS, BIT, Tick::ZERO);
    waveform.idle(20);
    waveform.byte(0x41);
    waveform.idle(10);
    // The start edge at 20; the stop bit, bit 9, read at 20 + 95; ten bits end at 120.
    assert_eq!(
        decode(config(), RATE, &waveform.finish()),
        vec!["frame 0x41 [20,120)@115", "eof 130"]
    );
}

#[test]
fn decodes_every_width_and_parity() {
    for data_bits in 5..=9 {
        for parity in [
            Parity::None,
            Parity::Even,
            Parity::Odd,
            Parity::Mark,
            Parity::Space,
        ] {
            let config = UartConfig {
                data_bits,
                parity,
                ..config()
            };
            let mask = (1u16 << data_bits) - 1;
            let values = [0x1a5 & mask, 0x05a & mask, mask, 1, 0];
            let mut waveform = UartWaveform::new(config, CHANNELS, BIT, Tick::ZERO);
            waveform.idle(20);
            let mut starts = Vec::new();
            for value in values {
                starts.push(waveform.position().0);
                waveform.byte(value);
                waveform.idle(13);
            }
            let block = waveform.finish();
            let expect = Expect { config, rate: RATE };
            let check = (parity != Parity::None).then_some(ParityCheck::Ok);
            let mut wanted: Vec<String> = values
                .iter()
                .zip(&starts)
                .map(|(value, start)| expect.frame(*start, *value, check, true))
                .collect();
            wanted.push(format!("eof {}", block.end()));
            assert_eq!(
                decode(config, RATE, &block),
                wanted,
                "{data_bits} bits, {parity:?}"
            );
        }
    }
}

#[test]
fn each_stop_period_sets_the_span_and_the_last_sample_point() {
    let cases = [
        (StopBits::One, ["[20,120)@115", "[120,220)@215"]),
        // The half stop bit is read halfway through it: 20 + 102.5, rounded down.
        (StopBits::OneAndAHalf, ["[20,125)@122", "[125,230)@227"]),
        (StopBits::Two, ["[20,130)@125", "[130,240)@235"]),
    ];
    for (stop_bits, spans) in cases {
        let config = UartConfig {
            stop_bits,
            ..config()
        };
        let mut waveform = UartWaveform::new(config, CHANNELS, BIT, Tick::ZERO);
        waveform.idle(20);
        waveform.bytes([0x96, 0x69]);
        waveform.idle(20);
        let block = waveform.finish();
        assert_eq!(
            decode(config, RATE, &block),
            vec![
                format!("frame 0x96 {}", spans[0]),
                format!("frame 0x69 {}", spans[1]),
                format!("eof {}", block.end()),
            ],
            "{stop_bits:?}"
        );
    }
}

#[test]
fn both_stop_bits_of_two_are_read() {
    // Sent with one stop bit, and a pulse to space where the second stop bit belongs.
    let mut waveform = UartWaveform::new(config(), CHANNELS, BIT, Tick::ZERO);
    waveform.idle(20);
    waveform.byte(0x5a);
    waveform.glitch(BIT);
    waveform.idle(30);
    waveform.byte(0x5a);
    waveform.idle(30);
    let two = UartConfig {
        stop_bits: StopBits::Two,
        ..config()
    };
    assert_eq!(
        decode(two, RATE, &waveform.finish()),
        vec![
            "frame 0x5a framing-error [20,130)@125",
            "frame 0x5a [160,270)@265",
            "eof 290"
        ]
    );
}

#[test]
fn inverted_lines_and_msb_first_order() {
    for (inverted, bit_order) in [
        (true, BitOrder::LsbFirst),
        (false, BitOrder::MsbFirst),
        (true, BitOrder::MsbFirst),
    ] {
        let config = UartConfig {
            inverted,
            bit_order,
            parity: Parity::Odd,
            ..config()
        };
        let mut waveform = UartWaveform::new(config, CHANNELS, BIT, Tick::ZERO);
        waveform.idle(20);
        let mut starts = Vec::new();
        for value in [0x01, 0x80, 0x3c] {
            starts.push(waveform.position().0);
            waveform.byte(value);
            waveform.idle(7);
        }
        let block = waveform.finish();
        let expect = Expect { config, rate: RATE };
        let ok = Some(ParityCheck::Ok);
        assert_eq!(
            decode(config, RATE, &block),
            vec![
                expect.frame(starts[0], 0x01, ok, true),
                expect.frame(starts[1], 0x80, ok, true),
                expect.frame(starts[2], 0x3c, ok, true),
                format!("eof {}", block.end()),
            ],
            "inverted {inverted}, {bit_order:?}"
        );
    }
    // The bit order decides which end of the value the first bit is.
    let msb = UartConfig {
        bit_order: BitOrder::MsbFirst,
        ..config()
    };
    let mut waveform = UartWaveform::new(msb, CHANNELS, BIT, Tick::ZERO);
    waveform.idle(20);
    waveform.byte(0x01);
    waveform.idle(10);
    assert_eq!(
        decode(config(), RATE, &waveform.finish()),
        vec!["frame 0x80 [20,120)@115", "eof 130"]
    );
}

#[test]
fn parity_and_framing_errors_are_reported() {
    for parity in [Parity::Even, Parity::Odd, Parity::Mark, Parity::Space] {
        let config = UartConfig { parity, ..config() };
        let mut waveform = UartWaveform::new(config, CHANNELS, BIT, Tick::ZERO);
        waveform.idle(20);
        waveform.frame(0x35, Faults::PARITY);
        waveform.idle(10);
        waveform.frame(0x35, Faults::FRAMING);
        waveform.idle(30);
        waveform.byte(0x35);
        waveform.idle(10);
        assert_eq!(
            decode(config, RATE, &waveform.finish()),
            vec![
                "frame 0x35 parity-error [20,130)@125",
                "frame 0x35 parity-ok framing-error [140,250)@245",
                "frame 0x35 parity-ok [280,390)@385",
                "eof 400",
            ],
            "{parity:?}"
        );
    }
    // Without a parity bit only the stop bit can be wrong.
    let mut waveform = UartWaveform::new(config(), CHANNELS, BIT, Tick::ZERO);
    waveform.idle(20);
    waveform.frame(0x35, Faults::PARITY);
    waveform.idle(10);
    waveform.frame(0x35, Faults::FRAMING);
    waveform.idle(30);
    assert_eq!(
        decode(config(), RATE, &waveform.finish()),
        vec![
            "frame 0x35 [20,120)@115",
            "frame 0x35 framing-error [130,230)@225",
            "eof 260"
        ]
    );
}

#[test]
fn a_break_is_reported_with_how_long_the_line_was_at_space() {
    let mut waveform = UartWaveform::new(config(), CHANNELS, BIT, Tick::ZERO);
    waveform.idle(20);
    waveform.byte(0x55);
    waveform.idle(20);
    // At space through the stop bit's sample point, 95 ticks after the start edge: a break.
    waveform.break_(96);
    waveform.idle(30);
    // Back at mark right at the stop bit's sample point: a NUL character.
    waveform.break_(95);
    waveform.idle(30);
    waveform.break_(400);
    waveform.idle(20);
    waveform.byte(0xaa);
    waveform.idle(10);
    assert_eq!(
        decode(config(), RATE, &waveform.finish()),
        vec![
            "frame 0x55 [20,120)@115",
            "break 96 [140,236)@236",
            "frame 0x0 [266,366)@361",
            "break 400 [391,791)@791",
            "frame 0xaa [811,911)@906",
            "eof 921",
        ]
    );
    // With a parity bit, a break holds space through the parity bit too.
    let even = UartConfig {
        parity: Parity::Even,
        ..config()
    };
    let mut waveform = UartWaveform::new(even, CHANNELS, BIT, Tick::ZERO);
    waveform.idle(20);
    waveform.break_(500);
    waveform.idle(20);
    assert_eq!(
        decode(even, RATE, &waveform.finish()),
        vec!["break 500 [20,520)@520", "eof 540"]
    );
}

#[test]
fn back_to_back_characters_need_no_idle_time() {
    let mut waveform = UartWaveform::new(config(), CHANNELS, BIT, Tick::ZERO);
    waveform.idle(20);
    waveform.bytes([0x00, 0xff, 0x55, 0x81, 0x7e]);
    waveform.idle(10);
    assert_eq!(
        decode(config(), RATE, &waveform.finish()),
        vec![
            "frame 0x0 [20,120)@115",
            "frame 0xff [120,220)@215",
            "frame 0x55 [220,320)@315",
            "frame 0x81 [320,420)@415",
            "frame 0x7e [420,520)@515",
            "eof 530",
        ]
    );
}

#[test]
fn a_glitch_that_is_over_by_the_start_bits_sample_point_is_ignored() {
    let mut waveform = UartWaveform::new(config(), CHANNELS, BIT, Tick::ZERO);
    waveform.idle(20);
    // Back at mark at 25, where the start bit is read.
    waveform.glitch(5);
    waveform.idle(20);
    waveform.byte(0x42);
    waveform.idle(20);
    // Still at space at 170: a start bit, followed by an idle line that reads as 0xff.
    waveform.glitch(6);
    waveform.idle(200);
    waveform.byte(0x24);
    waveform.idle(10);
    assert_eq!(
        decode(config(), RATE, &waveform.finish()),
        vec![
            "frame 0x42 [45,145)@140",
            "frame 0xff [165,265)@260",
            "frame 0x24 [371,471)@466",
            "eof 481",
        ]
    );
}

#[test]
fn odd_ratios_of_rate_to_baud_are_spanned_exactly() {
    // 1 MHz at 115 200 baud: 8.68 ticks per bit; the stop bit is read 82 ticks after the start
    // edge and ten bits end after 86.
    let fast = UartConfig {
        baud: 115_200,
        ..config()
    };
    let mut waveform = UartWaveform::at_rate(fast, CHANNELS, 1_000_000, Tick::ZERO);
    waveform.idle(37);
    let mut starts = Vec::new();
    let values = [0x55, 0xa3, 0x00, 0xff, 0x7e, 0x81];
    for (n, value) in values.into_iter().enumerate() {
        if n == 4 {
            waveform.idle(50);
        }
        starts.push(waveform.position().0);
        waveform.byte(value);
    }
    waveform.idle(40);
    let block = waveform.finish();
    // Back to back, each character starts 87 ticks after the one before: 86.8 rounded up.
    assert_eq!(starts, vec![37, 124, 211, 298, 435, 522]);
    let mut wanted: Vec<String> = values
        .iter()
        .zip(&starts)
        .map(|(value, start)| format!("frame {value:#x} [{start},{})@{}", start + 86, start + 82))
        .collect();
    wanted.push(format!("eof {}", block.end()));
    assert_eq!(decode(fast, 1_000_000, &block), wanted);

    // 1 MHz at 9600 baud with even parity and two stop bits: 104.17 ticks per bit.
    let slow = UartConfig {
        baud: 9600,
        parity: Parity::Even,
        stop_bits: StopBits::Two,
        ..config()
    };
    let mut waveform = UartWaveform::at_rate(slow, CHANNELS, 1_000_000, Tick::ZERO);
    waveform.idle(500);
    waveform.bytes([0x31, 0x32]);
    waveform.idle(500);
    let block = waveform.finish();
    // Twelve bits make 1250 ticks; the second stop bit is read at ⌊1197.9⌋.
    assert_eq!(
        decode(slow, 1_000_000, &block),
        vec![
            "frame 0x31 parity-ok [500,1750)@1697",
            "frame 0x32 parity-ok [1750,3000)@2947",
            "eof 3500",
        ]
    );
}

#[test]
fn a_character_under_way_at_the_first_sample_is_skipped() {
    let mut waveform = UartWaveform::new(config(), CHANNELS, BIT, Tick::ZERO);
    waveform.idle(20);
    // Data bits 0 to 3 at space, 4 to 7 at mark.
    waveform.byte(0xf0);
    waveform.idle(30);
    waveform.byte(0x3c);
    waveform.idle(10);
    let block = waveform.finish();
    // From inside the start bit, at space; and from inside data bit 4, at mark.
    for from in [23, 75] {
        let tail = block
            .slice(TickRange::new(Tick(from), block.end()))
            .expect("a slice");
        assert_eq!(
            decode(config(), RATE, &tail),
            vec!["frame 0x3c [150,250)@245", "eof 260"],
            "from {from}"
        );
    }
}

#[test]
fn a_character_cut_off_by_the_end_of_the_samples_is_not_reported() {
    let mut waveform = UartWaveform::new(config(), CHANNELS, BIT, Tick::ZERO);
    waveform.idle(20);
    waveform.byte(0x41);
    waveform.idle(20);
    waveform.byte(0x42);
    let block = waveform.finish();
    let head = block
        .slice(TickRange::new(Tick::ZERO, Tick(200)))
        .expect("a slice");
    let mut harness = build(config(), RATE, GraphConfig::default());
    harness.run_all(&head);
    // Progress goes no further than the start of the character that was cut off. (The
    // stream starts with the progress its source announces at its first tick.)
    assert_eq!(
        folded(describe(&harness.items)),
        vec![
            "progress 0",
            "frame 0x41 [20,120)@115",
            "progress 140",
            "eof 200"
        ]
    );
}

#[test]
fn a_gap_drops_the_character_it_cuts_and_decoding_resumes_after_it() {
    let mut waveform = UartWaveform::new(config(), CHANNELS, BIT, Tick::ZERO);
    waveform.idle(20);
    waveform.byte(0x41);
    waveform.idle(20);
    // Data bits 0 to 3 at space, 4 to 7 at mark: nothing after the gap looks like a start.
    waveform.byte(0xf0);
    waveform.idle(30);
    waveform.byte(0x43);
    waveform.idle(10);
    let block = waveform.finish();
    let gap = TickRange::new(Tick(162), Tick(178));
    let mut harness = build(config(), RATE, GraphConfig::default());
    harness.feed_with_gaps(&block, &[gap], &[u64::MAX]);
    harness.end();
    assert_eq!(
        folded(describe(&harness.items)),
        vec![
            "progress 0",
            "frame 0x41 [20,120)@115",
            "progress 140",
            "gap 162..178",
            "frame 0x43 [270,370)@365",
            "progress 380",
            "eof 380",
        ]
    );
    // A gap while the line idles is announced by progress up to its start.
    let gap = TickRange::new(Tick(125), Tick(135));
    let mut harness = build(config(), RATE, GraphConfig::default());
    harness.feed_with_gaps(&block, &[gap], &[u64::MAX]);
    harness.end();
    assert_eq!(
        describe(&harness.items)[..4],
        [
            "progress 0",
            "frame 0x41 [20,120)@115",
            "progress 125",
            "gap 125..135"
        ]
    );
}

/// Characters of every kind, glitches and breaks. Returns where things begin: `[0]` the
/// character with a parity error, `[1]` the idle line after the one with a framing error, `[2]`
/// a break, `[3]` another break, `[4..]` the characters of a final run.
fn rich_waveform(waveform: &mut UartWaveform, bit: u64) -> Vec<Tick> {
    let mut marks = Vec::new();
    waveform.idle(17);
    waveform.bytes([0x00, 0xff, 0x55]);
    waveform.idle(23);
    marks.push(waveform.position());
    waveform.frame(0x12, Faults::PARITY);
    waveform.idle(9);
    waveform.frame(0x34, Faults::FRAMING);
    marks.push(waveform.position());
    waveform.idle(3 * bit);
    waveform.glitch(bit / 3);
    waveform.idle(15);
    marks.push(waveform.position());
    waveform.break_(30 * bit);
    waveform.idle(4 * bit);
    waveform.bytes([0x7e, 0x81]);
    waveform.idle(5);
    waveform.byte(0x00);
    waveform.idle(60);
    marks.push(waveform.position());
    waveform.break_(20 * bit);
    waveform.idle(3 * bit);
    for i in 0..24u16 {
        marks.push(waveform.position());
        waveform.byte(i * 13);
        waveform.idle(u64::from(i % 4) * bit / 2);
    }
    waveform.idle(2 * bit);
    marks
}

#[test]
fn every_cut_and_pause_yields_the_same_events() {
    let plain = UartConfig {
        parity: Parity::Even,
        ..config()
    };
    let odd = UartConfig {
        baud: 76_800,
        data_bits: 7,
        parity: Parity::Odd,
        stop_bits: StopBits::OneAndAHalf,
        bit_order: BitOrder::MsbFirst,
        inverted: true,
        sample_point: 35,
        ..config()
    };
    for config in [plain, odd] {
        let mut waveform = UartWaveform::at_rate(config, CHANNELS, RATE, Tick::ZERO);
        let bit = RATE / u64::from(config.baud);
        let marks = rich_waveform(&mut waveform, bit);
        let block = waveform.finish();
        // In the idle line after the framing error, inside the first break, inside a character.
        let gaps = [
            TickRange::new(marks[1].offset(bit), marks[1].offset(2 * bit)),
            TickRange::new(marks[2].offset(5 * bit), marks[2].offset(9 * bit)),
            TickRange::new(marks[7].offset(3 * bit), marks[7].offset(4 * bit)),
        ];
        let reference = {
            let mut harness = build(config, RATE, GraphConfig::default());
            harness.feed_with_gaps(&block, &gaps, &[u64::MAX]);
            harness.end();
            describe(&harness.without_progress())
        };
        assert!(reference.iter().any(|line| line.starts_with("break ")));
        assert!(reference.iter().any(|line| line.contains("parity-error")));
        assert!(reference.iter().any(|line| line.contains("framing-error")));
        assert_eq!(
            reference
                .iter()
                .filter(|line| line.starts_with("gap "))
                .count(),
            3
        );
        assert_block_invariant(
            |graph_config| build(config, RATE, graph_config),
            &block,
            &gaps,
        );
    }
}

#[test]
fn progress_never_passes_a_character_still_being_read() {
    let config = UartConfig {
        parity: Parity::Odd,
        ..config()
    };
    let mut waveform = UartWaveform::new(config, CHANNELS, BIT, Tick::ZERO);
    let marks = rich_waveform(&mut waveform, BIT);
    let block = waveform.finish();
    let gaps = [TickRange::new(
        marks[5].offset(BIT / 2),
        marks[5].offset(3 * BIT),
    )];
    for sizes in [&[1u64][..], &[3], &[7, 64], &[97]] {
        let mut harness = build(config, RATE, GraphConfig::default());
        harness.feed_cut(&block, &gaps, sizes, &[1]);
        harness.end();
        let mut seen = Tick::ZERO;
        let mut progress = 0;
        for item in &harness.items {
            match item {
                Item::Progress(tick) => {
                    seen = seen.max(*tick);
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
fn a_source_with_fewer_than_two_ticks_per_bit_is_refused() {
    // 1.5 ticks per bit at 100 000 baud: bits cannot be told apart.
    let error = try_build(config(), 150_000, GraphConfig::default()).expect_err("refused");
    assert_eq!(
        error,
        GraphError::Requirement {
            from: "logic#0.samples".into(),
            to: "uart#0.logic".into(),
            key: "rateHz",
            requirement: ">= 200000".into(),
            found: "150000".into(),
        }
    );
    // Two ticks per bit are enough.
    let mut waveform = UartWaveform::new(config(), CHANNELS, 2, Tick::ZERO);
    waveform.idle(10);
    waveform.bytes([0x12, 0x34]);
    waveform.idle(10);
    assert_eq!(
        decode(config(), 200_000, &waveform.finish()),
        vec!["frame 0x12 [10,30)@29", "frame 0x34 [30,50)@49", "eof 60"]
    );
}

#[test]
fn the_sample_point_sets_where_bits_are_read() {
    let mut waveform = UartWaveform::new(config(), CHANNELS, BIT, Tick::ZERO);
    waveform.idle(20);
    waveform.byte(0x41);
    waveform.idle(10);
    let block = waveform.finish();
    // The stop bit is bit 9: read at 20 + ⌊10 × (900 + p) / 100⌋. At 1% the start bit is read
    // on the start edge itself.
    for (sample_point, settled) in [(1, 110), (25, 112), (50, 115), (75, 117), (99, 119)] {
        let config = UartConfig {
            sample_point,
            ..config()
        };
        assert_eq!(
            decode(config, RATE, &block),
            vec![format!("frame 0x41 [20,120)@{settled}"), "eof 130".into()],
            "sample point {sample_point}"
        );
    }
}

fn parity() -> impl Strategy<Value = Parity> {
    prop_oneof![
        Just(Parity::None),
        Just(Parity::Even),
        Just(Parity::Odd),
        Just(Parity::Mark),
        Just(Parity::Space),
    ]
}

fn stop_bits() -> impl Strategy<Value = StopBits> {
    prop_oneof![
        Just(StopBits::One),
        Just(StopBits::OneAndAHalf),
        Just(StopBits::Two),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 48, ..ProptestConfig::default() })]

    /// Whatever the format and however the samples are cut, every character sent is read back.
    #[test]
    fn random_characters_are_read_back(
        data_bits in 5u8..=9,
        parity in parity(),
        stop_bits in stop_bits(),
        msb_first in any::<bool>(),
        inverted in any::<bool>(),
        sample_point in 30u8..=70,
        bit in 3u64..=12,
        characters in prop::collection::vec((any::<u16>(), 0u64..30), 1..40),
        sizes in prop::collection::vec(1u64..300, 1..5),
    ) {
        let config = UartConfig {
            line: 1,
            baud: BAUD,
            data_bits,
            parity,
            stop_bits,
            bit_order: if msb_first { BitOrder::MsbFirst } else { BitOrder::LsbFirst },
            inverted,
            sample_point,
        };
        let mut waveform = UartWaveform::new(config, CHANNELS, bit, Tick::ZERO);
        waveform.idle(bit + 5);
        for (value, idle) in &characters {
            waveform.byte(*value);
            waveform.idle(*idle);
        }
        waveform.idle(2 * bit);
        let block = waveform.finish();
        let mut harness = build(config, u64::from(BAUD) * bit, GraphConfig::default());
        harness.feed(&block, &sizes);
        harness.end();
        let mask = (1u16 << data_bits) - 1;
        let sent: Vec<u16> = characters.iter().map(|(value, _)| value & mask).collect();
        let mut read = Vec::new();
        for event in harness.events() {
            match event.payload {
                UartEvent::Frame { value, parity: check, framing_ok } => {
                    prop_assert!(framing_ok);
                    prop_assert_eq!(check.is_some(), parity != Parity::None);
                    prop_assert_ne!(check, Some(ParityCheck::Error));
                    read.push(value);
                }
                UartEvent::Break { .. } => prop_assert!(false, "a break"),
            }
        }
        prop_assert_eq!(read, sent);
    }
}
