// SPDX-License-Identifier: Apache-2.0

//! The I²S decoder on synthesized buses: every format, word width and option, half-periods
//! cut short, under way at the start, lost in gaps and cut off by the end of the samples.

use proptest::prelude::*;
use uiharu_core::testing::{Harness, assert_block_invariant};
use uiharu_core::{GraphConfig, Item, LogicBlock, Tick, TickRange, TimeBase};
use uiharu_protocols::i2s::synth::I2sWaveform;
use uiharu_protocols::i2s::{Channel, Format, I2s, I2sConfig, I2sEvent, SampleEdge};

const CHANNELS: u8 = 3;
/// Samples per half clock period.
const HALF: u64 = 2;
/// Idle samples before the first clock and after the last.
const LEAD: u64 = 10;

fn harness(config: &I2sConfig, graph_config: GraphConfig) -> Harness<I2sEvent> {
    Harness::build(
        CHANNELS,
        TimeBase::new(3_072_000),
        graph_config,
        |graph, source| {
            let i2s = graph.add(I2s::new(config.clone()).expect("a valid configuration"));
            graph.connect(source.samples, I2s::input(i2s))?;
            Ok(I2s::events(i2s))
        },
    )
    .expect("builds")
}

fn bus(format: Format, bits: Option<u8>) -> I2sConfig {
    I2sConfig {
        format,
        bits,
        ..I2sConfig::on(0, 1, 2)
    }
}

/// A sample as a line: frame, channel, value and width, the ticks it covers and where it was
/// settled.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Line {
    frame: u64,
    channel: Channel,
    value: u32,
    bits: u8,
    complete: bool,
    span: TickRange,
    settled: Tick,
}

impl Line {
    fn text(&self) -> String {
        let channel = match self.channel {
            Channel::Left => "L",
            Channel::Right => "R",
        };
        let incomplete = if self.complete { "" } else { " incomplete" };
        format!(
            "f{} {channel} {:#x}/{}{incomplete} [{},{})@{}",
            self.frame, self.value, self.bits, self.span.start, self.span.end, self.settled
        )
    }
}

/// One line per item, progress left out.
fn lines(items: &[Item<I2sEvent>]) -> Vec<String> {
    items
        .iter()
        .filter_map(|item| match item {
            Item::Event(event) => {
                let I2sEvent::Sample {
                    frame,
                    channel,
                    value,
                    bits,
                    complete,
                } = event.payload;
                let line = Line {
                    frame,
                    channel,
                    value,
                    bits,
                    complete,
                    span: event.span,
                    settled: event.settled,
                };
                Some(line.text())
            }
            Item::Gap(range) => Some(format!("gap {range}")),
            Item::Progress(_) => None,
            Item::End(tick) => Some(format!("eof {tick}")),
        })
        .collect()
}

fn decode(config: &I2sConfig, block: &LogicBlock, gaps: &[TickRange]) -> Vec<String> {
    let mut harness = harness(config, GraphConfig::default());
    harness.feed_with_gaps(block, gaps, &[u64::MAX]);
    harness.end();
    lines(&harness.items)
}

/// One half-period as the transmitter sends it: a word of `bits` bits in `slots` clocks.
#[derive(Clone, Copy, Debug)]
struct Half {
    value: u32,
    bits: u8,
    slots: u32,
}

const fn half(value: u32, bits: u8, slots: u32) -> Half {
    Half { value, bits, slots }
}

/// The half-periods alternate from the left channel on.
fn channel_of(n: usize) -> Channel {
    if n.is_multiple_of(2) {
        Channel::Left
    } else {
        Channel::Right
    }
}

