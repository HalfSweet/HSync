// SPDX-License-Identifier: Apache-2.0

//! The APA102 decoder on synthesized SPI waveforms, with and without chip select.

use proptest::prelude::*;
use uiharu_core::testing::{Harness, Tap, assert_block_invariant, without_progress};
use uiharu_core::{GraphConfig, GraphError, Item, LogicBlock, Tick, TickRange, TimeBase};
use uiharu_protocols::apa102::synth::{Led, apa102_frame, sk9822_frame, words};
use uiharu_protocols::apa102::{Apa102, Apa102Event};
use uiharu_protocols::spi::synth::SpiWaveform;
use uiharu_protocols::spi::{BitOrder, Spi, SpiConfig, SpiEvent};

/// Clock, chip select, MOSI and, for some tests, MISO.
const CHANNELS: u8 = 4;
/// Samples per half clock period: a bit takes four, a byte 32.
const HALF: u64 = 2;
const WORD: u64 = 32;

/// Clock on channel 0, data on 2, chip select on 1 if `cs`; mode 0, bytes.
fn bus(cs: bool) -> SpiConfig {
    SpiConfig {
        clk: 0,
        cs: cs.then_some(1),
        mosi: Some(2),
        miso: None,
        mode: 0,
        word_bits: 8,
        bit_order: BitOrder::MsbFirst,
        cs_active_low: true,
    }
}

/// Source → SPI → APA102, the APA102 frames being the output, the SPI words a tap.
fn try_build(
    spi: SpiConfig,
    graph_config: GraphConfig,
) -> Result<(Harness<Apa102Event>, Tap<SpiEvent>), GraphError> {
    let mut words = None;
    let mut harness = Harness::build(
        CHANNELS,
        TimeBase::new(1_000_000),
        graph_config,
        |graph, source| {
            let spi = graph.add(Spi::new(spi).expect("a valid configuration"));
            graph.connect(source.samples, Spi::input(spi))?;
            let leds = graph.add(Apa102::new());
            graph.connect(Spi::events(spi), Apa102::input(leds))?;
            words = Some(graph.expose("spi", Spi::events(spi))?);
            Ok(Apa102::events(leds))
        },
    )?;
    let tap = harness.tap(words.expect("exposed"));
    Ok((harness, tap))
}

fn harness(spi: SpiConfig, graph_config: GraphConfig) -> Harness<Apa102Event> {
    try_build(spi, graph_config).expect("builds").0
}

fn span(range: TickRange) -> String {
    format!("[{},{})", range.start, range.end)
}

/// One line per item, readable in assertions; every span is written out, empty or not.
fn describe(items: &[Item<Apa102Event>]) -> Vec<String> {
    items
        .iter()
        .map(|item| match item {
            Item::Event(event) => {
                let what = match event.payload {
                    Apa102Event::FrameBegin { frame } => format!("frame-begin f{}", frame.0),
                    Apa102Event::Led {
                        frame,
                        index,
                        brightness,
                        blue,
                        green,
                        red,
                    } => format!(
                        "led f{} #{index} {brightness} {blue:02x}{green:02x}{red:02x}",
                        frame.0
                    ),
                    Apa102Event::FrameEnd {
                        frame,
                        leds,
                        end_bits,
                        reason,
                    } => format!("frame-end f{} {leds} {end_bits} {reason:?}", frame.0),
                    Apa102Event::Error { frame, reason } => {
                        format!("error f{} {reason:?}", frame.0)
                    }
                };
                format!("{what} {}@{}", span(event.span), event.settled)
            }
            Item::Gap(range) => format!("gap {range}"),
            Item::Progress(tick) => format!("progress {tick}"),
            Item::End(tick) => format!("eof {tick}"),
        })
        .collect()
}

/// Decodes `block` less `gaps` in one piece: the lines, progress left out.
fn decode_with(spi: SpiConfig, block: &LogicBlock, gaps: &[TickRange]) -> Vec<String> {
    let mut harness = harness(spi, GraphConfig::default());
    harness.feed_with_gaps(block, gaps, &[u64::MAX]);
    harness.end();
    describe(&harness.without_progress())
}

