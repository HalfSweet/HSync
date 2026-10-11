// SPDX-License-Identifier: Apache-2.0

//! The SD card decoder in SPI mode on synthesized SPI waveforms.

use proptest::prelude::*;
use uiharu_core::testing::{self, Harness};
use uiharu_core::{GraphConfig, GraphError, Item, Tick, TickRange, TimeBase};
use uiharu_protocols::sdcard_spi::synth::{self, data_response};
use uiharu_protocols::sdcard_spi::{
    ResponseFormat, SdCardSpi, SdCardSpiConfig, SdEnd, SdEvent, Token,
};
use uiharu_protocols::spi::synth::SpiWaveform;
use uiharu_protocols::spi::{BitOrder, Spi, SpiConfig};

const CHANNELS: u8 = 4;
const RATE: u64 = 1_000_000;
const HALF: u64 = 2;
/// Samples per word: eight bits of two half periods.
const WORD: u64 = 8 * 2 * HALF;
/// Where chip select asserts in [`waveform`].
const SELECT: u64 = 10;

const R1: ResponseFormat = ResponseFormat::R1;
const R1B: ResponseFormat = ResponseFormat::R1b;
const R2: ResponseFormat = ResponseFormat::R2;
const R3: ResponseFormat = ResponseFormat::R3;
const R7: ResponseFormat = ResponseFormat::R7;

fn build(
    spi: SpiConfig,
    sd: SdCardSpiConfig,
    graph: GraphConfig,
) -> Result<Harness<SdEvent>, GraphError> {
    Harness::build(CHANNELS, TimeBase::new(RATE), graph, |graph, source| {
        let spi = graph.add(Spi::new(spi).expect("a valid configuration"));
        graph.connect(source.samples, Spi::input(spi))?;
        let sd = graph.add(SdCardSpi::new(sd).expect("a valid configuration"));
        graph.connect(Spi::events(spi), SdCardSpi::input(sd))?;
        Ok(SdCardSpi::events(sd))
    })
}

fn harness(sd: SdCardSpiConfig) -> Harness<SdEvent> {
    build(SpiConfig::default(), sd, GraphConfig::default()).expect("builds")
}

fn blocks_of(block_length: u16) -> SdCardSpiConfig {
    SdCardSpiConfig { block_length }
}

fn waveform() -> SpiWaveform {
    let mut waveform = SpiWaveform::new(SpiConfig::default(), CHANNELS, HALF, Tick::ZERO);
    waveform.idle(SELECT);
    waveform
}

/// The span and settled tick of words `from..to` of a transaction that began at [`SELECT`].
fn words(from: u64, to: u64) -> String {
    let first_bit = SELECT + 2 * HALF;
    let start = first_bit + from * WORD;
    let last = first_bit + (to - 1) * WORD + 7 * 2 * HALF;
    format!("[{start},{})@{last}", last + 1)
}