/// The waveform of `halves` at `half_period` samples per half clock, between `LEAD` idle
/// samples, closed by one clock of the next half-period.
fn waveform(config: &I2sConfig, halves: &[Half], half_period: u64) -> LogicBlock {
    let mut waveform = I2sWaveform::new(config.clone(), CHANNELS, half_period, Tick::ZERO);
    waveform.idle(LEAD);
    for (n, half) in halves.iter().enumerate() {
        waveform.half(channel_of(n), half.value, half.bits, half.slots);
    }
    waveform.close();
    waveform.idle(LEAD);
    waveform.finish()
}

/// The tick of the sampling edge of clock `n`, counted from the first clock.
fn edge(n: u64, half_period: u64) -> Tick {
    Tick(LEAD + 2 * half_period * n + half_period)
}

/// What a receiver configured as `config` reads from `halves`: a model of the formats from
/// where the transmitter puts the bits, not from how the decoder finds them.
fn model(config: &I2sConfig, halves: &[Half], half_period: u64) -> Vec<Line> {
    let width = u64::from(config.bits.unwrap_or(32));
    let edge = |n: u64| edge(n, half_period);
    let mut lines = Vec::new();
    // The clock that begins the half-period, where the receiver finds WS changed.
    let mut begins = 0u64;
    for (n, half) in halves.iter().enumerate() {
        let slots = u64::from(half.slots);
        let bits = u64::from(half.bits);
        let ends = begins + slots;
        // The bit on clock `j` of the half-period, left- or right-justified.
        let left = |j: u64| j < bits && half.value >> (bits - 1 - j) & 1 != 0;
        let right = |j: u64| slots - 1 - j < bits && half.value >> (slots - 1 - j) & 1 != 0;
        let read = slots.min(width);
        let (first, word): (u64, Vec<bool>) = match config.format {
            Format::LeftJustified => (begins, (0..read).map(left).collect()),
            // One clock late: the last bit of a full word is read on the next half's first.
            Format::I2s => (begins + 1, (0..read).map(left).collect()),
            Format::RightJustified => (ends - read, (slots - read..slots).map(right).collect()),
        };
        let last = first + read - 1;
        let settled = match config.format {
            Format::I2s => edge(last),
            Format::LeftJustified if read == width => edge(last),
            Format::LeftJustified | Format::RightJustified => edge(ends),
        };
        lines.push(Line {
            frame: n as u64 / 2 + 1,
            channel: channel_of(n),
            value: word
                .iter()
                .fold(0, |value, bit| (value << 1) | u32::from(*bit)),
            bits: read as u8,
            complete: read == width || config.bits.is_none(),
            span: TickRange::new(edge(first), edge(last).next()),
            settled,
        });
        begins = ends;
    }
    // The closing clock begins one more half-period, which the end of the samples cuts off:
    // only a one-bit word, left-justified, is whole with its first clock.
    if config.format == Format::LeftJustified && width == 1 {
        let n = halves.len();
        lines.push(Line {
            frame: n as u64 / 2 + 1,
            channel: channel_of(n),
            value: 0,
            bits: 1,
            complete: true,
            span: TickRange::one(edge(begins)),
            settled: edge(begins),
        });
    }
    lines
}

fn texts(lines: &[Line]) -> Vec<String> {
    lines.iter().map(Line::text).collect()
}

/// The model's lines and the end of the samples.
fn expected(config: &I2sConfig, halves: &[Half], block: &LogicBlock) -> Vec<String> {
    let mut lines = texts(&model(config, halves, HALF));
    lines.push(format!("eof {}", block.end()));
    lines
}

/// A word of `bits` bits with its first and last bit set and a pattern between.
fn word(n: u32, bits: u8) -> u32 {
    let pattern = 0x9e37_79b9u32.rotate_left(n * 7) ^ n;
    let ends = (1u32 << (bits - 1)) | 1;
    let mask = if bits >= 32 {
        u32::MAX
    } else {
        (1 << bits) - 1
    };
    (pattern | ends) & mask
}

fn frames(count: u32, bits: u8, slots: u32) -> Vec<Half> {
    (0..2 * count)
        .map(|n| half(word(n, bits), bits, slots))
        .collect()
}