fn decode(spi: SpiConfig, block: &LogicBlock) -> Vec<String> {
    decode_with(spi, block, &[])
}

/// A waveform of the bus, idle until `idle`.
fn waveform(spi: SpiConfig, idle: u64) -> SpiWaveform {
    let mut waveform = SpiWaveform::new(spi, CHANNELS, HALF, Tick::ZERO);
    waveform.idle(idle);
    waveform
}

/// Where byte `k` of words clocked out back to back from `p0` is: the span from its first bit's
/// rising edge to its last's, and the last's tick, where it is settled.
fn byte(p0: u64, k: u64) -> (u64, u64, u64) {
    let first = p0 + WORD * k + HALF;
    (first, first + WORD - 3, first + WORD - 4)
}

/// The span of bytes `from..=to` and the settled tick of byte `settled_by`.
fn bytes(p0: u64, from: u64, to: u64, settled_by: u64) -> String {
    format!(
        "[{},{})@{}",
        byte(p0, from).0,
        byte(p0, to).1,
        byte(p0, settled_by).2
    )
}

fn led(frame: u64, index: u64, value: Led, at: String) -> String {
    format!(
        "led f{frame} #{index} {} {:02x}{:02x}{:02x} {at}",
        value.brightness, value.blue, value.green, value.red
    )
}

fn four() -> [Led; 4] {
    [
        Led::new(31, 0x10, 0x20, 0x30),
        // Red is zero: the LED frame could hold the start of a start frame until the next
        // byte shows it does not.
        Led::new(1, 0x00, 0x80, 0x00),
        Led::new(16, 0xaa, 0xbb, 0xcc),
        Led::new(0, 0x01, 0x02, 0x03),
    ]
}

#[test]
fn a_frame_of_four_leds_with_its_end_frame() {
    let leds = four();
    let mut waveform = waveform(bus(true), 10);
    waveform.select();
    let p0 = waveform.position().0;
    waveform.words(words(&apa102_frame(&leds, 1)));
    waveform.release();
    let release = waveform.position().0;
    waveform.idle(20);
    let block = waveform.finish();
    assert_eq!(release, p0 + 21 * WORD + HALF);
    assert_eq!(
        decode(bus(true), &block),
        vec![
            format!("frame-begin f1 {}", bytes(p0, 0, 3, 4)),
            led(1, 0, leds[0], bytes(p0, 4, 7, 7)),
            led(1, 1, leds[1], bytes(p0, 8, 11, 12)),
            led(1, 2, leds[2], bytes(p0, 12, 15, 15)),
            led(1, 3, leds[3], bytes(p0, 16, 19, 19)),
            // The end frame's byte is counted, up to the release, which ends the frame.
            format!(
                "frame-end f1 4 8 Release [{},{})@{release}",
                byte(p0, 20).0,
                release + 1
            ),
            format!("eof {}", block.end()),
        ]
    );
}

#[test]
fn frames_back_to_back_without_chip_select() {
    let leds = four();
    let mut waveform = waveform(bus(false), 10);
    let p0 = waveform.position().0;
    let mut stream = apa102_frame(&leds[..2], 1);
    stream.extend(apa102_frame(&leds[1..], 1));
    waveform.words(words(&stream));
    waveform.idle(40);
    let block = waveform.finish();
    let end = block.end().0;
    // Frame 1 is bytes 0 to 12, frame 2 bytes 13 to 29. The end frame's 0xff at byte 12
    // begins what looks like an LED frame, into which frame 2's start frame reaches: it is
    // the end frame, eight bits.
    assert_eq!(
        decode(bus(false), &block),
        vec![
            format!("frame-begin f1 {}", bytes(p0, 0, 3, 4)),
            led(1, 0, leds[0], bytes(p0, 4, 7, 7)),
            led(1, 1, leds[1], bytes(p0, 8, 11, 12)),
            format!(
                "frame-end f1 2 8 NextFrame [{},{})@{}",
                byte(p0, 12).0,
                byte(p0, 13).0,
                byte(p0, 17).2
            ),
            format!("frame-begin f2 {}", bytes(p0, 13, 16, 17)),
            led(2, 0, leds[1], bytes(p0, 17, 20, 21)),
            led(2, 1, leds[2], bytes(p0, 21, 24, 24)),
            led(2, 2, leds[3], bytes(p0, 25, 28, 28)),
            // The last frame has no next one: the end of the samples cuts it.
            format!(
                "frame-end f2 3 8 Aborted [{},{})@{end}",
                byte(p0, 29).0,
                byte(p0, 29).1
            ),
            format!("eof {end}"),
        ]
    );
}

