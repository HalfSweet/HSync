// SPDX-License-Identifier: Apache-2.0

//! The nRF24L01+ decoder on synthesized SPI waveforms.

use serde::Serialize;
use uiharu_core::testing::{self, Harness, assert_block_invariant};
use uiharu_core::{GraphConfig, GraphError, Item, LogicBlock, Tick, TickRange, TimeBase};
use uiharu_protocols::nrf24l01::{Direction, Nrf24Event, Nrf24l01, synth};
use uiharu_protocols::spi::synth::SpiWaveform;
use uiharu_protocols::spi::{BitOrder, Spi, SpiConfig};

const CHANNELS: u8 = 4;
const RATE: u64 = 1_000_000;
const HALF: u64 = 2;
/// Samples per word: eight bits of two half periods.
const WORD: u64 = 8 * 2 * HALF;
/// Where chip select asserts in [`waveform`].
const SELECT: u64 = 10;
/// STATUS after reset: no interrupt, RX FIFO empty.
const STATUS: u8 = 0x0e;

fn build(spi: SpiConfig, graph: GraphConfig) -> Result<Harness<Nrf24Event>, GraphError> {
    Harness::build(CHANNELS, TimeBase::new(RATE), graph, |graph, source| {
        let spi = graph.add(Spi::new(spi).expect("a valid configuration"));
        graph.connect(source.samples, Spi::input(spi))?;
        let radio = graph.add(Nrf24l01::new());
        graph.connect(Spi::events(spi), Nrf24l01::input(radio))?;
        Ok(Nrf24l01::events(radio))
    })
}

fn harness() -> Harness<Nrf24Event> {
    build(SpiConfig::default(), GraphConfig::default()).expect("builds")
}