/// The last bit of word `n` of a transaction that began at [`SELECT`].
fn last_bit(n: u64) -> u64 {
    SELECT + 2 * HALF + n * WORD + 7 * 2 * HALF
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
fn describe(items: &[Item<SdEvent>], ticks: bool) -> Vec<String> {
    testing::without_progress(items)
        .iter()
        .map(|item| match item {
            Item::Event(event) => {
                let what = match &event.payload {
                    SdEvent::Command {
                        op,
                        tx,
                        index,
                        acmd,
                        argument,
                        crc7,
                        crc_ok,
                    } => {
                        let name = if *acmd { "ACMD" } else { "CMD" };
                        let crc = if *crc_ok {
                            "crc ok".to_owned()
                        } else {
                            format!("crc {crc7:02x} bad")
                        };
                        format!(
                            "command op{} tx{} {name}{index} {argument:08x} {crc}",
                            op.0, tx.0
                        )
                    }
                    SdEvent::Response {
                        op,
                        format,
                        bytes,
                        busy_bytes,
                    } => {
                        let busy = if *format == ResponseFormat::R1b {
                            format!(" busy {busy_bytes}")
                        } else {
                            String::new()
                        };
                        format!("response op{} {format:?} {}{busy}", op.0, hex(bytes))
                    }
                    SdEvent::Token { op, token, value } => {
                        format!("token op{} {token:?} {value:02x}", op.0)
                    }
                    SdEvent::Data { op, offset, bytes } => {
                        format!("data op{} +{offset} {}", op.0, hex(bytes))
                    }
                    SdEvent::BlockEnd {
                        op,
                        crc16,
                        crc_ok,
                        length,
                    } => {
                        let crc = if *crc_ok { "ok" } else { "bad" };
                        format!("block-end op{} {length} crc {crc16:04x} {crc}", op.0)
                    }
                    SdEvent::End { op, reason } => format!("end op{} {reason:?}", op.0),
                    SdEvent::UnknownCommand { tx, byte } => {
                        format!("unknown tx{} {byte:02x}", tx.0)
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

/// The events of `harness` as lines without ticks, the end of the stream left out.
fn lines(harness: &Harness<SdEvent>) -> Vec<String> {
    describe(&harness.items, false)
        .into_iter()
        .filter(|line| !line.starts_with("eof "))
        .collect()
}

/// Decodes `transactions`, each in its own chip select period.
fn run(config: SdCardSpiConfig, transactions: &[Vec<(u8, u8)>]) -> Harness<SdEvent> {
    let mut waveform = waveform();
    for words in transactions {
        waveform.transaction(words.iter().copied());
        waveform.idle(10);
    }
    let block = waveform.finish();
    let mut harness = harness(config);
    harness.run_all(&block);
    harness
}

fn decode(config: SdCardSpiConfig, transactions: &[Vec<(u8, u8)>]) -> Vec<String> {
    lines(&run(config, transactions))
}

/// A command and the card's answer, as one transaction's words.
fn exchange(index: u8, argument: u32, answer: Vec<(u8, u8)>) -> Vec<(u8, u8)> {
    let mut words = synth::command(index, argument);
    words.extend(answer);
    words
}

/// The bytes the data events of `harness` carry, in order.
fn data_bytes(harness: &Harness<SdEvent>) -> Vec<u8> {
    harness
        .events()
        .into_iter()
        .filter_map(|event| match event.payload {
            SdEvent::Data { bytes, .. } => Some(bytes),
            _ => None,
        })
        .flatten()
        .collect()
}

/// `len` bytes of a pattern that differs with `seed`.
fn pattern(len: usize, seed: u8) -> Vec<u8> {
    (0..len)
        .map(|i| (i as u8).wrapping_mul(7).wrapping_add(seed))
        .collect()
}

#[test]
fn the_initialisation_sequence_decodes() {
    let lines = decode(
        SdCardSpiConfig::default(),
        &[
            exchange(0, 0, synth::response(R1, &[0x01], 0)),
            exchange(8, 0x1aa, synth::response(R7, &[0x01, 0, 0, 0x01, 0xaa], 0)),
            exchange(55, 0, synth::response(R1, &[0x01], 0)),
            exchange(41, 0x4000_0000, synth::response(R1, &[0x01], 0)),
            exchange(55, 0, synth::response(R1, &[0x01], 0)),
            exchange(41, 0x4000_0000, synth::response(R1, &[0x00], 0)),
            exchange(
                58,
                0,
                synth::response(R3, &[0x00, 0xc0, 0xff, 0x80, 0x00], 0),
            ),
        ],
    );
    assert_eq!(
        lines,
        [
            "command op1 tx1 CMD0 00000000 crc ok",
            "response op1 R1 01",
            "end op1 Complete",
            "command op2 tx2 CMD8 000001aa crc ok",
            "response op2 R7 01000001aa",
            "end op2 Complete",
            "command op3 tx3 CMD55 00000000 crc ok",
            "response op3 R1 01",
            "end op3 Complete",
            "command op4 tx4 ACMD41 40000000 crc ok",
            "response op4 R1 01",
            "end op4 Complete",
            "command op5 tx5 CMD55 00000000 crc ok",
            "response op5 R1 01",
            "end op5 Complete",
            "command op6 tx6 ACMD41 40000000 crc ok",
            "response op6 R1 00",
            "end op6 Complete",
            "command op7 tx7 CMD58 00000000 crc ok",
            "response op7 R3 00c0ff8000",
            "end op7 Complete",
        ]
    );
}

#[test]
fn cmd0_and_cmd8_carry_their_published_crcs() {
    let harness = run(
        SdCardSpiConfig::default(),
        &[
            exchange(0, 0, synth::response(R1, &[0x01], 0)),
            exchange(8, 0x1aa, synth::response(R7, &[0x01, 0, 0, 0x01, 0xaa], 0)),
        ],
    );
    let crcs: Vec<(u8, bool)> = harness
        .events()
        .into_iter()
        .filter_map(|event| match event.payload {
            SdEvent::Command { crc7, crc_ok, .. } => Some((crc7, crc_ok)),
            _ => None,
        })
        .collect();
    // On the bus as 0x95 and 0x87: the CRC7 and the end bit.
    assert_eq!(crcs, [(0x4a, true), (0x43, true)]);
    assert_eq!(synth::frame(0, 0)[5], 0x95);
    assert_eq!(synth::frame(8, 0x1aa)[5], 0x87);
    // A host that sends no CRC (a common 0xFF) is reported as such.
    let mut frame = synth::command(17, 0);
    frame[5] = (0xff, 0xff);
    frame.extend(synth::response(R1, &[0x04], 0));
    let lines = decode(SdCardSpiConfig::default(), &[frame]);
    assert_eq!(lines[0], "command op1 tx1 CMD17 00000000 crc 7f bad");
    assert_eq!(lines[1], "response op1 R1 04", "illegal command: no data");
    assert_eq!(lines[2], "end op1 Complete");
}

#[test]
fn events_cover_their_words() {
    let harness = run(
        SdCardSpiConfig::default(),
        &[exchange(0, 0, synth::response(R1, &[0x01], 0))],
    );
    // The frame is six words; one byte of fill comes before the response, which is not
    // reported; the operation ends with its response.
    assert_eq!(
        describe(&harness.items, true)[..3],
        [
            format!("command op1 tx1 CMD0 00000000 crc ok {}", words(0, 6)),
            format!("response op1 R1 01 {}", words(7, 8)),
            format!("end op1 Complete @{}", last_bit(7)),
        ]
    );
}

#[test]
fn a_host_that_keeps_chip_select_asserted_sends_several_commands() {
    let mut words = exchange(0, 0, synth::response(R1, &[0x01], 0));
    words.extend(synth::fill(2));
    words.extend(exchange(
        8,
        0x1aa,
        synth::response_after(3, R7, &[0x01, 0, 0, 0x01, 0xaa], 0),
    ));
    words.extend(exchange(55, 0, synth::response_after(0, R1, &[0x01], 0)));
    words.extend(exchange(
        41,
        0x4000_0000,
        synth::response_after(8, R1, &[0x00], 0),
    ));
    let lines = decode(SdCardSpiConfig::default(), &[words]);
    assert_eq!(
        lines,
        [
            "command op1 tx1 CMD0 00000000 crc ok",
            "response op1 R1 01",
            "end op1 Complete",
            "command op2 tx1 CMD8 000001aa crc ok",
            "response op2 R7 01000001aa",
            "end op2 Complete",
            "command op3 tx1 CMD55 00000000 crc ok",
            "response op3 R1 01",
            "end op3 Complete",
            "command op4 tx1 ACMD41 40000000 crc ok",
            "response op4 R1 00",
            "end op4 Complete",
        ]
    );
}

#[test]
fn a_block_read_comes_in_batches_with_its_crc() {
    let data = pattern(512, 3);
    let read = |crc| {
        let mut words = synth::command(17, 0x0000_1000);
        words.extend(synth::response(R1, &[0x00], 0));
        words.extend(synth::fill(3));
        words.extend(synth::read_block_with_crc(&data, crc));
        words
    };
    let good = synth::crc16(&data);
    let harness = run(
        SdCardSpiConfig::default(),
        &[read(good), read(good ^ 0x0100)],
    );
    let lines = lines(&harness);
    assert_eq!(lines[0], "command op1 tx1 CMD17 00001000 crc ok");
    assert_eq!(lines[1], "response op1 R1 00");
    assert_eq!(lines[2], "token op1 StartBlock fe");
    for batch in 0..16 {
        let offset = batch * 32;
        assert_eq!(
            lines[3 + batch],
            format!("data op1 +{offset} {}", hex(&data[offset..offset + 32]))
        );
    }
    assert_eq!(lines[19], format!("block-end op1 512 crc {good:04x} ok"));
    assert_eq!(lines[20], "end op1 Complete");
    assert_eq!(
        lines[40],
        format!("block-end op2 512 crc {:04x} bad", good ^ 0x0100)
    );
    assert_eq!(lines[41], "end op2 Complete");
    assert_eq!(lines.len(), 42);
    assert_eq!(data_bytes(&harness), [data.clone(), data].concat());

    // A batch goes out with its last byte; the CRC covers its two words.
    let ticks = describe(&harness.items, true);
    // Words: frame 0..6, fill 6, R1 7, fill 8..11, fill 11, token 12, data 13..525, CRC 525..527.
    assert_eq!(
        ticks[2],
        format!("token op1 StartBlock fe {}", words(12, 13))
    );
    assert!(ticks[3].ends_with(&words(13, 45)), "{}", ticks[3]);
    assert!(ticks[19].ends_with(&words(525, 527)), "{}", ticks[19]);
    assert!(
        ticks[20].ends_with(&format!(" @{}", last_bit(526))),
        "{}",
        ticks[20]
    );
}

#[test]
fn a_block_write_is_answered_by_a_data_response() {
    let data = pattern(512, 9);
    let mut words = synth::command(24, 0x0000_2000);
    words.extend(synth::response(R1, &[0x00], 0));
    words.extend(synth::write_block(0xfe, &data, data_response(0b010), 6));
    // The busy period after the data response belongs to no operation.
    words.extend(exchange(13, 0, synth::response(R2, &[0x00, 0x00], 0)));
    let mut rejected = synth::command(24, 0x0000_2200);
    rejected.extend(synth::response(R1, &[0x00], 0));
    rejected.extend(synth::write_block(
        0xfe,
        &data[..512],
        data_response(0b101),
        0,
    ));
    let harness = run(SdCardSpiConfig::default(), &[words, rejected]);
    let lines = lines(&harness);
    assert_eq!(lines[0], "command op1 tx1 CMD24 00002000 crc ok");
    assert_eq!(lines[1], "response op1 R1 00");
    assert_eq!(lines[2], "token op1 StartBlock fe");
    // Sixteen batches of data, 3 to 18.
    assert_eq!(
        lines[19],
        format!("block-end op1 512 crc {:04x} ok", synth::crc16(&data))
    );
    assert_eq!(lines[20], "token op1 DataResponse e5");
    assert_eq!(lines[21], "end op1 Complete");
    assert_eq!(lines[22], "command op2 tx1 CMD13 00000000 crc ok");
    assert_eq!(lines[23], "response op2 R2 0000");
    assert_eq!(lines[24], "end op2 Complete");
    assert_eq!(lines[45], "token op3 DataResponse eb", "a CRC error");
    assert_eq!(lines[46], "end op3 Complete");
    assert_eq!(lines.len(), 47);
    assert_eq!(&data_bytes(&harness)[..512], &data[..]);
}

#[test]
fn a_multiple_block_read_runs_until_cmd12() {
    let first = pattern(8, 1);
    let second = pattern(8, 2);
    let mut words = synth::command(18, 0x0000_0040);
    words.extend(synth::response(R1, &[0x00], 0));
    words.extend(synth::read_block(&first));
    words.extend(synth::read_block(&second));
    // The host stops the read while the card begins the next block: the card goes on
    // sending during the command, and the stuff byte after it still carries data bits.
    let stop = synth::frame(12, 0);
    let card = [0xff, 0xfe, 0x12, 0x34, 0x56, 0x78];
    words.extend(stop.iter().zip(card).map(|(host, card)| (*host, card)));
    words.push((0xff, 0x1f));
    words.extend(synth::response_after(1, R1B, &[0x00], 2));
    let lines = decode(blocks_of(8), &[words]);
    assert_eq!(
        lines,
        [
            "command op1 tx1 CMD18 00000040 crc ok".to_owned(),
            "response op1 R1 00".to_owned(),
            "token op1 StartBlock fe".to_owned(),
            format!("data op1 +0 {}", hex(&first)),
            format!("block-end op1 8 crc {:04x} ok", synth::crc16(&first)),
            "token op1 StartBlock fe".to_owned(),
            format!("data op1 +8 {}", hex(&second)),
            format!("block-end op1 8 crc {:04x} ok", synth::crc16(&second)),
            "end op1 Complete".to_owned(),
            "command op2 tx1 CMD12 00000000 crc ok".to_owned(),
            "response op2 R1b 00 busy 2".to_owned(),
            "end op2 Complete".to_owned(),
        ]
    );
}

#[test]
fn cmd12_inside_a_block_stops_the_read() {
    let mut words = synth::command(18, 0);
    words.extend(synth::response(R1, &[0x00], 0));
    words.extend(synth::read_block(&pattern(16, 4)));
    words.extend([(0xff, 0xfe), (0xff, 0xa0), (0xff, 0xa1)]);
    words.extend(synth::command(12, 0));
    words.push((0xff, 0xff));
    words.extend(synth::response_after(0, R1B, &[0x00], 0));
    let lines = decode(blocks_of(16), &[words]);
    assert_eq!(
        lines[5..],
        [
            "token op1 StartBlock fe",
            "data op1 +16 a0a1",
            "end op1 Complete",
            "command op2 tx1 CMD12 00000000 crc ok",
            "response op2 R1b 00 busy 0",
            "end op2 Complete",
        ]
    );
}

#[test]
fn r2_and_r1b_responses() {
    let mut cut = synth::command(38, 0);
    cut.extend(synth::response(R1B, &[0x00], 0));
    // Chip select released while the card is still busy: the specification allows it, and the
    // response is whole.
    cut.truncate(cut.len() - 1);
    cut.extend([(0xff, 0x00), (0xff, 0x00), (0xff, 0x00)]);
    let lines = decode(
        SdCardSpiConfig::default(),
        &[
            exchange(13, 0, synth::response(R2, &[0x00, 0x00], 0)),
            exchange(13, 0, synth::response(R2, &[0x20, 0x01], 0)),
            exchange(38, 0, synth::response(R1B, &[0x00], 5)),
            exchange(28, 0x0001_0000, synth::response(R1B, &[0x40], 0)),
            cut,
        ],
    );
    assert_eq!(
        lines,
        [
            "command op1 tx1 CMD13 00000000 crc ok",
            "response op1 R2 0000",
            "end op1 Complete",
            "command op2 tx2 CMD13 00000000 crc ok",
            "response op2 R2 2001",
            "end op2 Complete",
            "command op3 tx3 CMD38 00000000 crc ok",
            "response op3 R1b 00 busy 5",
            "end op3 Complete",
            "command op4 tx4 CMD28 00010000 crc ok",
            "response op4 R1b 40 busy 0",
            "end op4 Complete",
            "command op5 tx5 CMD38 00000000 crc ok",
            "response op5 R1b 00 busy 3",
            "end op5 Complete",
        ]
    );
    // The R1b response covers R1 and the busy bytes and settles when the card is ready.
    let harness = run(
        SdCardSpiConfig::default(),
        &[exchange(38, 0, synth::response(R1B, &[0x00], 5))],
    );
    let ticks = describe(&harness.items, true);
    // Words: frame 0..6, fill 6, R1 7, busy 8..13, ready 13.
    let span = words(7, 13);
    let span = &span[..span.find('@').expect("a settled tick")];
    assert_eq!(
        ticks[1],
        format!("response op1 R1b 00 busy 5 {span}@{}", last_bit(13))
    );
    assert_eq!(ticks[2], format!("end op1 Complete @{}", last_bit(13)));
}

#[test]
fn an_error_token_replaces_the_block() {
    let mut words = synth::command(17, 0x7fff_fe00);
    words.extend(synth::response(R1, &[0x00], 0));
    words.extend(synth::error_token(0x08));
    let lines = decode(SdCardSpiConfig::default(), &[words]);
    assert_eq!(
        lines,
        [
            "command op1 tx1 CMD17 7ffffe00 crc ok",
            "response op1 R1 00",
            "token op1 Error 08",
            "end op1 Complete",
        ]
    );
}

#[test]
fn cmd16_sets_the_block_length() {
    let read = |data: &[u8]| {
        let mut words = synth::command(17, 0);
        words.extend(synth::response(R1, &[0x00], 0));
        words.extend(synth::read_block(data));
        words
    };
    let lines = decode(
        SdCardSpiConfig::default(),
        &[
            exchange(16, 4, synth::response(R1, &[0x00], 0)),
            read(&[1, 2, 3, 4]),
            // Refused with a parameter error: the length stays.
            exchange(16, 4096, synth::response(R1, &[0x40], 0)),
            read(&[5, 6, 7, 8]),
            // A reset returns to 512 bytes.
            exchange(0, 0, synth::response(R1, &[0x01], 0)),
            read(&pattern(512, 0)),
        ],
    );
    assert_eq!(lines[6], "data op2 +0 01020304");
    assert!(lines[7].starts_with("block-end op2 4 crc "), "{lines:?}");
    assert_eq!(lines[15], "data op4 +0 05060708");
    assert!(lines[16].starts_with("block-end op4 4 crc "), "{lines:?}");
    assert!(
        lines
            .iter()
            .any(|line| line.starts_with("block-end op6 512 crc ") && line.ends_with(" ok")),
        "{lines:?}"
    );
}

#[test]
fn registers_have_their_lengths() {
    let read = |index: u8, app: bool, data: &[u8]| {
        let mut words = Vec::new();
        if app {
            words.extend(exchange(55, 0, synth::response(R1, &[0x00], 0)));
        }
        words.extend(synth::command(index, 0));
        words.extend(synth::response(R1, &[0x00], 0));
        words.extend(synth::read_block(data));
        words
    };
    let csd = pattern(16, 0x40);
    let cid = pattern(16, 0x03);
    let scr = pattern(8, 0x02);
    let status = pattern(4, 0);
    let lines = decode(
        SdCardSpiConfig::default(),
        &[
            read(9, false, &csd),
            read(10, false, &cid),
            read(51, true, &scr),
            read(30, false, &status),
        ],
    );
    assert_eq!(lines[3], format!("data op1 +0 {}", hex(&csd)));
    assert!(lines[4].starts_with("block-end op1 16 "), "{lines:?}");
    assert_eq!(lines[9], format!("data op2 +0 {}", hex(&cid)));
    // CMD55 is operation 3, in the same transaction as ACMD51.
    assert_eq!(lines[15], "command op4 tx3 ACMD51 00000000 crc ok");
    assert_eq!(lines[18], format!("data op4 +0 {}", hex(&scr)));
    assert!(lines[19].starts_with("block-end op4 8 "), "{lines:?}");
    assert_eq!(lines[24], format!("data op5 +0 {}", hex(&status)));
    assert!(lines[25].starts_with("block-end op5 4 "), "{lines:?}");
}

#[test]
fn a_multiple_block_write_ends_with_the_stop_token() {
    let first = pattern(8, 5);
    let second = pattern(8, 6);
    let mut words = synth::command(25, 0x100);
    words.extend(synth::response(R1, &[0x00], 0));
    words.extend(synth::write_block(0xfc, &first, data_response(0b010), 3));
    words.extend(synth::write_block(0xfc, &second, data_response(0b010), 1));
    words.extend(synth::stop_tran(4));
    let lines = decode(blocks_of(8), &[words]);
    assert_eq!(
        lines,
        [
            "command op1 tx1 CMD25 00000100 crc ok".to_owned(),
            "response op1 R1 00".to_owned(),
            "token op1 StartMultiBlock fc".to_owned(),
            format!("data op1 +0 {}", hex(&first)),
            format!("block-end op1 8 crc {:04x} ok", synth::crc16(&first)),
            "token op1 DataResponse e5".to_owned(),
            "token op1 StartMultiBlock fc".to_owned(),
            format!("data op1 +8 {}", hex(&second)),
            format!("block-end op1 8 crc {:04x} ok", synth::crc16(&second)),
            "token op1 DataResponse e5".to_owned(),
            "token op1 StopTran fd".to_owned(),
            "end op1 Complete".to_owned(),
        ]
    );
}

#[test]
fn application_commands_follow_an_accepted_cmd55() {
    let mut sd_status = exchange(55, 0, synth::response(R1, &[0x00], 0));
    sd_status.extend(synth::command(13, 0));
    sd_status.extend(synth::response(R2, &[0x00, 0x00], 0));
    sd_status.extend(synth::read_block(&pattern(64, 1)));
    let lines = decode(
        SdCardSpiConfig::default(),
        &[
            // Refused: the next command is a regular one.
            exchange(55, 0, synth::response(R1, &[0x05], 0)),
            exchange(41, 0, synth::response(R1, &[0x05], 0)),
            // CMD17 has no application meaning: the card takes it as itself.
            exchange(55, 0, synth::response(R1, &[0x00], 0)),
            {
                let mut words = synth::command(17, 0);
                words.extend(synth::response(R1, &[0x00], 0));
                words.extend(synth::read_block(&[0xaa; 512]));
                words
            },
            sd_status,
        ],
    );
    assert_eq!(lines[3], "command op2 tx2 CMD41 00000000 crc ok");
    assert_eq!(lines[9], "command op4 tx4 CMD17 00000000 crc ok");
    assert!(
        lines.contains(&"command op6 tx5 ACMD13 00000000 crc ok".to_owned()),
        "{lines:?}"
    );
    assert!(
        lines.contains(&"response op6 R2 0000".to_owned()),
        "{lines:?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.starts_with("block-end op6 64 crc ") && line.ends_with(" ok")),
        "{lines:?}"
    );
}

#[test]
fn a_refused_command_is_answered_by_r1_alone() {
    // A version 1 card does not know CMD8 and sends R1 alone; the host goes on at once.
    let mut words = exchange(8, 0x1aa, synth::response(R1, &[0x05], 0));
    words.extend(exchange(55, 0, synth::response_after(0, R1, &[0x01], 0)));
    let lines = decode(SdCardSpiConfig::default(), &[words]);
    assert_eq!(
        lines,
        [
            "command op1 tx1 CMD8 000001aa crc ok",
            "response op1 R1 05",
            "end op1 Complete",
            "command op2 tx1 CMD55 00000000 crc ok",
            "response op2 R1 01",
            "end op2 Complete",
        ]
    );
}

#[test]
fn operations_cut_short_end_incomplete() {
    let mut no_response = synth::command(0, 0);
    no_response.extend(synth::fill(2));
    let mut mid_block = synth::command(17, 0);
    mid_block.extend(synth::response(R1, &[0x00], 0));
    mid_block.extend(synth::read_block(&pattern(512, 7)).into_iter().take(12));
    let mut no_token = synth::command(24, 0);
    no_token.extend(synth::response(R1, &[0x00], 0));
    no_token.extend(synth::fill(4));
    let mut short_r7 = synth::command(8, 0x1aa);
    short_r7.extend(synth::response(R7, &[0x01, 0x00], 0));
    // The host gives up waiting and sends the next command.
    let mut given_up = synth::command(58, 0);
    given_up.extend(synth::fill(9));
    given_up.extend(exchange(0, 0, synth::response(R1, &[0x01], 0)));
    let lines = decode(
        SdCardSpiConfig::default(),
        &[
            no_response,
            mid_block,
            no_token,
            short_r7,
            given_up,
            synth::command(17, 1)[..3].to_vec(),
        ],
    );
    assert_eq!(
        lines,
        [
            "command op1 tx1 CMD0 00000000 crc ok",
            "end op1 Incomplete",
            "command op2 tx2 CMD17 00000000 crc ok",
            "response op2 R1 00",
            "token op2 StartBlock fe",
            "data op2 +0 070e151c232a31383f46",
            "end op2 Incomplete",
            "command op3 tx3 CMD24 00000000 crc ok",
            "response op3 R1 00",
            "end op3 Incomplete",
            "command op4 tx4 CMD8 000001aa crc ok",
            "end op4 Incomplete",
            "command op5 tx5 CMD58 00000000 crc ok",
            "end op5 Incomplete",
            "command op6 tx5 CMD0 00000000 crc ok",
            "response op6 R1 01",
            "end op6 Complete",
        ]
    );
}

#[test]
fn an_incomplete_end_covers_what_was_read_toward_it() {
    let mut words = synth::command(8, 0x1aa);
    words.extend(synth::response(R7, &[0x01, 0x00], 0));
    let harness = run(SdCardSpiConfig::default(), &[words]);
    let ticks = describe(&harness.items, true);
    // Words: frame 0..6, fill 6, R1 7, one byte of the four that should follow, 8; chip
    // select is released half a period after it.
    let release = SELECT + HALF + 9 * WORD + HALF;
    let start = SELECT + 2 * HALF + 7 * WORD;
    assert_eq!(
        ticks[1],
        format!("end op1 Incomplete [{start},{})@{release}", release + 1)
    );
}

#[test]
fn a_gap_aborts_the_operation_and_decoding_resumes() {
    let data = pattern(512, 1);
    let mut waveform = waveform();
    let mut read = synth::command(17, 0);
    read.extend(synth::response(R1, &[0x00], 0));
    read.extend(synth::read_block(&data));
    waveform.transaction(read);
    waveform.idle(10);
    waveform.transaction(exchange(13, 0, synth::response(R2, &[0x00, 0x00], 0)));
    waveform.idle(10);
    let block = waveform.finish();
    // Words: frame 0..6, fill 6, R1 7, fill 8, token 9, data from 10. Lose a stretch from
    // inside word 53, data byte 43, on: 43 bytes are whole.
    let gap_start = SELECT + 2 * HALF + 53 * WORD + 5;
    let gap = TickRange::new(Tick(gap_start), Tick(gap_start + 3 * WORD));
    let mut harness = harness(SdCardSpiConfig::default());
    harness.feed_with_gaps(&block, &[gap], &[100]);
    harness.end();
    let lines = describe(&harness.items, true);
    assert!(lines[3].starts_with(&format!("data op1 +0 {} [", hex(&data[..32]))));
    assert!(
        lines[4].starts_with(&format!("data op1 +32 {} [", hex(&data[32..43]))),
        "{lines:?}"
    );
    assert!(lines[4].ends_with(&format!(")@{gap_start}")), "{lines:?}");
    assert_eq!(lines[5], format!("end op1 Aborted @{gap_start}"));
    assert_eq!(lines[6], format!("gap {gap}"));
    // The rest of the read's transaction is skipped; the next one is read whole.
    assert!(
        lines[7].starts_with("command op2 tx3 CMD13 00000000 crc ok "),
        "{lines:?}"
    );
    assert!(lines[8].starts_with("response op2 R2 0000 "), "{lines:?}");
    assert!(lines[9].starts_with("end op2 Complete "), "{lines:?}");
    assert_eq!(lines.len(), 11, "{lines:?}");
}

#[test]
fn the_end_of_the_samples_aborts_an_open_operation() {
    let mut waveform = waveform();
    waveform.select();
    waveform.words(exchange(38, 0, synth::response(R1B, &[0x00], 40)));
    let block = waveform.finish();
    let cut = Tick(SELECT + 2 * HALF + 20 * WORD + 3);
    let head = block
        .slice(TickRange::new(Tick::ZERO, cut))
        .expect("a slice");
    let mut harness = harness(SdCardSpiConfig::default());
    harness.run_all(&head);
    let lines = describe(&harness.items, true);
    // R1 at word 7, busy from word 8: 12 busy bytes are whole when the samples end.
    assert!(
        lines[1].starts_with("response op1 R1b 00 busy 12 ["),
        "{lines:?}"
    );
    assert!(lines[1].ends_with(&format!(")@{cut}")), "{lines:?}");
    assert_eq!(lines[2], format!("end op1 Aborted @{cut}"));
    assert_eq!(lines[3], format!("eof {cut}"));
}

#[test]
fn a_gap_forgets_cmd55_but_not_the_block_length() {
    let mut waveform = waveform();
    waveform.transaction(exchange(16, 8, synth::response(R1, &[0x00], 0)));
    waveform.idle(10);
    waveform.transaction(exchange(55, 0, synth::response(R1, &[0x00], 0)));
    waveform.idle(10);
    let gap_start = waveform.position();
    waveform.idle(30);
    let gap = TickRange::new(gap_start, waveform.position());
    // Chip select asserted at the first sample after the gap would make a partial transaction.
    waveform.idle(10);
    waveform.transaction(exchange(41, 0, synth::response(R1, &[0x00], 0)));
    waveform.idle(10);
    let mut read = synth::command(17, 0);
    read.extend(synth::response(R1, &[0x00], 0));
    read.extend(synth::read_block(&pattern(8, 0)));
    waveform.transaction(read);
    waveform.idle(10);
    let block = waveform.finish();
    let mut harness = harness(SdCardSpiConfig::default());
    harness.feed_with_gaps(&block, &[gap], &[64]);
    harness.end();
    let lines = lines(&harness);
    assert_eq!(lines[6], format!("gap {gap}"));
    // A CMD55 the gap separates from the next command is forgotten.
    assert_eq!(lines[7], "command op3 tx3 CMD41 00000000 crc ok");
    // The card keeps the block length CMD16 set.
    assert_eq!(lines[13], format!("data op4 +0 {}", hex(&pattern(8, 0))));
    assert!(lines[14].starts_with("block-end op4 8 crc "), "{lines:?}");
}

#[test]
fn transactions_without_a_command() {
    // The host clocks on after a multiple-block read in a new chip select period.
    let mut continued = synth::fill(2);
    continued.extend(synth::read_block(&pattern(512, 3)));
    let mut data_token = vec![(0xfe, 0xff)];
    data_token.extend(pattern(16, 0).into_iter().map(|byte| (byte, 0xff)));
    let lines = decode(
        SdCardSpiConfig::default(),
        &[
            continued,
            data_token,
            vec![(0xff, 0xff), (0x00, 0xff), (0x40, 0xff)],
            synth::fill(3),
            exchange(13, 0, synth::response(R2, &[0x00, 0x00], 0)),
        ],
    );
    assert_eq!(
        lines,
        [
            "unknown tx2 fe",
            "unknown tx3 00",
            "command op1 tx5 CMD13 00000000 crc ok",
            "response op1 R2 0000",
            "end op1 Complete",
        ]
    );
}

#[test]
fn a_transaction_under_way_at_the_start_is_skipped() {
    let mut waveform = waveform();
    waveform.transaction(exchange(13, 0, synth::response(R2, &[0x00, 0x00], 0)));
    waveform.idle(10);
    waveform.transaction(exchange(13, 0, synth::response(R2, &[0x00, 0x01], 0)));
    waveform.idle(10);
    let block = waveform.finish();
    // Start inside the first transaction, at a word boundary.
    let from = Tick(SELECT + HALF + 2 * WORD);
    let tail = block
        .slice(TickRange::new(from, block.end()))
        .expect("a slice");
    let mut harness = harness(SdCardSpiConfig::default());
    harness.run_all(&tail);
    assert_eq!(
        lines(&harness),
        [
            "command op1 tx2 CMD13 00000000 crc ok",
            "response op1 R2 0001",
            "end op1 Complete",
        ]
    );
}

/// Initialisation, register reads, single and multiple reads and writes, errors and a
/// transaction cut short, with a host that sometimes keeps chip select asserted. Returns where
/// each transaction begins.
fn rich_traffic(waveform: &mut SpiWaveform) -> Vec<Tick> {
    let mut init = exchange(0, 0, synth::response(R1, &[0x01], 0));
    init.extend(exchange(
        8,
        0x1aa,
        synth::response_after(2, R7, &[0x01, 0, 0, 0x01, 0xaa], 0),
    ));
    let mut multi_read = synth::command(18, 0x20);
    multi_read.extend(synth::response(R1, &[0x00], 0));
    multi_read.extend(synth::read_block(&pattern(64, 1)));
    multi_read.extend(synth::fill(3));
    multi_read.extend(synth::read_block(&pattern(64, 2)));
    multi_read.extend(synth::command(12, 0));
    multi_read.push((0xff, 0x05));
    multi_read.extend(synth::response_after(1, R1B, &[0x00], 4));
    let mut multi_write = synth::command(25, 0x40);
    multi_write.extend(synth::response(R1, &[0x00], 0));
    multi_write.extend(synth::write_block(
        0xfc,
        &pattern(64, 3),
        data_response(0b010),
        5,
    ));
    multi_write.extend(synth::write_block(
        0xfc,
        &pattern(64, 4),
        data_response(0b110),
        2,
    ));
    multi_write.extend(synth::stop_tran(6));
    let mut csd = synth::command(9, 0);
    csd.extend(synth::response(R1, &[0x00], 0));
    csd.extend(synth::read_block(&pattern(16, 9)));
    let mut error = synth::command(17, 0xffff_ff00);
    error.extend(synth::response(R1, &[0x00], 0));
    error.extend(synth::error_token(0x08));
    let mut cut = synth::command(17, 0x80);
    cut.extend(synth::response(R1, &[0x00], 0));
    cut.extend(synth::read_block(&pattern(64, 5)).into_iter().take(30));
    let transactions = [
        init,
        exchange(55, 0, synth::response(R1, &[0x01], 0)),
        exchange(41, 0x4000_0000, synth::response(R1, &[0x00], 0)),
        exchange(
            58,
            0,
            synth::response(R3, &[0x00, 0xc0, 0xff, 0x80, 0x00], 0),
        ),
        exchange(16, 64, synth::response(R1, &[0x00], 0)),
        csd,
        multi_read,
        exchange(13, 0, synth::response(R2, &[0x00, 0x00], 0)),
        multi_write,
        error,
        cut,
        exchange(38, 0, synth::response(R1B, &[0x00], 9)),
        vec![(0x00, 0xff), (0x40, 0xff)],
    ];
    let mut starts = Vec::new();
    for (n, words) in transactions.iter().enumerate() {
        starts.push(waveform.position());
        waveform.transaction(words.iter().copied());
        waveform.idle(4 + (n as u64 % 3) * 9);
    }
    starts
}

#[test]
fn the_events_are_the_same_however_the_samples_are_cut() {
    let mut waveform = waveform();
    let starts = rich_traffic(&mut waveform);
    let block = waveform.finish();
    // One gap inside the first block of the multiple-block read (word 30 of its transaction),
    // one inside CMD38's busy period (word 12).
    let word = |transaction: usize, word: u64| starts[transaction].offset(2 * HALF + word * WORD);
    let gaps = [
        TickRange::new(word(6, 30).offset(3), word(6, 30).offset(48)),
        TickRange::new(word(11, 12).offset(1), word(11, 12).offset(8)),
    ];
    let build = |graph_config| {
        build(
            SpiConfig::default(),
            SdCardSpiConfig::default(),
            graph_config,
        )
        .expect("builds")
    };
    let mut reference = build(GraphConfig::default());
    reference.feed_with_gaps(&block, &gaps, &[u64::MAX]);
    reference.end();
    let events = reference.events();
    assert!(events.len() > 50, "{} events", events.len());
    let ended = |wanted: SdEnd| {
        events
            .iter()
            .any(|event| matches!(event.payload, SdEvent::End { reason, .. } if reason == wanted))
    };
    assert!(ended(SdEnd::Complete) && ended(SdEnd::Incomplete) && ended(SdEnd::Aborted));
    assert!(events.iter().any(|event| matches!(
        event.payload,
        SdEvent::Token {
            token: Token::StopTran,
            ..
        }
    )));
    testing::assert_block_invariant(build, &block, &gaps);
}

#[test]
fn the_bus_must_meet_the_decoders_requirements() {
    let refused = |spi: SpiConfig| {
        build(spi, SdCardSpiConfig::default(), GraphConfig::default()).expect_err("refused")
    };
    assert_eq!(
        refused(SpiConfig {
            word_bits: 7,
            ..SpiConfig::default()
        }),
        GraphError::Requirement {
            from: "spi#0.events".into(),
            to: "sdcard-spi#1.bus".into(),
            key: "wordBits",
            requirement: "= 8".into(),
            found: "7".into(),
        }
    );
    let key = |error: GraphError| match error {
        GraphError::Requirement { key, .. } => key,
        other => panic!("not a requirement: {other}"),
    };
    let lsb = SpiConfig {
        bit_order: BitOrder::LsbFirst,
        ..SpiConfig::default()
    };
    assert_eq!(key(refused(lsb)), "bitOrder");
    let unframed = SpiConfig {
        cs: None,
        ..SpiConfig::default()
    };
    assert_eq!(key(refused(unframed)), "cs");
    let no_miso = SpiConfig {
        miso: None,
        ..SpiConfig::default()
    };
    assert_eq!(key(refused(no_miso)), "miso");
    let no_mosi = SpiConfig {
        mosi: None,
        ..SpiConfig::default()
    };
    assert_eq!(key(refused(no_mosi)), "mosi");
    assert!(
        SdCardSpi::requires()
            .unmet(&SpiConfig::default().props())
            .is_none()
    );
}

/// One operation of a random scenario.
#[derive(Clone, Debug)]
enum Op {
    Command { index: u8, r1: u8, fill: usize },
    Register { which: u8, refused: bool },
    Erase { busy: u16 },
    Read { error: bool, bad_crc: bool },
    ReadMultiple { blocks: u8 },
    Write { status: u8, busy: u16 },
    WriteMultiple { blocks: u8 },
    Stray { byte: u8 },
}

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        3 => (
            prop::sample::select(vec![1u8, 23, 32, 33, 41, 55, 59]),
            prop::sample::select(vec![0x00u8, 0x01, 0x04, 0x40]),
            0usize..9,
        )
            .prop_map(|(index, r1, fill)| Op::Command { index, r1, fill }),
        2 => (0u8..3, any::<bool>()).prop_map(|(which, refused)| Op::Register { which, refused }),
        1 => (0u16..12).prop_map(|busy| Op::Erase { busy }),
        3 => (any::<bool>(), any::<bool>()).prop_map(|(error, bad_crc)| Op::Read { error, bad_crc }),
        2 => (1u8..4).prop_map(|blocks| Op::ReadMultiple { blocks }),
        2 => (prop::sample::select(vec![0b010u8, 0b101, 0b110]), 0u16..6)
            .prop_map(|(status, busy)| Op::Write { status, busy }),
        2 => (1u8..4).prop_map(|blocks| Op::WriteMultiple { blocks }),
        1 => any::<u8>().prop_map(|byte| Op::Stray { byte }),
    ]
}

/// The words of `op`, the `n`th of its scenario, with blocks of 16 bytes.
fn op_words(op: &Op, n: usize) -> Vec<(u8, u8)> {
    let seed = n as u8;
    match op {
        Op::Command { index, r1, fill } => exchange(
            *index,
            n as u32,
            synth::response_after(*fill, R1, &[*r1], 0),
        ),
        Op::Register { which, refused } => {
            let (index, format, bytes) = match which {
                0 => (13, R2, vec![0x00, 0x00]),
                1 => (58, R3, vec![0x00, 0xc0, 0xff, 0x80, 0x00]),
                _ => (8, R7, vec![0x01, 0x00, 0x00, 0x01, 0xaa]),
            };
            let answer = if *refused {
                synth::response(R1, &[0x05], 0)
            } else {
                synth::response(format, &bytes, 0)
            };
            exchange(index, 0x1aa, answer)
        }
        Op::Erase { busy } => exchange(38, 0, synth::response(R1B, &[0x00], *busy)),
        Op::Read { error, bad_crc } => {
            let mut words = exchange(17, n as u32, synth::response(R1, &[0x00], 0));
            if *error {
                words.extend(synth::error_token(0x08));
            } else {
                let data = pattern(16, seed);
                let crc = synth::crc16(&data) ^ u16::from(*bad_crc);
                words.extend(synth::read_block_with_crc(&data, crc));
            }
            words
        }
        Op::ReadMultiple { blocks } => {
            let mut words = exchange(18, n as u32, synth::response(R1, &[0x00], 0));
            for b in 0..*blocks {
                words.extend(synth::read_block(&pattern(16, seed ^ b)));
            }
            words.extend(synth::command(12, 0));
            words.push((0xff, 0x3c));
            words.extend(synth::response_after(1, R1B, &[0x00], 2));
            words
        }
        Op::Write { status, busy } => {
            let mut words = exchange(24, n as u32, synth::response(R1, &[0x00], 0));
            let response = data_response(*status);
            words.extend(synth::write_block(
                0xfe,
                &pattern(16, seed),
                response,
                *busy,
            ));
            words
        }
        Op::WriteMultiple { blocks } => {
            let mut words = exchange(25, n as u32, synth::response(R1, &[0x00], 0));
            for b in 0..*blocks {
                let response = data_response(0b010);
                words.extend(synth::write_block(
                    0xfc,
                    &pattern(16, seed ^ b),
                    response,
                    1,
                ));
            }
            words.extend(synth::stop_tran(3));
            words
        }
        Op::Stray { byte } => vec![(*byte, 0xff), (0x7f, 0xff)],
    }
}

/// The samples of `ops`; an operation marked `true` shares the chip select period of the one
/// before it.
fn scenario(ops: &[(Op, bool)], mode: u8) -> uiharu_core::LogicBlock {
    let config = SpiConfig {
        mode,
        ..SpiConfig::default()
    };
    let mut waveform = SpiWaveform::new(config, CHANNELS, HALF, Tick::ZERO);
    waveform.idle(SELECT);
    for (n, (op, shared)) in ops.iter().enumerate() {
        if !(*shared && waveform.is_selected()) {
            if waveform.is_selected() {
                waveform.release();
                waveform.idle(3 + n as u64 % 7);
            }
            waveform.select();
        }
        waveform.words(op_words(op, n));
    }
    if waveform.is_selected() {
        waveform.release();
    }
    waveform.idle(5);
    waveform.finish()
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 64, ..ProptestConfig::default() })]

    /// Random traffic cut into random blocks, read with random pauses through tiny queues and
    /// budgets, yields the events of one uninterrupted run; every operation ends once, and
    /// the stream settles in order.
    #[test]
    fn random_traffic_decodes_alike_however_it_is_cut(
        ops in prop::collection::vec((op(), any::<bool>()), 1..12),
        mode in prop::sample::select(vec![0u8, 3]),
        sizes in prop::collection::vec(1u64..300, 1..5),
        pauses in prop::collection::vec(0usize..9, 1..6),
        queue_capacity in 1usize..5,
        budget in 1u32..30,
        cut in prop::option::of(0.05f64..1.0),
        gap in prop::option::of((0.05f64..0.95, 1u64..150)),
    ) {
        let whole = scenario(&ops, mode);
        let samples = match cut {
            Some(cut) => {
                let end = Tick(((whole.len() as f64) * cut) as u64).max(Tick(1));
                whole.slice(TickRange::new(Tick::ZERO, end)).expect("a slice")
            }
            None => whole,
        };
        let gaps: Vec<TickRange> = gap
            .and_then(|(at, len)| {
                let start = ((samples.len() as f64) * at) as u64;
                let end = (start + len).min(samples.len());
                (start < end).then(|| TickRange::new(Tick(start), Tick(end)))
            })
            .into_iter()
            .collect();
        let spi = SpiConfig { mode, ..SpiConfig::default() };
        let mut reference = build(spi.clone(), blocks_of(16), GraphConfig::default())
            .expect("builds");
        reference.feed_with_gaps(&samples, &gaps, &[u64::MAX]);
        reference.end();

        let config = GraphConfig { queue_capacity, budget };
        let mut harness = build(spi, blocks_of(16), config).expect("builds");
        harness.drain_on_full = 1;
        harness.feed_cut(&samples, &gaps, &sizes, &pauses);
        harness.end();
        prop_assert_eq!(harness.without_progress(), reference.without_progress());

        let mut open = None;
        let mut settled = Tick::ZERO;
        for event in reference.events() {
            prop_assert!(event.settled >= settled, "settled out of order: {event:?}");
            settled = event.settled;
            match event.payload {
                SdEvent::Command { op, .. } => {
                    prop_assert_eq!(open, None, "a command inside an operation");
                    open = Some(op);
                }
                SdEvent::End { op, .. } => {
                    prop_assert_eq!(open, Some(op), "an end without its command");
                    open = None;
                }
                SdEvent::Data { bytes, .. } => prop_assert!(bytes.len() <= 32),
                SdEvent::UnknownCommand { .. } => prop_assert_eq!(open, None),
                _ => prop_assert!(open.is_some(), "an event outside an operation"),
            }
        }
        prop_assert_eq!(open, None, "an operation never ended");
    }
}