#[test]
fn an_sk9822_reset_frame_is_counted_with_the_end_frame() {
    let leds = [
        Led::new(5, 1, 2, 3),
        Led::new(6, 4, 5, 6),
        Led::new(7, 7, 8, 9),
        Led::new(8, 9, 9, 0),
    ];
    let mut stream = sk9822_frame(&leds, 1);
    stream.extend(sk9822_frame(&leds[..2], 1));
    // Without chip select: frame 1's LED frames are followed by nine zero bytes, the reset
    // frame, one byte of end frame and frame 2's start frame.
    let mut unframed = waveform(bus(false), 10);
    let p0 = unframed.position().0;
    unframed.words(words(&stream));
    unframed.idle(40);
    let lines = decode(bus(false), &unframed.finish());
    assert_eq!(
        lines[4],
        // The last LED ends in a zero byte; four zero bytes after it show it is an LED.
        led(1, 3, leds[3], bytes(p0, 16, 19, 23))
    );
    assert_eq!(
        lines[5],
        format!(
            "frame-end f1 4 40 NextFrame [{},{})@{}",
            byte(p0, 20).0,
            byte(p0, 25).0,
            byte(p0, 29).2
        )
    );
    assert_eq!(
        lines[6],
        format!("frame-begin f2 {}", bytes(p0, 25, 28, 29))
    );
    assert!(
        lines[9].starts_with("frame-end f2 2 40 Aborted "),
        "{lines:?}"
    );
    // With chip select, one frame per transaction: the release ends each.
    let mut waveform = waveform(bus(true), 10);
    waveform.transaction(words(&sk9822_frame(&leds, 1)));
    waveform.idle(10);
    waveform.transaction(words(&sk9822_frame(&leds[..2], 1)));
    waveform.idle(10);
    let ends: Vec<String> = decode(bus(true), &waveform.finish())
        .into_iter()
        .filter(|line| line.starts_with("frame-end"))
        .map(|line| line.split(" [").next().unwrap_or_default().to_owned())
        .collect();
    assert_eq!(
        ends,
        vec!["frame-end f1 4 40 Release", "frame-end f2 2 40 Release"]
    );
}

#[test]
fn a_transaction_cut_off_aborts_its_frame() {
    let leds = four();
    let mut waveform = waveform(bus(true), 10);
    waveform.select();
    let p0 = waveform.position().0;
    waveform.words(words(&apa102_frame(&leds, 1)));
    waveform.release();
    waveform.idle(20);
    let block = waveform.finish();
    // The samples end in the middle of byte 14, with chip select still asserted.
    let cut = byte(p0, 14).0 + 10;
    let head = block
        .slice(TickRange::new(Tick::ZERO, Tick(cut)))
        .expect("a slice");
    assert_eq!(
        decode(bus(true), &head),
        vec![
            format!("frame-begin f1 {}", bytes(p0, 0, 3, 4)),
            led(1, 0, leds[0], bytes(p0, 4, 7, 7)),
            led(1, 1, leds[1], bytes(p0, 8, 11, 12)),
            // Bytes 12 and 13 began an LED frame that was cut off: they are counted.
            format!(
                "frame-end f1 2 16 Aborted [{},{})@{cut}",
                byte(p0, 12).0,
                byte(p0, 13).1
            ),
            format!("eof {cut}"),
        ]
    );
    // A gap cuts it just the same, at its start: byte 4 began LED 0. After the gap the
    // transaction is partial, its words shifted, and nothing in it is a frame.
    let gap = TickRange::new(Tick(byte(p0, 5).0), Tick(byte(p0, 9).0));
    let lines = decode_with(bus(true), &block, &[gap]);
    assert_eq!(
        lines[1],
        format!(
            "frame-end f1 0 8 Aborted [{},{})@{}",
            byte(p0, 4).0,
            byte(p0, 4).1,
            gap.start
        )
    );
    assert_eq!(lines[2], format!("gap {gap}"));
    assert_eq!(lines.len(), 4, "{lines:?}");
}

