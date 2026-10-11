// SPDX-License-Identifier: Apache-2.0

//! The AVR serial programming decoder on synthesized SPI waveforms.

use serde::Serialize;
use uiharu_core::testing::{Harness, assert_block_invariant};
use uiharu_core::{GraphConfig, GraphError, Item, LogicBlock, Tick, TickRange, TimeBase};
use uiharu_protocols::avr_isp::{AvrIsp, IspEvent, synth};
use uiharu_protocols::spi::synth::SpiWaveform;
use uiharu_protocols::spi::{BitOrder, Spi, SpiConfig};

const CHANNELS: u8 = 4;
const RATE: u64 = 1_000_000;
const HALF: u64 = 2;
/// Samples per word: eight bits of two half periods.
const WORD: u64 = 8 * 2 * HALF;
/// Where chip select (RESET) asserts in [`waveform`].
const SELECT: u64 = 10;

/// The SPI bus with RESET as its chip select.
fn framed() -> SpiConfig {
    SpiConfig::default()
}

/// The SPI bus without RESET connected.
fn unframed() -> SpiConfig {
    SpiConfig {
        cs: None,
        ..SpiConfig::default()
    }
}

fn build(spi: SpiConfig, graph: GraphConfig) -> Result<Harness<IspEvent>, GraphError> {
    Harness::build(CHANNELS, TimeBase::new(RATE), graph, |graph, source| {
        let spi = graph.add(Spi::new(spi).expect("a valid configuration"));
        graph.connect(source.samples, Spi::input(spi))?;
        let isp = graph.add(AvrIsp::new());
        graph.connect(Spi::events(spi), AvrIsp::input(isp))?;
        Ok(AvrIsp::events(isp))
    })
}

fn harness(spi: SpiConfig) -> Harness<IspEvent> {
    build(spi, GraphConfig::default()).expect("builds")
}

fn waveform(spi: SpiConfig) -> SpiWaveform {
    let mut waveform = SpiWaveform::new(spi, CHANNELS, HALF, Tick::ZERO);
    waveform.idle(SELECT);
    waveform
}

/// The first bit of word `n` of a transaction that began at [`SELECT`].
fn first_bit(n: u64) -> u64 {
    SELECT + 2 * HALF + n * WORD
}

/// The last bit of word `n` of a transaction that began at [`SELECT`].
fn last_bit(n: u64) -> u64 {
    first_bit(n) + 7 * 2 * HALF
}

/// The span and settled tick of words `from..to` of a transaction that began at [`SELECT`].
fn words(from: u64, to: u64) -> String {
    format!(
        "[{},{})@{}",
        first_bit(from),
        last_bit(to - 1) + 1,
        last_bit(to - 1)
    )
}

/// The tick chip select is released after `count` words of a transaction that began at
/// [`SELECT`].
fn release(count: u64) -> u64 {
    first_bit(count)
}