fn waveform() -> SpiWaveform {
    let mut waveform = SpiWaveform::new(SpiConfig::default(), CHANNELS, HALF, Tick::ZERO);
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

/// The command word of a transaction that began at [`SELECT`], with chip select's assertion.
fn head() -> String {
    format!("[{SELECT},{})@{}", last_bit(0) + 1, last_bit(0))
}

/// Chip select's release after `count` words of a transaction that began at [`SELECT`].
fn release(count: u64) -> String {
    let tick = first_bit(count);
    format!("[{tick},{})@{tick}", tick + 1)
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
fn describe(items: &[Item<Nrf24Event>], ticks: bool) -> Vec<String> {
    testing::without_progress(items)
        .iter()
        .map(|item| match item {
            Item::Event(event) => {
                let what = match &event.payload {
                    Nrf24Event::Command {
                        op,
                        tx,
                        command,
                        register,
                        register_name,
                        pipe,
                        status,
                    } => format!(
                        "command op{} tx{} {} reg={} {} pipe={} status={status:02x}",
                        op.0,
                        tx.0,
                        name(command),
                        register.map_or("--".to_owned(), |register| format!("{register:02x}")),
                        register_name.map_or("-".to_owned(), |register| name(&register)),
                        pipe.map_or("-".to_owned(), |pipe| pipe.to_string()),
                    ),
                    Nrf24Event::Data {
                        op,
                        direction,
                        offset,
                        bytes,
                    } => {
                        let direction = match direction {
                            Direction::In => "in",
                            Direction::Out => "out",
                        };
                        format!("data op{} {direction} +{offset} {}", op.0, hex(bytes))
                    }
                    Nrf24Event::End { op, reason } => format!("end op{} {}", op.0, name(reason)),
                    Nrf24Event::Unknown {
                        tx,
                        command,
                        status,
                    } => format!("unknown tx{} {command:02x} status={status:02x}", tx.0),
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

/// Decodes `transactions`, each in its own chip select period, with the events' ticks.
fn decode_with_ticks(transactions: &[Vec<(u8, u8)>]) -> Vec<String> {
    let mut waveform = waveform();
    for words in transactions {
        waveform.transaction(words.iter().copied());
        waveform.idle(10);
    }
    let block = waveform.finish();
    let mut harness = harness();
    harness.run_all(&block);
    describe(&harness.items, true)
}

/// Decodes `transactions` and returns the lines without ticks, the end of the stream left out.
fn decode(transactions: &[Vec<(u8, u8)>]) -> Vec<String> {
    decode_with_ticks(transactions)
        .into_iter()
        .filter(|line| !line.starts_with("eof "))
        .map(|line| {
            line.rsplit_once(' ')
                .map_or(line.as_str(), |(what, _)| what)
                .to_owned()
        })
        .collect()
}

#[test]
fn register_writes_of_config_and_tx_addr() {
    let lines = decode_with_ticks(&[
        synth::write_register(0x00, STATUS, &[0x0b]),
        synth::write_register(0x10, STATUS, &[0xe7, 0xe6, 0xe5, 0xe4, 0xe3]),
    ]);
    assert_eq!(
        lines[..3],
        [
            format!(
                "command op1 tx1 w-register reg=00 config pipe=- status=0e {}",
                head()
            ),
            // CONFIG is one byte: the data is whole with it.
            format!("data op1 in +0 0b {}", words(1, 2)),
            format!("end op1 complete {}", release(2)),
        ]
    );
    assert_eq!(
        decode(&[
            synth::write_register(0x00, STATUS, &[0x0b]),
            synth::write_register(0x10, STATUS, &[0xe7, 0xe6, 0xe5, 0xe4, 0xe3]),
        ]),
        [
            "command op1 tx1 w-register reg=00 config pipe=- status=0e",
            "data op1 in +0 0b",
            "end op1 complete",
            "command op2 tx2 w-register reg=10 tx-addr pipe=- status=0e",
            "data op2 in +0 e7e6e5e4e3",
            "end op2 complete",
        ]
    );
}

#[test]
fn register_reads_of_status_and_rx_addr_p1() {
    assert_eq!(
        decode(&[
            synth::read_register(0x07, STATUS, &[STATUS]),
            synth::read_register(0x0b, STATUS, &[0xc2, 0xc2, 0xc2, 0xc2, 0xc2]),
            // Three bytes, as SETUP_AW may set.
            synth::read_register(0x0b, 0x4e, &[0xc2, 0xc3, 0xc4]),
            synth::read_register(0x17, STATUS, &[0x11]),
        ]),
        [
            "command op1 tx1 r-register reg=07 status pipe=- status=0e",
            "data op1 out +0 0e",
            "end op1 complete",
            "command op2 tx2 r-register reg=0b rx-addr-p1 pipe=- status=0e",
            "data op2 out +0 c2c2c2c2c2",
            "end op2 complete",
            "command op3 tx3 r-register reg=0b rx-addr-p1 pipe=- status=4e",
            "data op3 out +0 c2c3c4",
            "end op3 complete",
            "command op4 tx4 r-register reg=17 fifo-status pipe=- status=0e",
            "data op4 out +0 11",
            "end op4 complete",
        ]
    );
    // An address shorter than five bytes is whole only when chip select is released.
    let lines = decode_with_ticks(&[synth::read_register(0x0b, 0x4e, &[0xc2, 0xc3, 0xc4])]);
    let release_tick = first_bit(4);
    assert_eq!(
        lines[1],
        format!(
            "data op1 out +0 c2c3c4 [{},{})@{release_tick}",
            first_bit(1),
            last_bit(3) + 1
        )
    );
}

#[test]
fn a_payload_of_32_bytes_goes_out_in_one_batch_before_the_release() {
    let payload: Vec<u8> = (0..32u8).map(|i| i.wrapping_mul(37) ^ 0x5a).collect();
    let mut waveform = waveform();
    waveform.transaction(synth::write(0xa0, STATUS, &payload));
    waveform.idle(10);
    let block = waveform.finish();
    // Fed up to the last data bit: the payload is out, the end is not.
    let cut = Tick(last_bit(32) + 1);
    let head_block = block
        .slice(TickRange::new(Tick::ZERO, cut))
        .expect("a slice");
    let mut harness = harness();
    harness.feed(&head_block, &[23]);
    harness.collect();
    assert_eq!(
        describe(&harness.items, true),
        [
            format!(
                "command op1 tx1 w-tx-payload reg=-- - pipe=- status=0e {}",
                head()
            ),
            format!("data op1 in +0 {} {}", hex(&payload), words(1, 33)),
        ]
    );
    let tail = block
        .slice(TickRange::new(cut, block.end()))
        .expect("a slice");
    harness.feed(&tail, &[23]);
    harness.end();
    let lines = describe(&harness.items, true);
    assert_eq!(lines[2], format!("end op1 complete {}", release(33)));
    assert_eq!(lines.len(), 4, "{lines:?}");
}

#[test]
fn received_payloads_and_their_width() {
    let payload = [0x48, 0x65, 0x6c, 0x6c, 0x6f];
    assert_eq!(
        decode(&[
            synth::read(0x60, 0x40, &[5]),
            synth::read(0x61, 0x40, &payload),
            synth::write_register(0x07, 0x40, &[0x40]),
        ]),
        [
            "command op1 tx1 r-rx-pl-wid reg=-- - pipe=- status=40",
            "data op1 out +0 05",
            "end op1 complete",
            "command op2 tx2 r-rx-payload reg=-- - pipe=- status=40",
            "data op2 out +0 48656c6c6f",
            "end op2 complete",
            "command op3 tx3 w-register reg=07 status pipe=- status=40",
            "data op3 in +0 40",
            "end op3 complete",
        ]
    );
}

#[test]
fn commands_without_data_report_status() {
    assert_eq!(
        decode(&[
            synth::command(0xe1, STATUS),
            synth::command(0xe2, STATUS),
            synth::command(0xe3, 0x1e),
            synth::command(0xff, 0x2e),
        ]),
        [
            "command op1 tx1 flush-tx reg=-- - pipe=- status=0e",
            "end op1 complete",
            "command op2 tx2 flush-rx reg=-- - pipe=- status=0e",
            "end op2 complete",
            "command op3 tx3 reuse-tx-pl reg=-- - pipe=- status=1e",
            "end op3 complete",
            "command op4 tx4 nop reg=-- - pipe=- status=2e",
            "end op4 complete",
        ]
    );
    let lines = decode_with_ticks(&[synth::command(0xff, 0x2e)]);
    assert_eq!(
        lines[..2],
        [
            format!("command op1 tx1 nop reg=-- - pipe=- status=2e {}", head()),
            format!("end op1 complete {}", release(1)),
        ]
    );
}

#[test]
fn ack_payloads_name_their_pipe() {
    let transactions: Vec<Vec<(u8, u8)>> = (0..8u8)
        .map(|pipe| synth::write(0xa8 | pipe, STATUS, &[pipe, 0xaa]))
        .collect();
    let lines = decode(&transactions);
    let mut expected = Vec::new();
    for pipe in 0..6 {
        expected.push(format!(
            "command op{} tx{} w-ack-payload reg=-- - pipe={pipe} status=0e",
            pipe + 1,
            pipe + 1
        ));
        expected.push(format!("data op{} in +0 {pipe:02x}aa", pipe + 1));
        expected.push(format!("end op{} complete", pipe + 1));
    }
    // PPP of 110 and 111 names no pipe.
    expected.push("unknown tx7 ae status=0e".to_owned());
    expected.push("unknown tx8 af status=0e".to_owned());
    assert_eq!(lines, expected);
}

#[test]
fn bytes_that_name_no_command_are_unknown_and_activate_needs_its_key() {
    let lines = decode(&[
        synth::write(0x40, STATUS, &[1, 2, 3]),
        synth::activate(STATUS),
        synth::write(0x50, STATUS, &[0x00, 0x73]),
        synth::command(0x50, STATUS),
        synth::read(0xe0, 0x0f, &[1]),
        synth::write(0xb0, STATUS, &[0x99]),
    ]);
    assert_eq!(
        lines,
        [
            "unknown tx1 40 status=0e",
            "command op1 tx2 activate reg=-- - pipe=- status=0e",
            "data op1 in +0 73",
            "end op1 complete",
            "unknown tx3 50 status=0e",
            "unknown tx4 50 status=0e",
            "unknown tx5 e0 status=0f",
            "command op2 tx6 w-tx-payload-noack reg=-- - pipe=- status=0e",
            "data op2 in +0 99",
            "end op2 complete",
        ]
    );
    // ACTIVATE is decided by its key, and alone by the release.
    let lines = decode_with_ticks(&[synth::activate(STATUS)]);
    assert_eq!(
        lines[0],
        format!(
            "command op1 tx1 activate reg=-- - pipe=- status=0e [{SELECT},{})@{}",
            last_bit(0) + 1,
            last_bit(1)
        )
    );
    let lines = decode_with_ticks(&[synth::command(0x50, STATUS)]);
    let release_tick = first_bit(1);
    assert_eq!(
        lines[0],
        format!(
            "unknown tx1 50 status=0e [{SELECT},{})@{release_tick}",
            last_bit(0) + 1
        )
    );
}

#[test]
fn bytes_past_what_a_command_takes_make_it_too_long() {
    let payload = [0x33u8; 33];
    assert_eq!(
        decode(&[
            synth::write(0xe1, STATUS, &[0x00]),
            synth::write_register(0x00, STATUS, &[0x0b, 0x0c]),
            synth::write(0xa0, STATUS, &payload),
            synth::read(0x60, STATUS, &[5, 5]),
            synth::write(0x50, STATUS, &[0x73, 0x73]),
            synth::read_register(0x0a, STATUS, &[1, 2, 3, 4, 5, 6, 7]),
        ]),
        [
            "command op1 tx1 flush-tx reg=-- - pipe=- status=0e",
            "end op1 too-long",
            "command op2 tx2 w-register reg=00 config pipe=- status=0e",
            "data op2 in +0 0b",
            "end op2 too-long",
            "command op3 tx3 w-tx-payload reg=-- - pipe=- status=0e",
            &format!("data op3 in +0 {}", hex(&payload[..32])),
            "end op3 too-long",
            "command op4 tx4 r-rx-pl-wid reg=-- - pipe=- status=0e",
            "data op4 out +0 05",
            "end op4 too-long",
            "command op5 tx5 activate reg=-- - pipe=- status=0e",
            "data op5 in +0 73",
            "end op5 too-long",
            "command op6 tx6 r-register reg=0a rx-addr-p0 pipe=- status=0e",
            "data op6 out +0 0102030405",
            "end op6 too-long",
        ]
    );
    // The end covers the first byte too many.
    let lines = decode_with_ticks(&[synth::write(0xe1, STATUS, &[0x00, 0x01])]);
    assert_eq!(lines[1], format!("end op1 too-long {}", words(1, 2)));
    assert_eq!(lines.len(), 3, "{lines:?}");
}

#[test]
fn a_command_without_its_data_byte_is_incomplete() {
    assert_eq!(
        decode(&[
            synth::read_register(0x07, STATUS, &[]),
            synth::write_register(0x05, STATUS, &[]),
            synth::write(0xa0, STATUS, &[]),
            synth::read(0x60, STATUS, &[]),
            // An empty transaction has no command.
            vec![],
            synth::read(0x61, STATUS, &[]),
        ]),
        [
            "command op1 tx1 r-register reg=07 status pipe=- status=0e",
            "end op1 incomplete",
            "command op2 tx2 w-register reg=05 rf-ch pipe=- status=0e",
            "end op2 incomplete",
            "command op3 tx3 w-tx-payload reg=-- - pipe=- status=0e",
            "end op3 incomplete",
            "command op4 tx4 r-rx-pl-wid reg=-- - pipe=- status=0e",
            "end op4 incomplete",
            "command op5 tx6 r-rx-payload reg=-- - pipe=- status=0e",
            "end op5 incomplete",
        ]
    );
    let lines = decode_with_ticks(&[synth::read_register(0x07, STATUS, &[])]);
    assert_eq!(lines[1], format!("end op1 incomplete {}", release(1)));
}

#[test]
fn a_command_cut_by_the_end_of_the_samples_is_aborted() {
    let payload: Vec<u8> = (1..=10).collect();
    let mut waveform = waveform();
    waveform.select();
    waveform.words(synth::read(0x61, 0x40, &payload));
    waveform.partial_word(0xff, 0x0b, 5);
    let block = waveform.finish();
    let mut harness = harness();
    harness.run_all(&block);
    let end = block.end();
    assert_eq!(
        describe(&harness.items, true),
        [
            format!(
                "command op1 tx1 r-rx-payload reg=-- - pipe=- status=40 {}",
                head()
            ),
            format!(
                "data op1 out +0 {} [{},{})@{end}",
                hex(&payload),
                first_bit(1),
                last_bit(10) + 1
            ),
            format!("end op1 aborted @{end}"),
            format!("eof {end}"),
        ]
    );
}

#[test]
fn a_gap_aborts_the_command_and_skips_the_rest_of_the_transaction() {
    let payload: Vec<u8> = (0..20).map(|i| i * 3).collect();
    let mut waveform = waveform();
    waveform.transaction(synth::read(0x61, 0x40, &payload));
    waveform.idle(10);
    waveform.transaction(synth::write_register(0x05, STATUS, &[0x4c]));
    waveform.idle(10);
    let block = waveform.finish();
    // Lose the 8th and 9th payload bytes.
    let gap = TickRange::new(Tick(first_bit(8) + 2), Tick(first_bit(10) - 3));
    let mut harness = harness();
    harness.feed_with_gaps(&block, &[gap], &[19]);
    harness.end();
    let lines = describe(&harness.items, true);
    assert_eq!(
        lines[..4],
        [
            format!(
                "command op1 tx1 r-rx-payload reg=-- - pipe=- status=40 {}",
                head()
            ),
            format!(
                "data op1 out +0 {} [{},{})@{}",
                hex(&payload[..7]),
                first_bit(1),
                last_bit(7) + 1,
                gap.start
            ),
            format!("end op1 aborted @{}", gap.start),
            format!("gap {gap}"),
        ]
    );
    // The rest of the read is skipped; the next transaction is whole.
    let rest: Vec<String> = lines[4..]
        .iter()
        .map(|line| {
            line.rsplit_once(' ')
                .map_or(line.as_str(), |(what, _)| what)
                .to_owned()
        })
        .collect();
    assert_eq!(
        rest,
        [
            "command op2 tx3 w-register reg=05 rf-ch pipe=- status=0e",
            "data op2 in +0 4c",
            "end op2 complete",
            "eof",
        ]
    );
}

#[test]
fn a_transaction_under_way_at_the_first_sample_is_skipped() {
    let mut waveform = SpiWaveform::new(SpiConfig::default(), CHANNELS, HALF, Tick::ZERO);
    // Chip select is asserted from the first sample on: the payload bytes after the command
    // byte look like commands, but none is.
    waveform.select();
    waveform.words(synth::write(0xa0, STATUS, &[0xff, 0x20, 0x01]));
    waveform.release();
    waveform.idle(10);
    waveform.transaction(synth::command(0xe1, STATUS));
    waveform.idle(10);
    let block = waveform.finish();
    let mut harness = harness();
    harness.run_all(&block);
    let lines: Vec<String> = describe(&harness.items, false)
        .into_iter()
        .filter(|line| !line.starts_with("eof "))
        .collect();
    assert_eq!(
        lines,
        [
            "command op1 tx2 flush-tx reg=-- - pipe=- status=0e",
            "end op1 complete",
        ]
    );
}

#[test]
fn progress_never_passes_a_command_under_way() {
    let block = rich_waveform();
    for sizes in [&[1u64][..], &[5], &[7, 64], &[97]] {
        let mut harness = harness();
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
        // The SPI reader passes progress on between transactions only.
        assert!(progress > 0, "blocks of {sizes:?}");
    }
}

#[test]
fn the_bus_must_meet_the_decoders_requirements() {
    let refused = |spi: SpiConfig| build(spi, GraphConfig::default()).expect_err("refused");
    assert_eq!(
        refused(SpiConfig {
            cs: None,
            ..SpiConfig::default()
        }),
        GraphError::Requirement {
            from: "spi#0.events".into(),
            to: "nrf24l01#1.bus".into(),
            key: "cs",
            requirement: "= true".into(),
            found: "false".into(),
        }
    );
    for (spi, key) in [
        (
            SpiConfig {
                word_bits: 7,
                ..SpiConfig::default()
            },
            "wordBits",
        ),
        (
            SpiConfig {
                bit_order: BitOrder::LsbFirst,
                ..SpiConfig::default()
            },
            "bitOrder",
        ),
        (
            SpiConfig {
                mosi: None,
                ..SpiConfig::default()
            },
            "mosi",
        ),
        (
            SpiConfig {
                miso: None,
                ..SpiConfig::default()
            },
            "miso",
        ),
    ] {
        assert!(
            matches!(refused(spi), GraphError::Requirement { key: found, .. } if found == key),
            "{key}"
        );
    }
}

/// A radio set up, sending, receiving and flushing, with every kind of mistake among it.
fn rich_waveform() -> LogicBlock {
    let payload: Vec<u8> = (0..32u8).map(|i| i ^ 0xa5).collect();
    let transactions = [
        synth::write_register(0x00, STATUS, &[0x0e]),
        synth::write_register(0x10, STATUS, &[0xe7; 5]),
        synth::write_register(0x0a, STATUS, &[0xe7; 5]),
        synth::read_register(0x03, STATUS, &[0x03]),
        synth::write(0xa0, STATUS, &payload),
        synth::command(0xff, 0x2e),
        synth::write_register(0x07, 0x2e, &[0x20]),
        synth::read(0x60, 0x40, &[12]),
        synth::read(0x61, 0x40, &payload[..12]),
        synth::write(0xaa, STATUS, &payload[..4]),
        synth::activate(STATUS),
        synth::write(0x50, STATUS, &[0x01]),
        synth::write(0x77, STATUS, &[1, 2]),
        synth::command(0xe1, STATUS),
        synth::write(0xe2, STATUS, &[0]),
        synth::read_register(0x1e, STATUS, &[1, 2]),
        synth::write(0xb0, STATUS, &[]),
        vec![],
        synth::read_register(0x0b, STATUS, &[1, 2, 3, 4, 5, 6]),
    ];
    let mut waveform = waveform();
    for round in 0..2u64 {
        for (n, words) in transactions.iter().enumerate() {
            waveform.transaction(words.iter().copied());
            waveform.idle(5 + (n as u64 + round) % 4 * 7);
        }
    }
    waveform.finish()
}

#[test]
fn every_cut_and_pause_yields_the_same_events() {
    let block = rich_waveform();
    let len = block.len();
    let gaps = [
        TickRange::new(Tick(len / 8), Tick(len / 8 + 40)),
        TickRange::new(Tick(len / 3), Tick(len / 3 + 3)),
        TickRange::new(Tick(len / 2 + 17), Tick(len / 2 + 200)),
        TickRange::new(Tick(len - 60), Tick(len - 59)),
    ];
    let reference = {
        let mut harness = harness();
        harness.feed_with_gaps(&block, &gaps, &[u64::MAX]);
        harness.end();
        describe(&harness.items, true)
    };
    assert!(reference.len() > 80, "{reference:?}");
    for what in [
        "too-long",
        "incomplete",
        "aborted",
        "unknown ",
        "activate",
        "gap ",
    ] {
        assert!(
            reference.iter().any(|line| line.contains(what)),
            "{what}: {reference:?}"
        );
    }
    assert_block_invariant(
        |graph| build(SpiConfig::default(), graph).expect("builds"),
        &block,
        &gaps,
    );
}