#[test]
fn a_gap_aborts_the_frame_and_the_next_start_frame_resynchronizes() {
    let leds = four();
    let mut waveform = waveform(bus(false), 10);
    let p0 = waveform.position().0;
    let mut stream = apa102_frame(&leds, 1);
    stream.extend(apa102_frame(&leds[2..], 1));
    waveform.words(words(&stream));
    waveform.idle(40);
    let block = waveform.finish();
    // Inside LED 2 of frame 1: what follows the gap is no frame until frame 2's start frame.
    // The gap ends between two bytes, where the clock idles, so the words after it line up
    // with the bytes again; ending inside a byte, it would shift every later word.
    let gap = TickRange::new(Tick(byte(p0, 13).0 + 3), Tick(p0 + 15 * WORD));
    let lines = decode_with(bus(false), &block, &[gap]);
    assert_eq!(
        lines,
        vec![
            format!("frame-begin f1 {}", bytes(p0, 0, 3, 4)),
            led(1, 0, leds[0], bytes(p0, 4, 7, 7)),
            led(1, 1, leds[1], bytes(p0, 8, 11, 12)),
            // Byte 12 began LED 2; it is counted.
            format!(
                "frame-end f1 2 8 Aborted [{},{})@{}",
                byte(p0, 12).0,
                byte(p0, 12).1,
                gap.start
            ),
            format!("gap {gap}"),
            format!("frame-begin f2 {}", bytes(p0, 21, 24, 25)),
            led(2, 0, leds[2], bytes(p0, 25, 28, 28)),
            led(2, 1, leds[3], bytes(p0, 29, 32, 32)),
            format!(
                "frame-end f2 2 8 Aborted [{},{})@{}",
                byte(p0, 33).0,
                byte(p0, 33).1,
                block.end()
            ),
            format!("eof {}", block.end()),
        ]
    );
}

#[test]
fn a_frame_under_way_at_the_first_sample_yields_nothing_until_a_start_frame() {
    let leds = four();
    let mut stream = apa102_frame(&leds, 1);
    stream.extend(apa102_frame(&leds[..1], 1));
    for cs in [false, true] {
        let mut waveform = waveform(bus(cs), 10);
        if cs {
            waveform.select();
        }
        let p0 = waveform.position().0;
        waveform.words(words(&stream));
        if cs {
            waveform.release();
        }
        waveform.idle(20);
        let block = waveform.finish();
        // From the middle of LED 1, between two of its bytes: with chip select the
        // transaction is partial.
        let from = p0 + 9 * WORD;
        let tail = block
            .slice(TickRange::new(Tick(from), block.end()))
            .expect("a slice")
            .rebased(Tick::ZERO);
        let lines = decode(bus(cs), &tail);
        // Byte 9 is the tail's first: byte `k` of the stream is byte `k - 9` there.
        assert_eq!(
            lines[0],
            format!("frame-begin f1 {}", bytes(0, 12, 15, 16)),
            "cs {cs}"
        );
        assert_eq!(lines[1], led(1, 0, leds[0], bytes(0, 16, 19, 19)));
        assert_eq!(lines.len(), 4, "cs {cs}: {lines:?}");
    }
}

#[test]
fn a_frame_without_the_header_bits_is_an_error() {
    let mut stream = vec![0, 0, 0, 0, 0x5a, 0xe1, 1, 2, 3];
    stream.extend(apa102_frame(&[Led::new(3, 4, 5, 6)], 1));
    let mut waveform = waveform(bus(false), 10);
    let p0 = waveform.position().0;
    waveform.words(words(&stream));
    waveform.idle(10);
    let block = waveform.finish();
    assert_eq!(
        decode(bus(false), &block)[..4],
        [
            format!("frame-begin f1 {}", bytes(p0, 0, 3, 4)),
            format!("error f1 Header {}", bytes(p0, 4, 4, 4)),
            // Everything up to the next start frame is end bits, LED-like bytes included.
            format!(
                "frame-end f1 0 40 NextFrame [{},{})@{}",
                byte(p0, 4).0,
                byte(p0, 9).0,
                byte(p0, 13).2
            ),
            format!("frame-begin f2 {}", bytes(p0, 9, 12, 13)),
        ]
    );
}