fn name(value: &impl Serialize) -> String {
    serde_json::to_value(value)
        .expect("serializes")
        .as_str()
        .expect("a name")
        .to_owned()
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

/// One line per item, progress left out; `ticks` adds where each event lies.
fn describe(items: &[Item<IspEvent>], ticks: bool) -> Vec<String> {
    uiharu_core::testing::without_progress(items)
        .iter()
        .map(|item| match item {
            Item::Event(event) => {
                let what = match &event.payload {
                    IspEvent::Instruction {
                        op,
                        tx,
                        command,
                        address,
                        data_in,
                        data_out,
                        in_sync,
                        bytes,
                    } => format!(
                        "instruction op{} tx{} {} addr={} in={} out={} sync={} {}",
                        op.0,
                        tx.0,
                        name(command),
                        address.map_or("----".to_owned(), |address| format!("{address:04x}")),
                        data_in.map_or("--".to_owned(), |byte| format!("{byte:02x}")),
                        data_out.map_or("--".to_owned(), |byte| format!("{byte:02x}")),
                        in_sync.map_or("-", |sync| if sync { "yes" } else { "no" }),
                        hex(bytes)
                    ),
                    IspEvent::Unknown { tx, bytes } => format!("unknown tx{} {}", tx.0, hex(bytes)),
                    IspEvent::Error { tx, reason, bytes } => {
                        format!("error tx{} {} {}", tx.0, name(reason), hex(bytes))
                    }
                };
                if ticks {
                    format!("{what} {}", at(event.span, event.settled))
                } else {
                    what
                }
            }
            Item::Gap(range) => format!("gap {range}"),
            Item::Progress(tick) => format!("progress {tick}"),
            Item::End(tick) => format!("eof {tick}"),
        })
        .collect()
}

/// The events as lines without ticks, the end of the stream left out.
fn lines(harness: &Harness<IspEvent>) -> Vec<String> {
    describe(&harness.items, false)
        .into_iter()
        .filter(|line| !line.starts_with("eof "))
        .collect()
}

/// Decodes `sessions`, each in its own chip select period, with the events' ticks.
fn decode_with_ticks(sessions: &[Vec<Vec<(u8, u8)>>]) -> Vec<String> {
    let mut waveform = waveform(framed());
    for session in sessions {
        waveform.select();
        for instruction in session {
            waveform.words(instruction.iter().copied());
        }
        waveform.release();
        waveform.idle(10);
    }
    let block = waveform.finish();
    let mut harness = harness(framed());
    harness.run_all(&block);
    describe(&harness.items, true)
}

/// Decodes `sessions` and returns the lines without ticks.
fn decode(sessions: &[Vec<Vec<(u8, u8)>>]) -> Vec<String> {
    decode_with_ticks(sessions)
        .into_iter()
        .filter(|line| !line.starts_with("eof "))
        .map(|line| {
            line.rsplit_once(' ')
                .map_or(line.as_str(), |(what, _)| what)
                .to_owned()
        })
        .collect()
}

/// A session that reads the device, erases it, programs a page, the EEPROM and the fuses.
fn programming_session() -> Vec<Vec<(u8, u8)>> {
    vec![
        synth::programming_enable(true),
        synth::read([0x30, 0x00, 0x00], 0x1e),
        synth::read([0x30, 0x00, 0x01], 0x95),
        synth::read([0x30, 0x00, 0x02], 0x0f),
        synth::read([0x50, 0x00, 0x00], 0x62),
        synth::read([0x58, 0x08, 0x00], 0xd9),
        synth::read([0x50, 0x08, 0x00], 0xff),
        synth::read([0x58, 0x00, 0x00], 0xff),
        synth::read([0x38, 0x00, 0x00], 0x9c),
        synth::instruction([0xac, 0x80, 0x00, 0x00]),
        synth::read([0xf0, 0x00, 0x00], 0x01),
        synth::read([0xf0, 0x00, 0x00], 0x00),
        synth::instruction([0x4d, 0x00, 0x01, 0x00]),
        synth::instruction([0x40, 0x00, 0x00, 0x0c]),
        synth::instruction([0x48, 0x00, 0x00, 0x94]),
        synth::instruction([0x4c, 0x00, 0x00, 0x00]),
        synth::read([0x20, 0x00, 0x00], 0x0c),
        synth::read([0x28, 0x00, 0x00], 0x94),
        synth::instruction([0xc1, 0x00, 0x00, 0x55]),
        synth::instruction([0xc2, 0x00, 0x00, 0x00]),
        synth::instruction([0xc0, 0x01, 0x23, 0xaa]),
        synth::read([0xa0, 0x01, 0x23], 0xaa),
        synth::instruction([0xac, 0xa0, 0x00, 0xff]),
        synth::instruction([0xac, 0xa8, 0x00, 0xde]),
        synth::instruction([0xac, 0xa4, 0x00, 0xfd]),
        synth::instruction([0xac, 0xe0, 0x00, 0xfc]),
    ]
}

#[test]
fn every_instruction_is_decoded_with_its_fields() {
    assert_eq!(
        decode(&[programming_session()]),
        [
            "instruction op1 tx1 programming-enable addr=---- in=-- out=-- sync=yes ac530000",
            "instruction op2 tx1 read-signature-byte addr=0000 in=-- out=1e sync=- 30000000",
            "instruction op3 tx1 read-signature-byte addr=0001 in=-- out=95 sync=- 30000100",
            "instruction op4 tx1 read-signature-byte addr=0002 in=-- out=0f sync=- 30000200",
            "instruction op5 tx1 read-fuse-bits addr=---- in=-- out=62 sync=- 50000000",
            "instruction op6 tx1 read-fuse-high-bits addr=---- in=-- out=d9 sync=- 58080000",
            "instruction op7 tx1 read-extended-fuse-bits addr=---- in=-- out=ff sync=- 50080000",
            "instruction op8 tx1 read-lock-bits addr=---- in=-- out=ff sync=- 58000000",
            "instruction op9 tx1 read-calibration-byte addr=---- in=-- out=9c sync=- 38000000",
            "instruction op10 tx1 chip-erase addr=---- in=-- out=-- sync=- ac800000",
            "instruction op11 tx1 poll-rdy-bsy addr=---- in=-- out=01 sync=- f0000000",
            "instruction op12 tx1 poll-rdy-bsy addr=---- in=-- out=00 sync=- f0000000",
            "instruction op13 tx1 load-extended-address addr=0001 in=-- out=-- sync=- 4d000100",
            "instruction op14 tx1 load-program-memory-page-low addr=0000 in=0c out=-- sync=- \
             4000000c",
            "instruction op15 tx1 load-program-memory-page-high addr=0000 in=94 out=-- sync=- \
             48000094",
            "instruction op16 tx1 write-program-memory-page addr=0000 in=-- out=-- sync=- 4c000000",
            "instruction op17 tx1 read-program-memory-low addr=0000 in=-- out=0c sync=- 20000000",
            "instruction op18 tx1 read-program-memory-high addr=0000 in=-- out=94 sync=- 28000000",
            "instruction op19 tx1 load-eeprom-memory-page addr=0000 in=55 out=-- sync=- c1000055",
            "instruction op20 tx1 write-eeprom-memory-page addr=0000 in=-- out=-- sync=- c2000000",
            "instruction op21 tx1 write-eeprom-memory addr=0123 in=aa out=-- sync=- c00123aa",
            "instruction op22 tx1 read-eeprom-memory addr=0123 in=-- out=aa sync=- a0012300",
            "instruction op23 tx1 write-fuse-bits addr=---- in=ff out=-- sync=- aca000ff",
            "instruction op24 tx1 write-fuse-high-bits addr=---- in=de out=-- sync=- aca800de",
            "instruction op25 tx1 write-extended-fuse-bits addr=---- in=fd out=-- sync=- aca400fd",
            "instruction op26 tx1 write-lock-bits addr=---- in=fc out=-- sync=- ace000fc",
        ]
    );
}

#[test]
fn an_instruction_spans_its_four_words_and_not_the_reset() {
    let lines = decode_with_ticks(&[programming_session()]);
    assert!(
        lines[0].ends_with(&format!(" {}", words(0, 4))),
        "{lines:?}"
    );
    assert!(
        lines[2].ends_with(&format!(" {}", words(8, 12))),
        "{lines:?}"
    );
    assert!(
        lines[25].ends_with(&format!(" {}", words(100, 104))),
        "{lines:?}"
    );
    // The release after a whole instruction is nothing to report.
    assert_eq!(lines.len(), 27, "{lines:?}");
}

#[test]
fn programming_enable_reports_whether_the_device_echoed() {
    // Out of sync: the programmer pulses RESET and tries again.
    let lines = decode(&[
        vec![synth::programming_enable(false)],
        vec![
            synth::programming_enable(true),
            synth::read([0x30, 0x00, 0x00], 0x1e),
        ],
    ]);
    assert_eq!(
        lines,
        [
            "instruction op1 tx1 programming-enable addr=---- in=-- out=-- sync=no ac530000",
            "instruction op2 tx2 programming-enable addr=---- in=-- out=-- sync=yes ac530000",
            "instruction op3 tx2 read-signature-byte addr=0000 in=-- out=1e sync=- 30000000",
        ]
    );
}

#[test]
fn bytes_the_set_does_not_name_are_unknown_and_keep_the_grouping() {
    let lines = decode(&[vec![
        synth::programming_enable(true),
        synth::instruction([0xff, 0xff, 0xff, 0xff]),
        synth::instruction([0xac, 0x00, 0x12, 0x34]),
        synth::instruction([0x58, 0x01, 0x00, 0x00]),
        synth::read([0x30, 0x00, 0x00], 0x1e),
    ]]);
    assert_eq!(
        lines,
        [
            "instruction op1 tx1 programming-enable addr=---- in=-- out=-- sync=yes ac530000",
            "unknown tx1 ffffffff",
            "unknown tx1 ac001234",
            "unknown tx1 58010000",
            "instruction op2 tx1 read-signature-byte addr=0000 in=-- out=1e sync=- 30000000",
        ]
    );
}

#[test]
fn an_instruction_cut_by_the_release_of_reset_is_incomplete() {
    let mut waveform = waveform(framed());
    waveform.select();
    waveform.words(synth::programming_enable(true));
    waveform.words([(0x30, 0xff), (0x00, 0x30)]);
    waveform.release();
    waveform.idle(10);
    waveform.select();
    waveform.words(synth::programming_enable(true));
    waveform.release();
    waveform.idle(10);
    let block = waveform.finish();
    let mut harness = harness(framed());
    harness.run_all(&block);
    let lines = describe(&harness.items, true);
    let cut = release(6);
    assert_eq!(
        lines[..2],
        [
            format!(
                "instruction op1 tx1 programming-enable addr=---- in=-- out=-- sync=yes ac530000 {}",
                words(0, 4)
            ),
            format!(
                "error tx1 incomplete 3000 [{},{})@{cut}",
                first_bit(4),
                cut + 1
            ),
        ]
    );
    // The next session starts its own grouping.
    assert!(
        lines[2].starts_with("instruction op2 tx2 programming-enable "),
        "{lines:?}"
    );
    assert_eq!(lines.len(), 4, "{lines:?}");
}

/// Decodes `block` on `spi` with `gap` left out, with the events' ticks.
fn decode_gapped(spi: SpiConfig, block: &LogicBlock, gap: TickRange) -> Vec<String> {
    let mut harness = harness(spi);
    harness.feed_with_gaps(block, &[gap], &[29]);
    harness.end();
    describe(&harness.items, true)
}

#[test]
fn a_gap_aborts_the_instruction_and_the_grouping_waits_for_programming_enable() {
    let mut waveform = waveform(framed());
    waveform.select();
    waveform.words(synth::programming_enable(true));
    waveform.words(synth::read([0x30, 0x00, 0x00], 0x1e));
    waveform.words(synth::read([0x30, 0x00, 0x01], 0x95));
    // After the gap, in what is left of the session: skipped until Programming Enable.
    waveform.words(synth::read([0x50, 0x00, 0x00], 0x62));
    waveform.words(synth::programming_enable(true));
    waveform.words(synth::read([0x58, 0x00, 0x00], 0xff));
    waveform.release();
    waveform.idle(10);
    let block = waveform.finish();
    // From the last bit of word 9 to the start of word 12, so that the words after the gap
    // are aligned on bytes.
    let gap = TickRange::new(Tick(last_bit(9) - 3), Tick(first_bit(12) - HALF));
    assert_eq!(
        decode_gapped(framed(), &block, gap),
        [
            format!(
                "instruction op1 tx1 programming-enable addr=---- in=-- out=-- sync=yes ac530000 {}",
                words(0, 4)
            ),
            format!(
                "instruction op2 tx1 read-signature-byte addr=0000 in=-- out=1e sync=- 30000000 {}",
                words(4, 8)
            ),
            format!(
                "error tx1 aborted 30 [{},{})@{}",
                first_bit(8),
                gap.start,
                gap.start
            ),
            format!("gap {gap}"),
            format!(
                "instruction op3 tx2 programming-enable addr=---- in=-- out=-- sync=yes ac530000 {}",
                words(16, 20)
            ),
            format!(
                "instruction op4 tx2 read-lock-bits addr=---- in=-- out=ff sync=- 58000000 {}",
                words(20, 24)
            ),
            format!("eof {}", block.end()),
        ]
    );
}

#[test]
fn after_a_gap_inside_a_word_the_rest_of_the_session_is_skipped() {
    let mut waveform = waveform(framed());
    waveform.select();
    waveform.words(synth::programming_enable(true));
    for address in 0..3 {
        waveform.words(synth::read([0x30, 0x00, address], 0x1e + address));
    }
    waveform.words(synth::programming_enable(true));
    waveform.words(synth::read([0x30, 0x00, 0x00], 0x1e));
    waveform.release();
    waveform.idle(10);
    waveform.select();
    waveform.words(synth::programming_enable(true));
    waveform.words(synth::read([0x58, 0x00, 0x00], 0xff));
    waveform.release();
    waveform.idle(10);
    let block = waveform.finish();
    // Inside words 5 and 6: what follows is not aligned on bytes.
    let gap = TickRange::new(Tick(first_bit(5) + 9), Tick(first_bit(6) + 13));
    let lines: Vec<String> = decode_gapped(framed(), &block, gap)
        .into_iter()
        .map(|line| {
            line.rsplit_once(' ')
                .map_or(line.as_str(), |(what, _)| what)
                .to_owned()
        })
        .collect();
    assert_eq!(
        lines,
        [
            "instruction op1 tx1 programming-enable addr=---- in=-- out=-- sync=yes ac530000",
            "error tx1 aborted 30",
            "gap",
            "instruction op2 tx3 programming-enable addr=---- in=-- out=-- sync=yes ac530000",
            "instruction op3 tx3 read-lock-bits addr=---- in=-- out=ff sync=- 58000000",
            "eof",
        ]
    );
}

#[test]
fn a_session_under_way_at_the_first_sample_waits_for_programming_enable() {
    let mut waveform = SpiWaveform::new(framed(), CHANNELS, HALF, Tick::ZERO);
    // RESET is low from the first sample on, the clock idle.
    waveform.select();
    waveform.words(synth::read([0x30, 0x00, 0x01], 0x95));
    waveform.words(synth::instruction([0x40, 0x00, 0xac, 0x0c]));
    waveform.words(synth::programming_enable(false));
    waveform.words(synth::programming_enable(true));
    waveform.words(synth::read([0x30, 0x00, 0x02], 0x0f));
    waveform.release();
    waveform.idle(10);
    let block = waveform.finish();
    let mut harness = harness(framed());
    harness.run_all(&block);
    // A Programming Enable that the device does not echo is no place to start.
    assert_eq!(
        lines(&harness),
        [
            "instruction op1 tx1 programming-enable addr=---- in=-- out=-- sync=yes ac530000",
            "instruction op2 tx1 read-signature-byte addr=0002 in=-- out=0f sync=- 30000200",
        ]
    );
}

#[test]
fn without_reset_the_grouping_starts_at_programming_enable() {
    let mut waveform = waveform(unframed());
    let mut starts = Vec::new();
    let instructions = [
        // Before Programming Enable: where instructions start is not known.
        synth::read([0x30, 0x00, 0x02], 0x0f),
        synth::programming_enable(true),
        synth::read([0x30, 0x00, 0x00], 0x1e),
        synth::read([0x30, 0x00, 0x01], 0x95),
        synth::read([0x30, 0x00, 0x02], 0x0f),
        // After the gap: skipped until Programming Enable.
        synth::read([0x50, 0x00, 0x00], 0x62),
        synth::programming_enable(true),
        synth::read([0x58, 0x00, 0x00], 0xff),
    ];
    for instruction in &instructions {
        starts.push(waveform.position());
        waveform.words(instruction.iter().copied());
        waveform.idle(6);
    }
    waveform.idle(10);
    let block = waveform.finish();
    // From the second word of the fifth instruction to the start of the sixth: the words
    // after the gap are aligned on bytes.
    let gap = TickRange::new(starts[4].offset(WORD + 3), starts[5]);
    let span = |n: usize| {
        let first = starts[n].get() + HALF;
        let last = starts[n].get() + 3 * WORD + 15 * HALF;
        format!("[{first},{})@{last}", last + 1)
    };
    assert_eq!(
        decode_gapped(unframed(), &block, gap),
        [
            format!(
                "instruction op1 tx0 programming-enable addr=---- in=-- out=-- sync=yes ac530000 {}",
                span(1)
            ),
            format!(
                "instruction op2 tx0 read-signature-byte addr=0000 in=-- out=1e sync=- 30000000 {}",
                span(2)
            ),
            format!(
                "instruction op3 tx0 read-signature-byte addr=0001 in=-- out=95 sync=- 30000100 {}",
                span(3)
            ),
            format!(
                "error tx0 aborted 30 [{},{})@{}",
                starts[4].get() + HALF,
                gap.start,
                gap.start
            ),
            format!("gap {gap}"),
            format!(
                "instruction op4 tx0 programming-enable addr=---- in=-- out=-- sync=yes ac530000 {}",
                span(6)
            ),
            format!(
                "instruction op5 tx0 read-lock-bits addr=---- in=-- out=ff sync=- 58000000 {}",
                span(7)
            ),
            format!("eof {}", block.end()),
        ]
    );
}

#[test]
fn the_bus_must_meet_the_decoders_requirements() {
    let refused = |spi: SpiConfig| build(spi, GraphConfig::default()).expect_err("refused");
    assert_eq!(
        refused(SpiConfig {
            word_bits: 7,
            ..framed()
        }),
        GraphError::Requirement {
            from: "spi#0.events".into(),
            to: "avr-isp#1.bus".into(),
            key: "wordBits",
            requirement: "= 8".into(),
            found: "7".into(),
        }
    );
    assert!(matches!(
        refused(SpiConfig {
            bit_order: BitOrder::LsbFirst,
            ..framed()
        }),
        GraphError::Requirement {
            key: "bitOrder",
            ..
        }
    ));
    assert!(matches!(
        refused(SpiConfig {
            mosi: None,
            ..framed()
        }),
        GraphError::Requirement { key: "mosi", .. }
    ));
    assert!(matches!(
        refused(SpiConfig {
            miso: None,
            ..framed()
        }),
        GraphError::Requirement { key: "miso", .. }
    ));
    // Chip select is not required.
    assert!(build(unframed(), GraphConfig::default()).is_ok());
}

/// Four sessions of every kind of instruction with unknown ones among them, the second
/// beginning with a Programming Enable out of sync, the third ending in an instruction cut by
/// RESET; with where each begins and ends.
fn rich_waveform(spi: SpiConfig) -> (LogicBlock, Vec<(Tick, Tick)>) {
    let mut waveform = waveform(spi);
    let session = programming_session();
    let mut sessions = Vec::new();
    for round in 0..4usize {
        let start = waveform.position();
        waveform.select();
        if round == 1 {
            waveform.words(synth::programming_enable(false));
        }
        for (n, instruction) in session.iter().enumerate() {
            waveform.words(instruction.iter().copied());
            if n % 5 == round {
                waveform.words(synth::instruction([0x12, 0x34, 0x56, 0x78]));
            }
        }
        if round == 2 {
            waveform.words([(0x28, 0xff), (0x00, 0x28), (0x10, 0x00)]);
        }
        waveform.release();
        sessions.push((start, waveform.position()));
        waveform.idle(7 + round as u64);
    }
    (waveform.finish(), sessions)
}

#[test]
fn every_cut_and_pause_yields_the_same_events() {
    for spi in [framed(), unframed()] {
        let (block, sessions) = rich_waveform(spi.clone());
        let gaps = [
            // Inside the first session's instructions.
            TickRange::new(sessions[0].0.offset(2000), sessions[0].0.offset(2003)),
            // Between the second session and the third.
            TickRange::new(sessions[1].1.offset(1), sessions[1].1.offset(5)),
            // Across several words of the last session.
            TickRange::new(
                sessions[3].0.offset(1500),
                sessions[3].0.offset(1500 + 5 * WORD + 7),
            ),
            // In the idle time at the end.
            TickRange::new(Tick(block.end().get() - 5), Tick(block.end().get() - 2)),
        ];
        let reference = {
            let mut harness = harness(spi.clone());
            harness.feed_with_gaps(&block, &gaps, &[u64::MAX]);
            harness.end();
            describe(&harness.items, true)
        };
        // Without RESET, the gap inside a word leaves the SPI words unaligned until the gap
        // between sessions: the second session is lost there.
        assert!(
            reference
                .iter()
                .filter(|line| line.starts_with("instruction "))
                .count()
                > 35,
            "{reference:?}"
        );
        for what in ["unknown ", "error ", "gap ", "instruction op2 "] {
            assert!(
                reference.iter().any(|line| line.starts_with(what)),
                "{what}: {reference:?}"
            );
        }
        assert_block_invariant(
            |graph| build(spi.clone(), graph).expect("builds"),
            &block,
            &gaps,
        );
    }
}

#[test]
fn progress_never_passes_an_instruction_under_way() {
    for spi in [framed(), unframed()] {
        let (block, _) = rich_waveform(spi.clone());
        for sizes in [&[1u64][..], &[5], &[7, 64], &[97]] {
            let mut harness = harness(spi.clone());
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
}