#[test]
fn decodes_the_first_frame_where_each_format_puts_it() {
    let halves = [half(0xa001, 16, 32), half(0x8ff1, 16, 32)];
    // Clock n is read at 12 + 4n; the close is clock 64 and the samples end at 280.
    let cases = [
        (
            Format::LeftJustified,
            ["f1 L 0xa001/16 [12,73)@72", "f1 R 0x8ff1/16 [140,201)@200"],
        ),
        (
            Format::I2s,
            ["f1 L 0xa001/16 [16,77)@76", "f1 R 0x8ff1/16 [144,205)@204"],
        ),
        (
            Format::RightJustified,
            [
                "f1 L 0xa001/16 [76,137)@140",
                "f1 R 0x8ff1/16 [204,265)@268",
            ],
        ),
    ];
    for (format, [left, right]) in cases {
        let config = bus(format, Some(16));
        let block = waveform(&config, &halves, HALF);
        assert_eq!(
            decode(&config, &block, &[]),
            vec![left, right, "eof 280"],
            "{format:?}"
        );
        assert_eq!(
            decode(&config, &block, &[]),
            expected(&config, &halves, &block)
        );
    }
}

#[test]
fn decodes_every_format_at_16_24_and_32_bits() {
    for format in [Format::I2s, Format::LeftJustified, Format::RightJustified] {
        for bits in [16, 24, 32] {
            // In 32-clock slots, and in slots as wide as the word.
            for slots in [32, u32::from(bits)] {
                let config = bus(format, Some(bits));
                let halves = frames(3, bits, slots);
                let block = waveform(&config, &halves, HALF);
                let decoded = decode(&config, &block, &[]);
                assert_eq!(
                    decoded.len(),
                    7,
                    "{format:?} {bits} in {slots}: {decoded:?}"
                );
                assert_eq!(
                    decoded,
                    expected(&config, &halves, &block),
                    "{format:?} {bits} in {slots}"
                );
            }
        }
    }
}

#[test]
fn without_a_width_the_half_period_decides_it() {
    for format in [Format::I2s, Format::LeftJustified] {
        for slots in [8, 16, 24, 32] {
            let config = bus(format, None);
            let bits = slots as u8;
            let halves = frames(2, bits, slots);
            let block = waveform(&config, &halves, HALF);
            let decoded = decode(&config, &block, &[]);
            assert!(
                decoded[0].contains(&format!("/{bits} [")),
                "{format:?} in {slots}: {decoded:?}"
            );
            assert_eq!(
                decoded,
                expected(&config, &halves, &block),
                "{format:?} in {slots}"
            );
        }
    }
    // Left-justified, a 16-bit word of WS-determined width is settled where the receiver
    // finds WS changed, the clock after its last bit.
    let config = bus(Format::LeftJustified, None);
    let block = waveform(&config, &[half(0xa001, 16, 16)], HALF);
    assert_eq!(
        decode(&config, &block, &[]),
        vec!["f1 L 0xa001/16 [12,73)@76", "eof 88"]
    );
}

#[test]
fn one_bit_words_are_whole_with_their_bit() {
    let halves = [half(1, 1, 3), half(1, 1, 2)];
    // Left-justified, even the closing clock carries a whole word; in the I²S format its word
    // would begin a clock later.
    let config = bus(Format::LeftJustified, Some(1));
    let block = waveform(&config, &halves, HALF);
    let lines = vec![
        "f1 L 0x1/1 [12,13)@12",
        "f1 R 0x1/1 [24,25)@24",
        "f2 L 0x0/1 [32,33)@32",
        "eof 44",
    ];
    assert_eq!(decode(&config, &block, &[]), lines);
    assert_eq!(expected(&config, &halves, &block), lines);
    let config = bus(Format::I2s, Some(1));
    let block = waveform(&config, &halves, HALF);
    let lines = vec!["f1 L 0x1/1 [16,17)@16", "f1 R 0x1/1 [28,29)@28", "eof 44"];
    assert_eq!(decode(&config, &block, &[]), lines);
    assert_eq!(expected(&config, &halves, &block), lines);
}