#[test]
fn the_datasheet_end_frame_of_ones_reads_as_an_led() {
    let leds = [Led::new(2, 1, 1, 1)];
    let mut waveform = waveform(bus(true), 10);
    waveform.select();
    let p0 = waveform.position().0;
    waveform.words(words(&apa102_frame(&leds, 4)));
    waveform.release();
    waveform.idle(10);
    let lines = decode(bus(true), &waveform.finish());
    // 32 one bits are an LED frame of brightness 31, full white: only the length of the
    // chain tells them apart.
    assert_eq!(
        lines[2],
        led(1, 1, Led::new(31, 0xff, 0xff, 0xff), bytes(p0, 8, 11, 11))
    );
    assert!(
        lines[3].starts_with("frame-end f1 2 0 Release "),
        "{lines:?}"
    );
}

#[test]
fn an_led_ending_in_zero_bytes_shows_up_with_the_next_byte() {
    let leds = four();
    let mut waveform = waveform(bus(false), 10);
    let p0 = waveform.position().0;
    waveform.words(words(&apa102_frame(&leds, 1)));
    waveform.idle(40);
    let block = waveform.finish();
    let mut harness = harness(bus(false), GraphConfig::default());
    let mut from = Tick::ZERO;
    // LED 1 ends with byte 11, a zero; byte 12 shows it is no start frame.
    for (cut, seen) in [
        (byte(p0, 11).2 + 1, 2),
        (byte(p0, 12).2, 2),
        (byte(p0, 12).2 + 1, 3),
    ] {
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
fn the_spi_stream_must_carry_bytes_on_mosi_msb_first() {
    let refused = |spi: SpiConfig| match try_build(spi, GraphConfig::default()) {
        Err(GraphError::Requirement {
            key,
            requirement,
            found,
            ..
        }) => format!("{key} {requirement}, found {found}"),
        Err(other) => format!("{other}"),
        Ok(_) => "accepted".to_owned(),
    };
    let four_bits = SpiConfig {
        word_bits: 4,
        ..bus(false)
    };
    assert_eq!(
        try_build(four_bits.clone(), GraphConfig::default())
            .err()
            .map(|error| error.to_string()),
        Some("apa102#1.bus requires wordBits = 8 of spi#0.events, which declares 4".to_owned())
    );
    assert_eq!(refused(four_bits), "wordBits = 8, found 4");
    assert_eq!(
        refused(SpiConfig {
            bit_order: BitOrder::LsbFirst,
            ..bus(true)
        }),
        "bitOrder = \"msb-first\", found \"lsb-first\""
    );
    assert_eq!(
        refused(SpiConfig {
            mosi: None,
            miso: Some(2),
            ..bus(false)
        }),
        "mosi = true, found false"
    );
    // Chip select and MISO may be there or not.
    for cs in [false, true] {
        assert_eq!(refused(bus(cs)), "accepted");
        assert_eq!(
            refused(SpiConfig {
                miso: Some(3),
                ..bus(cs)
            }),
            "accepted"
        );
    }
}

/// Traffic of every kind: frames of all sizes, end frames of ones and of zeros and of none,
/// SK9822 reset frames, LEDs ending in zero bytes, a header error, junk between frames.
fn busy_stream() -> Vec<u8> {
    let mut stream = vec![0x12, 0xe5, 0x00, 0x00];
    for n in 0..8u8 {
        let leds: Vec<Led> = (0..n % 5 + 1)
            .map(|i| {
                let red = if (n + i) % 3 == 0 { 0 } else { n ^ i };
                Led::new(n + i, i.wrapping_mul(53), n.wrapping_mul(17), red)
            })
            .collect();
        match n % 4 {
            0 => stream.extend(apa102_frame(&leds, 1)),
            1 => stream.extend(sk9822_frame(&leds, 2)),
            2 => stream.extend(apa102_frame(&leds, 0)),
            _ => {
                stream.extend(apa102_frame(&leds, 3));
                stream.extend([0x00, 0x00, 0x00, 0x00, 0x07, 0x99]);
            }
        }
    }
    stream
}

fn busy_block(cs: bool) -> LogicBlock {
    let stream = busy_stream();
    let mut waveform = waveform(bus(cs), 17);
    if cs {
        // A transaction per 23 bytes: frames are cut by releases.
        for chunk in stream.chunks(23) {
            waveform.transaction(words(chunk));
            waveform.idle(9);
        }
    } else {
        waveform.words(words(&stream));
        waveform.idle(9);
    }
    waveform.finish()
}

#[test]
fn blocks_pauses_and_gaps_do_not_change_the_frames() {
    for cs in [false, true] {
        let block = busy_block(cs);
        let len = block.len();
        let gaps = [
            TickRange::new(Tick(len / 4), Tick(len / 4 + 45)),
            TickRange::new(Tick(len / 2 + 3), Tick(len / 2 + 4)),
            TickRange::new(Tick(3 * len / 4), Tick(3 * len / 4 + 300)),
        ];
        let reference = decode_with(bus(cs), &block, &gaps);
        for wanted in ["frame-begin", "led f", " Aborted ", "Header"] {
            assert!(
                reference.iter().any(|line| line.contains(wanted)),
                "cs {cs}: {wanted} in {reference:?}"
            );
        }
        let ended = if cs { " Release " } else { " NextFrame " };
        assert!(
            reference.iter().any(|line| line.contains(ended)),
            "cs {cs}: {reference:?}"
        );
        assert_block_invariant(|graph| harness(bus(cs), graph), &block, &gaps);
    }
}

#[test]
fn progress_never_passes_a_byte_not_yet_reported() {
    for cs in [false, true] {
        let block = busy_block(cs);
        for sizes in [&[1u64][..], &[13], &[64, 7], &[500]] {
            let mut harness = harness(bus(cs), GraphConfig::default());
            harness.feed_cut(&block, &[], sizes, &[1]);
            harness.end();
            let mut seen = Tick::ZERO;
            for item in &harness.items {
                match item {
                    Item::Progress(tick) => seen = seen.max(*tick),
                    Item::Event(event) => {
                        assert!(event.span.start >= seen, "{event:?} after {seen}");
                    }
                    Item::Gap(_) | Item::End(_) => {}
                }
            }
        }
    }
}

/// A byte as the SPI stream carried it: value, span and settled tick.
type SpiByte = (u8, u64, u64, u64);

fn is_header(byte: u8) -> bool {
    byte & 0xe0 == 0xe0
}

/// The decoder's rules applied with the whole of a stretch of bytes in view — every start frame
/// found first, then each frame read up to the next — rather than byte by byte: what the
/// decoder must report for `bytes`, which stop at `stop` (a gap's start or the end).
fn model_stretch(bytes: &[SpiByte], stop: u64, next_id: &mut u64, lines: &mut Vec<String>) {
    // Each start frame is the last four zero bytes of a run of four or more before a byte that
    // is not zero; the index kept is that byte's.
    let mut starts = Vec::new();
    let mut run = 0;
    for (i, &(value, ..)) in bytes.iter().enumerate() {
        if value == 0 {
            run += 1;
        } else {
            if run >= 4 {
                starts.push(i);
            }
            run = 0;
        }
    }
    for (k, &first) in starts.iter().enumerate() {
        let id = *next_id;
        *next_id += 1;
        let next = starts.get(k + 1).copied();
        // The frame's bytes run up to the next start frame, or to the stop.
        let limit = next.map_or(bytes.len(), |next| next - 4);
        let decided = bytes[first].3;
        lines.push(format!(
            "frame-begin f{id} [{},{})@{decided}",
            bytes[first - 4].1,
            bytes[first - 1].2
        ));
        let mut i = first;
        let mut leds = 0u32;
        if !is_header(bytes[first].0) {
            let (_, start, end, settled) = bytes[first];
            lines.push(format!("error f{id} Header [{start},{end})@{settled}"));
        } else {
            while i + 4 <= limit && is_header(bytes[i].0) {
                // An LED frame ending in zero bytes is settled by the first byte after it that
                // is not zero, by the fourth zero after it, or by the stop.
                let settled = if bytes[i + 3].0 != 0 {
                    bytes[i + 3].3
                } else {
                    let mut zeros = 0;
                    let mut j = i + 4;
                    loop {
                        match bytes.get(j) {
                            None => break stop,
                            Some(&(0, .., settled)) => {
                                zeros += 1;
                                if zeros == 4 {
                                    break settled;
                                }
                            }
                            Some(&(.., settled)) => break settled,
                        }
                        j += 1;
                    }
                };
                let [header, blue, green, red] = [0, 1, 2, 3].map(|n| bytes[i + n].0);
                lines.push(format!(
                    "led f{id} #{leds} {} {blue:02x}{green:02x}{red:02x} [{},{})@{settled}",
                    header & 0x1f,
                    bytes[i].1,
                    bytes[i + 3].2
                ));
                leds += 1;
                i += 4;
            }
        }
        let end_bits = 8 * (limit - i);
        match next {
            Some(next) => {
                let start = bytes[next - 4].1;
                let from = if limit > i { bytes[i].1 } else { start };
                lines.push(format!(
                    "frame-end f{id} {leds} {end_bits} NextFrame [{from},{start})@{}",
                    bytes[next].3
                ));
            }
            None => {
                let (from, to) = if limit > i {
                    (bytes[i].1, bytes[limit - 1].2)
                } else {
                    (stop, stop)
                };
                lines.push(format!(
                    "frame-end f{id} {leds} {end_bits} Aborted [{from},{to})@{stop}"
                ));
            }
        }
    }
}

/// The model over a whole SPI stream without chip select: stretches of words between gaps.
fn model(spi: &[Item<SpiEvent>]) -> Vec<String> {
    let mut lines = Vec::new();
    let mut next_id = 1;
    let mut stretch: Vec<SpiByte> = Vec::new();
    for item in spi {
        match item {
            Item::Event(event) => {
                if let SpiEvent::Word {
                    mosi: Some(value), ..
                } = event.payload
                {
                    stretch.push((value, event.span.start.0, event.span.end.0, event.settled.0));
                }
            }
            Item::Gap(range) => {
                model_stretch(&stretch, range.start.0, &mut next_id, &mut lines);
                stretch.clear();
                lines.push(format!("gap {range}"));
            }
            Item::End(tick) => {
                model_stretch(&stretch, tick.0, &mut next_id, &mut lines);
                lines.push(format!("eof {tick}"));
            }
            Item::Progress(_) => {}
        }
    }
    lines
}

#[test]
fn the_model_agrees_on_the_busy_stream() {
    let block = busy_block(false);
    let gaps = [TickRange::new(Tick(2000), Tick(2100))];
    let (mut harness, spi) = try_build(bus(false), GraphConfig::default()).expect("builds");
    harness.feed_with_gaps(&block, &gaps, &[u64::MAX]);
    harness.end();
    let lines = describe(&harness.without_progress());
    assert!(lines.len() > 20, "{lines:?}");
    assert_eq!(lines, model(&spi.items()));
}

/// A piece of a random byte stream.
#[derive(Clone, Debug)]
enum Piece {
    /// A frame: its LED frames, then `end` bytes of `fill`.
    Frame {
        leds: Vec<Led>,
        end: usize,
        fill: u8,
    },
    /// Bytes of anything.
    Junk(Vec<u8>),
}

fn piece() -> impl Strategy<Value = Piece> {
    let led = (
        0u8..32,
        any::<u8>(),
        any::<u8>(),
        prop_oneof![1 => Just(0u8), 2 => any::<u8>()],
    )
        .prop_map(|(brightness, blue, green, red)| Led::new(brightness, blue, green, red));
    prop_oneof![
        4 => (
            prop::collection::vec(led, 0..6),
            0usize..9,
            prop_oneof![Just(0x00u8), Just(0xffu8), any::<u8>()],
        )
            .prop_map(|(leds, end, fill)| Piece::Frame { leds, end, fill }),
        1 => prop::collection::vec(prop_oneof![2 => Just(0u8), 1 => any::<u8>()], 0..12)
            .prop_map(Piece::Junk),
    ]
}

fn stream_of(pieces: &[Piece]) -> Vec<u8> {
    let mut stream = Vec::new();
    for piece in pieces {
        match piece {
            Piece::Frame { leds, end, fill } => {
                stream.extend([0u8; 4]);
                for led in leds {
                    stream.extend(led.frame());
                }
                stream.extend(std::iter::repeat_n(*fill, *end));
            }
            Piece::Junk(bytes) => stream.extend(bytes),
        }
    }
    stream
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 48, ..ProptestConfig::default() })]

    /// Random streams without chip select read in blocks of any size, by a reader that pauses,
    /// through tiny queues and budgets, give what the model gives; with chip select, cut into
    /// random transactions, they give the same however they come.
    #[test]
    fn random_streams_decode_as_the_model_says(
        pieces in prop::collection::vec(piece(), 1..8),
        cs in any::<bool>(),
        chunk in 1usize..40,
        sizes in prop::collection::vec(1u64..400, 1..6),
        pauses in prop::collection::vec(0usize..12, 1..8),
        queue_capacity in 1usize..6,
        budget in 1u32..40,
        gap in prop::option::of((0.05f64..0.95, 1u64..300)),
    ) {
        let stream = stream_of(&pieces);
        let mut waveform = waveform(bus(cs), 11);
        if cs {
            for transaction in stream.chunks(chunk) {
                waveform.transaction(words(transaction));
                waveform.idle(5);
            }
        } else {
            waveform.words(words(&stream));
        }
        waveform.idle(7);
        let block = waveform.finish();
        let gaps: Vec<TickRange> = gap
            .and_then(|(at, len)| {
                let start = ((block.len() as f64) * at) as u64;
                let end = (start + len).min(block.len());
                (start < end).then(|| TickRange::new(Tick(start), Tick(end)))
            })
            .into_iter()
            .collect();
        let (mut reference, spi) =
            try_build(bus(cs), GraphConfig::default()).expect("builds");
        reference.feed_with_gaps(&block, &gaps, &[u64::MAX]);
        reference.end();
        let lines = describe(&reference.without_progress());
        if !cs {
            prop_assert_eq!(&lines, &model(&spi.items()));
        }

        let mut harness = harness(bus(cs), GraphConfig { queue_capacity, budget });
        harness.drain_on_full = 1;
        harness.feed_cut(&block, &gaps, &sizes, &pauses);
        harness.end();
        prop_assert_eq!(describe(&without_progress(&harness.items)), lines);
        let stats = harness.pipeline().stats();
        prop_assert!(stats.within_capacity(), "{:?}", stats);

        // Every frame that begins ends once, with as many LEDs as it reported, in order.
        let mut open: Option<(u64, u32)> = None;
        let mut settled = Tick::ZERO;
        for event in reference.events() {
            prop_assert!(event.settled >= settled, "{:?} settled before {:?}", event, settled);
            settled = event.settled;
            match event.payload {
                Apa102Event::FrameBegin { frame } => {
                    prop_assert!(open.is_none(), "two frames open");
                    open = Some((frame.0, 0));
                }
                Apa102Event::Led { frame, index, brightness, .. } => {
                    let (id, leds) = open.as_mut().expect("an open frame");
                    prop_assert_eq!((frame.0, index), (*id, *leds));
                    prop_assert!(brightness < 32);
                    *leds += 1;
                }
                Apa102Event::Error { frame, .. } => {
                    prop_assert_eq!(Some(frame.0), open.map(|(id, _)| id));
                }
                Apa102Event::FrameEnd { frame, leds, .. } => {
                    let (id, counted) = open.take().expect("an open frame");
                    prop_assert_eq!((frame.0, leds), (id, counted));
                }
            }
        }
        prop_assert!(open.is_none(), "a frame left open");
    }
}