#[test]
fn a_long_half_period_keeps_its_first_32_bits() {
    for (format, first) in [
        (Format::LeftJustified, 0xaaaa_aaaa),
        (Format::I2s, 0x5555_5555),
    ] {
        let config = bus(format, None);
        let mut waveform = I2sWaveform::new(config.clone(), CHANNELS, HALF, Tick::ZERO);
        waveform.idle(LEAD);
        // Forty clocks of alternating bits, as the format reads them from the first clock or
        // from the second.
        for n in 0..40 {
            waveform.clock(Channel::Left, n % 2 == 0);
        }
        waveform.half(Channel::Right, 0x1234, 16, 16);
        waveform.close();
        let block = waveform.finish();
        let decoded = decode(&config, &block, &[]);
        assert_eq!(decoded.len(), 3, "{format:?}: {decoded:?}");
        // The word is whole with its 32nd bit; the eight after it are dropped.
        let (from, to) = match format {
            Format::I2s => (1, 32),
            _ => (0, 31),
        };
        let left = Line {
            frame: 1,
            channel: Channel::Left,
            value: first,
            bits: 32,
            complete: true,
            span: TickRange::new(edge(from, HALF), edge(to, HALF).next()),
            settled: edge(to, HALF),
        };
        assert_eq!(decoded[0], left.text(), "{format:?}");
        assert!(
            decoded[1].starts_with("f1 R 0x1234/16 ["),
            "{format:?}: {decoded:?}"
        );
    }
}

#[test]
fn a_short_half_period_yields_an_incomplete_sample() {
    // Twenty-four-bit words, the second frame in sixteen-clock slots.
    let halves = [
        half(0x00a5_5a5a, 24, 32),
        half(0x0081_8181, 24, 32),
        half(0x00c3_3c3c, 24, 16),
        half(0x00f0_0f0f, 24, 16),
        half(0x0012_3456, 24, 32),
        half(0x0065_4321, 24, 32),
    ];
    for format in [Format::I2s, Format::LeftJustified, Format::RightJustified] {
        let config = bus(format, Some(24));
        let block = waveform(&config, &halves, HALF);
        let decoded = decode(&config, &block, &[]);
        assert_eq!(decoded, expected(&config, &halves, &block), "{format:?}");
        // A sixteen-clock slot holds the first sixteen bits of a word; right-justified, its
        // last sixteen.
        let (left, right) = match format {
            Format::RightJustified => ("0x3c3c", "0xf0f"),
            _ => ("0xc33c", "0xf00f"),
        };
        assert!(
            decoded[2].starts_with(&format!("f2 L {left}/16 incomplete [")),
            "{format:?}: {decoded:?}"
        );
        assert!(
            decoded[3].starts_with(&format!("f2 R {right}/16 incomplete [")),
            "{format:?}: {decoded:?}"
        );
        assert!(decoded[4].starts_with("f3 L 0x123456/24 ["), "{decoded:?}");
    }
}

#[test]
fn the_sampling_edge_can_be_the_falling_one() {
    for format in [Format::I2s, Format::LeftJustified, Format::RightJustified] {
        let config = I2sConfig {
            sample_edge: SampleEdge::Falling,
            ..bus(format, Some(24))
        };
        let halves = frames(3, 24, 32);
        let block = waveform(&config, &halves, HALF);
        // The clock idles low and the bits are read where it falls.
        assert_eq!(block.level(0, Tick(0)), Some(false));
        assert_eq!(block.level(0, edge(0, HALF)), Some(false));
        assert_eq!(block.level(0, Tick(edge(0, HALF).0 - 1)), Some(true));
        assert_eq!(
            decode(&config, &block, &[]),
            expected(&config, &halves, &block),
            "{format:?}"
        );
        // Read on the rising edges, where this transmitter sets the lines, every word would
        // be placed half a clock early.
        let wrong = I2sConfig {
            sample_edge: SampleEdge::Rising,
            ..config.clone()
        };
        let early = decode(&wrong, &block, &[]);
        assert_ne!(early, expected(&config, &halves, &block), "{format:?}");
        assert_eq!(
            early[0].split('[').next(),
            expected(&config, &halves, &block)[0].split('[').next(),
            "{format:?}"
        );
    }
}

#[test]
fn ws_inverted_makes_ws_high_the_left_channel() {
    let config = I2sConfig {
        ws_inverted: true,
        ..bus(Format::I2s, Some(16))
    };
    let halves = frames(3, 16, 16);
    let block = waveform(&config, &halves, HALF);
    // The first half-period, the left one, has WS high.
    assert_eq!(block.level(1, edge(0, HALF)), Some(true));
    assert_eq!(
        decode(&config, &block, &[]),
        expected(&config, &halves, &block)
    );
    // Read the other way round, the first half-period is a right one, which begins the
    // first frame, and every frame after it begins with the next.
    let plain = I2sConfig {
        ws_inverted: false,
        ..config
    };
    let heads: Vec<String> = decode(&plain, &block, &[])
        .iter()
        .map(|line| line.split(' ').take(2).collect::<Vec<_>>().join(" "))
        .collect();
    assert_eq!(
        heads,
        vec!["f1 R", "f2 L", "f2 R", "f3 L", "f3 R", "f4 L", "eof 408"]
    );
}

#[test]
fn the_half_period_under_way_at_the_start_is_not_reported() {
    let config = bus(Format::LeftJustified, Some(16));
    let halves = frames(3, 16, 16);
    let block = waveform(&config, &halves, HALF);
    let whole = model(&config, &halves, HALF);
    let from = |tick: u64| {
        block
            .slice(TickRange::new(Tick(tick), block.end()))
            .expect("a slice")
    };
    // From the middle of the first left word: the right one after it is the first reported,
    // and still frame 1.
    let tail = from(edge(5, HALF).0);
    let mut lines = texts(&whole[1..]);
    lines.push(format!("eof {}", block.end()));
    assert_eq!(decode(&config, &tail, &[]), lines);
    // From the middle of the first right word: the frames are counted from the next left
    // one.
    let tail = from(edge(21, HALF).0);
    let renumbered: Vec<Line> = whole[2..]
        .iter()
        .map(|line| Line {
            frame: line.frame - 1,
            ..line.clone()
        })
        .collect();
    let mut lines = texts(&renumbered);
    lines.push(format!("eof {}", block.end()));
    assert_eq!(decode(&config, &tail, &[]), lines);
}

#[test]
fn a_gap_drops_the_words_it_cuts_and_frames_go_on() {
    let config = bus(Format::I2s, Some(16));
    let halves = frames(6, 16, 16);
    let block = waveform(&config, &halves, HALF);
    let whole = model(&config, &halves, HALF);
    // Half-period `n` begins on clock 16 n. The first gap runs from the middle of the second
    // left word to the middle of the right one after it; the second from the middle of the
    // fourth left word to the middle of the sixth, so that two whole half-periods are lost.
    let middle = |n: u64| edge(16 * n + 5, HALF);
    let gaps = [
        TickRange::new(middle(2), middle(3)),
        TickRange::new(middle(6), middle(10)),
    ];
    let renumbered = Line {
        frame: 5,
        ..whole[11].clone()
    };
    let mut lines = texts(&whole[..2]);
    lines.push(format!("gap {}", gaps[0]));
    lines.extend(texts(&whole[4..6]));
    lines.push(format!("gap {}", gaps[1]));
    // After the second gap the sixth right word begins a frame, the fifth counted.
    lines.push(renumbered.text());
    lines.push(format!("eof {}", block.end()));
    assert_eq!(decode(&config, &block, &gaps), lines);
}

#[test]
fn the_end_of_the_samples_drops_the_word_under_way() {
    for format in [Format::I2s, Format::LeftJustified, Format::RightJustified] {
        let config = bus(format, Some(32));
        let halves = frames(2, 32, 32);
        let block = waveform(&config, &halves, HALF);
        let whole = model(&config, &halves, HALF);
        // Cut just before the third half-period's first clock: the I²S word still misses its
        // last bit, the right-justified one the WS change that decides it.
        let cut = Tick(edge(64, HALF).0 - 1);
        let head = block
            .slice(TickRange::new(Tick::ZERO, cut))
            .expect("a slice");
        let mut lines = texts(&whole[..1]);
        if format == Format::LeftJustified {
            lines.extend(texts(&whole[1..2]));
        }
        lines.push(format!("eof {cut}"));
        assert_eq!(decode(&config, &head, &[]), lines, "{format:?}");
    }
}

#[test]
fn a_word_shows_up_as_soon_as_it_is_decided() {
    let halves = frames(1, 16, 32);
    // The samples arrive up to the middle of the left half-period, after its sixteenth clock,
    // then up to just after the sampling edge that finds WS changed, then the rest.
    let cuts = [edge(24, HALF).next(), edge(32, HALF).next()];
    for (format, seen) in [
        (Format::LeftJustified, [1, 1]),
        (Format::RightJustified, [0, 1]),
    ] {
        let config = bus(format, Some(16));
        let block = waveform(&config, &halves, HALF);
        let whole = expected(&config, &halves, &block);
        let mut harness = harness(&config, GraphConfig::default());
        let mut from = Tick::ZERO;
        for (cut, seen) in cuts.into_iter().zip(seen) {
            let piece = block.slice(TickRange::new(from, cut)).expect("a slice");
            harness.feed(&piece, &[u64::MAX]);
            harness.collect();
            assert_eq!(
                lines(&harness.items),
                whole[..seen].to_vec(),
                "{format:?} up to {cut}"
            );
            from = cut;
        }
        let rest = block
            .slice(TickRange::new(from, block.end()))
            .expect("a slice");
        harness.feed(&rest, &[u64::MAX]);
        harness.end();
        assert_eq!(lines(&harness.items), whole, "{format:?}");
    }
}

#[test]
fn progress_is_held_back_to_the_word_under_way() {
    let halves = frames(1, 16, 32);
    // The samples arrive up to just after clocks 5 and 24 of the left half-period and clock
    // 40, in the right one, which begins on clock 32.
    let cuts = [5, 24, 40].map(|n| edge(n, HALF).next());
    for (format, held) in [
        // The left word is whole on clock 15; the right one starts on clock 32.
        (
            Format::LeftJustified,
            [edge(0, HALF), cuts[1], edge(32, HALF)],
        ),
        // Each word a clock later.
        (Format::I2s, [edge(1, HALF), cuts[1], edge(33, HALF)]),
        // The last sixteen bits read, until the WS change decides the word.
        (
            Format::RightJustified,
            [edge(0, HALF), edge(9, HALF), edge(32, HALF)],
        ),
    ] {
        let config = bus(format, Some(16));
        let block = waveform(&config, &halves, HALF);
        let mut harness = harness(&config, GraphConfig::default());
        let mut from = Tick::ZERO;
        for (cut, held) in cuts.into_iter().zip(held) {
            let piece = block.slice(TickRange::new(from, cut)).expect("a slice");
            harness.feed(&piece, &[u64::MAX]);
            harness.collect();
            assert_eq!(
                harness.items.last(),
                Some(&Item::Progress(held)),
                "{format:?} up to {cut}"
            );
            from = cut;
        }
        harness.assert_progress_promise();
    }
}

/// Half-periods of every width, some cut short, with pauses of the clock between some.
fn rich(config: &I2sConfig) -> LogicBlock {
    let mut waveform = I2sWaveform::new(config.clone(), CHANNELS, 1, Tick::ZERO);
    waveform.idle(7);
    let slots = [32, 32, 24, 24, 16, 16, 20, 7, 32, 40, 12, 12, 1, 2, 32, 32];
    for n in 0..48u32 {
        let width = [24, 16, 32, 20][n as usize % 4];
        let channel = channel_of(n as usize);
        waveform.half(
            channel,
            word(n, width),
            width,
            slots[n as usize % slots.len()],
        );
        if n % 5 == 4 {
            waveform.idle(u64::from(n % 3) * 9 + 1);
        }
    }
    waveform.close();
    waveform.idle(5);
    waveform.finish()
}

#[test]
fn blocks_pauses_and_gaps_do_not_change_the_samples() {
    let configs = [
        bus(Format::I2s, None),
        I2sConfig {
            sample_edge: SampleEdge::Falling,
            ..bus(Format::LeftJustified, Some(24))
        },
        I2sConfig {
            ws_inverted: true,
            ..bus(Format::RightJustified, Some(16))
        },
    ];
    for config in configs {
        let block = rich(&config);
        let gaps = [
            TickRange::new(Tick(200), Tick(203)),
            TickRange::new(Tick(1000), Tick(1001)),
            TickRange::new(Tick(1500), Tick(1777)),
        ];
        let reference = decode(&config, &block, &gaps);
        assert!(reference.len() > 40, "{config:?}: {reference:?}");
        assert!(
            reference.iter().any(|line| line.contains("incomplete")) || config.bits.is_none(),
            "{config:?}: {reference:?}"
        );
        assert_block_invariant(|graph| harness(&config, graph), &block, &gaps);
    }
}

fn formats() -> impl Strategy<Value = Format> {
    prop_oneof![
        Just(Format::I2s),
        Just(Format::LeftJustified),
        Just(Format::RightJustified),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 64, ..ProptestConfig::default() })]

    /// Whatever the format, widths, slots, edges and polarity, and however the samples are
    /// cut and read, the decoder reads what the transmitter put on the bus.
    #[test]
    fn reads_what_the_transmitter_sent(
        format in formats(),
        width in prop::option::of(1u8..=32),
        falling in any::<bool>(),
        ws_inverted in any::<bool>(),
        sent in prop::collection::vec((any::<u32>(), 1u8..=32, 1u32..=40), 1..10),
        half_period in 1u64..4,
        sizes in prop::collection::vec(1u64..300, 1..5),
        pauses in prop::collection::vec(0usize..6, 1..5),
        queue_capacity in 1usize..5,
        budget in 1u32..30,
    ) {
        let bits = match format {
            Format::RightJustified => Some(width.unwrap_or(24)),
            _ => width,
        };
        let config = I2sConfig {
            format,
            bits,
            sample_edge: if falling { SampleEdge::Falling } else { SampleEdge::Rising },
            ws_inverted,
            ..I2sConfig::on(0, 1, 2)
        };
        let halves: Vec<Half> = sent
            .iter()
            .map(|&(value, bits, slots)| half(value, bits, slots))
            .collect();
        let block = waveform(&config, &halves, half_period);
        let mut expected = texts(&model(&config, &halves, half_period));
        expected.push(format!("eof {}", block.end()));

        let mut harness = harness(&config, GraphConfig { queue_capacity, budget });
        harness.drain_on_full = 1;
        harness.feed_cut(&block, &[], &sizes, &pauses);
        harness.end();
        harness.assert_progress_promise();
        prop_assert_eq!(lines(&harness.items), expected);
        let stats = harness.pipeline().stats();
        prop_assert!(stats.within_capacity(), "{stats:?}");
    }
}
