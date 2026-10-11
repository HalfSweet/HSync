// SPDX-License-Identifier: Apache-2.0

//! The AM230x decoder on synthesized waveforms.

use uiharu_core::testing::{Harness, assert_block_invariant, without_progress};
use uiharu_core::{GraphConfig, GraphError, Item, LogicBlock, Tick, TickRange, TimeBase};
use uiharu_protocols::am230x::synth::{Am230xWaveform, Timing, with_checksum};
use uiharu_protocols::am230x::{Am230x, Am230xConfig, Am230xEvent, Model};

const CHANNELS: u8 = 2;
/// 1 MHz: a tick is a microsecond.
const RATE: u64 = 1_000_000;
/// From a reading's start signal to its first bit's fall: 1 ms, 30 µs, 80 µs and 80 µs.
const TO_BITS: u64 = 1_190;

/// The data line on channel 1; channel 0 stays low.
fn config(model: Model) -> Am230xConfig {
    Am230xConfig {
        data: 1,
        model,
        ..Am230xConfig::default()
    }
}

fn build(model: Model, rate: u64, graph: GraphConfig) -> Harness<Am230xEvent> {
    Harness::build(CHANNELS, TimeBase::new(rate), graph, |graph, source| {
        let sensor = graph.add(Am230x::new(config(model)).expect("a valid configuration"));
        graph.connect(source.samples, Am230x::input(sensor))?;
        Ok(Am230x::events(sensor))
    })
    .expect("builds")
}

fn new_waveform(model: Model) -> Am230xWaveform {
    let mut waveform =
        Am230xWaveform::new(config(model), CHANNELS, TimeBase::new(RATE), Tick::ZERO);
    waveform.idle_us(2_000);
    waveform
}

fn at(span: TickRange, settled: Tick) -> String {
    if span.is_empty() {
        format!("@{settled}")
    } else {
        format!("[{},{})@{settled}", span.start, span.end)
    }
}

/// One line per item, progress left out: it depends on how the samples were cut.
fn describe(items: &[Item<Am230xEvent>]) -> Vec<String> {
    without_progress(items)
        .iter()
        .map(|item| match item {
            Item::Event(event) => {
                let what = match event.payload {
                    Am230xEvent::Reading {
                        bytes,
                        checksum_ok,
                        humidity_raw,
                        temperature_raw,
                    } => {
                        let hex: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
                        let checksum = if checksum_ok { "ok" } else { "bad" };
                        format!(
                            "reading {hex} checksum {checksum} rh {humidity_raw} t {temperature_raw}"
                        )
                    }
                    Am230xEvent::Error { reason } => format!("error {reason:?}"),
                };
                format!("{what} {}", at(event.span, event.settled))
            }
            Item::Gap(range) => format!("gap {range}"),
            Item::Progress(tick) => format!("progress {tick}"),
            Item::End(tick) => format!("eof {tick}"),
        })
        .collect()
}

fn decode(model: Model, block: &LogicBlock) -> Vec<String> {
    let mut harness = build(model, RATE, GraphConfig::default());
    harness.run_all(block);
    describe(&harness.items)
}

/// Where the sensor releases the line after a reading of `bytes` whose start signal fell at
/// `start`, with the typical times: a 0 lasts 50 + 27 µs, a 1 50 + 70 µs, and the final low 50.
fn final_release(start: u64, start_us: u64, bytes: [u8; 5]) -> u64 {
    let ones: u64 = bytes.iter().map(|byte| u64::from(byte.count_ones())).sum();
    start + start_us + 190 + 40 * 50 + ones * 70 + (40 - ones) * 27 + 50
}

#[test]
fn a_reading_and_its_checksum() {
    // 65.2 % and 35.1 °C: 0x028c and 0x015f; 0x02 + 0x8c + 0x01 + 0x5f = 0xee.
    let bytes = [0x02, 0x8c, 0x01, 0x5f, 0xee];
    let mut waveform = new_waveform(Model::Am2302);
    let start = waveform.position().get();
    waveform.reading(bytes);
    waveform.idle_us(500);
    let block = waveform.finish();
    let release = final_release(start, 1_000, bytes);
    assert_eq!(
        decode(Model::Am2302, &block),
        vec![
            format!(
                "reading 028c015fee checksum ok rh 652 t 351 [{start},{})@{release}",
                release + 1
            ),
            format!("eof {}", block.end()),
        ]
    );
    // A checksum one off.
    let mut waveform = new_waveform(Model::Am2302);
    waveform.reading([0x02, 0x8c, 0x01, 0x5f, 0xef]);
    waveform.idle_us(500);
    let lines = decode(Model::Am2302, &waveform.finish());
    assert!(
        lines[0].starts_with("reading 028c015fef checksum bad "),
        "{lines:?}"
    );
}

#[test]
fn a_negative_temperature_sets_bit_15() {
    // 40.0 % and -10.1 °C: 0x0190 and 0x8065; 0x01 + 0x90 + 0x80 + 0x65 = 0x176, so 0x76.
    let bytes = [0x01, 0x90, 0x80, 0x65, 0x76];
    assert_eq!(with_checksum([0x01, 0x90, 0x80, 0x65]), bytes);
    let mut waveform = new_waveform(Model::Am2302);
    waveform.reading(bytes);
    waveform.idle_us(500);
    let lines = decode(Model::Am2302, &waveform.finish());
    assert!(
        lines[0].starts_with("reading 0190806576 checksum ok rh 400 t 32869 "),
        "{lines:?}"
    );
}

#[test]
fn a_dht11_needs_its_longer_start_signal() {
    // 45 %, 23.5 °C: integral and decimal parts; 45 + 0 + 23 + 5 = 73.
    let bytes = [45, 0, 23, 5, 73];
    let mut waveform = new_waveform(Model::Dht11);
    let start = waveform.position().get();
    waveform.reading(bytes);
    waveform.idle_us(500);
    // The same sensor asked with the AM2302's 1 ms start signal does not count it as one.
    let short = waveform.position().get();
    waveform.set_timing(Timing {
        start: 1_000_000,
        ..Timing::of(Model::Dht11)
    });
    waveform.reading(bytes);
    waveform.idle_us(500);
    let block = waveform.finish();
    let release = final_release(start, 18_000, bytes);
    assert_eq!(
        decode(Model::Dht11, &block),
        vec![
            format!(
                "reading 2d00170549 checksum ok rh 11520 t 5893 [{start},{})@{release}",
                release + 1
            ),
            format!("error Start [{short},{})@{}", short + 1_001, short + 1_000),
            format!("eof {}", block.end()),
        ]
    );
    // An AM2302 decoder takes both: its start signal has no longest bound.
    let lines = decode(Model::Am2302, &block);
    assert_eq!(lines.len(), 3, "{lines:?}");
    assert!(
        lines
            .iter()
            .take(2)
            .all(|line| line.starts_with("reading 2d00170549 "))
    );
}

#[test]
fn a_pulse_without_a_start_signal_is_an_error() {
    // The sensor's response and bits with no start signal before them, then a whole reading.
    let bytes = with_checksum([0, 0x55, 0, 0x22]);
    let mut waveform = new_waveform(Model::Am2302);
    let response = waveform.position().get();
    waveform.response();
    waveform.bytes(&bytes);
    waveform.end();
    waveform.idle_us(1_000);
    let start = waveform.position().get();
    waveform.reading(bytes);
    waveform.idle_us(500);
    // A start signal of 0.6 ms, shorter than 1 ms less 30 %.
    let short = waveform.position().get();
    waveform.low(600_000);
    waveform.idle_us(500);
    let block = waveform.finish();
    let lines = decode(Model::Am2302, &block);
    assert_eq!(
        lines[0],
        format!(
            "error Start [{response},{})@{}",
            response + 81,
            response + 80
        )
    );
    assert!(
        lines[1].starts_with(&format!(
            "reading 0055002277 checksum ok rh 85 t 34 [{start},"
        )),
        "{lines:?}"
    );
    assert_eq!(
        lines[2],
        format!("error Start [{short},{})@{}", short + 601, short + 600)
    );
    assert_eq!(lines.len(), 4, "{lines:?}");

    // A line that was idle when a gap began is idle after it.
    let gap = TickRange::new(Tick(response - 600), Tick(response - 200));
    let mut harness = build(Model::Am2302, RATE, GraphConfig::default());
    harness.feed_with_gaps(&block, &[gap], &[500]);
    harness.end();
    let lines = describe(&harness.items);
    assert_eq!(
        lines[..2],
        [
            format!("gap {gap}"),
            format!(
                "error Start [{response},{})@{}",
                response + 81,
                response + 80
            ),
        ]
    );
}

#[test]
fn a_response_outside_its_windows_is_an_error() {
    let bytes = with_checksum([1, 2, 3, 4]);
    // No answer: 200 µs after the release, and 30 % more, the line is still high.
    let mut waveform = new_waveform(Model::Am2302);
    let start = waveform.position().get();
    waveform.start_signal();
    waveform.idle_us(3_000);
    let next = waveform.position().get();
    waveform.reading(bytes);
    waveform.idle_us(500);
    let lines = decode(Model::Am2302, &waveform.finish());
    let release = start + 1_000;
    let end = final_release(next, 1_000, bytes);
    assert_eq!(
        lines[..2],
        [
            format!(
                "error Response [{start},{})@{}",
                release + 261,
                release + 260
            ),
            format!(
                "reading 010203040a checksum ok rh 258 t 772 [{next},{})@{end}",
                end + 1
            ),
        ]
    );

    // An AM2302 may answer up to 200 µs after the release; a DHT11 within 40 µs.
    for (model, release_us, ok) in [
        (Model::Am2302, 180, true),
        (Model::Dht11, 45, true),
        (Model::Dht11, 60, false),
    ] {
        let mut waveform = new_waveform(model);
        waveform.set_timing(Timing {
            release: release_us * 1_000,
            ..Timing::of(model)
        });
        waveform.reading(bytes);
        waveform.idle_us(500);
        let lines = decode(model, &waveform.finish());
        assert_eq!(
            lines[0].starts_with("reading 010203040a "),
            ok,
            "{model:?} answering after {release_us} µs: {lines:?}"
        );
    }

    // A response low of 30 µs ends too early; one of 150 µs too late.
    for (response_low, decided) in [(30_000, 30), (150_000, 104)] {
        let mut waveform = new_waveform(Model::Am2302);
        let start = waveform.position().get();
        waveform.set_timing(Timing {
            response_low,
            ..Timing::of(Model::Am2302)
        });
        waveform.reading(bytes);
        waveform.idle_us(500);
        let answer = start + 1_030;
        let lines = decode(Model::Am2302, &waveform.finish());
        assert_eq!(
            lines[0],
            format!(
                "error Response [{start},{})@{}",
                answer + decided + 1,
                answer + decided
            )
        );
        assert_eq!(lines.len(), 2, "the rest is skipped: {lines:?}");
    }

    // A response high of 120 µs ends too late.
    let mut waveform = new_waveform(Model::Am2302);
    let start = waveform.position().get();
    waveform.set_timing(Timing {
        response_high: 120_000,
        ..Timing::of(Model::Am2302)
    });
    waveform.reading(bytes);
    waveform.idle_us(500);
    let rise = start + 1_110;
    assert_eq!(
        decode(Model::Am2302, &waveform.finish())[0],
        format!("error Response [{start},{})@{}", rise + 105, rise + 104)
    );
}

#[test]
fn bits_outside_their_windows_are_errors() {
    // The sixth bit's high lasts 45 µs: too long for a 0, too short for a 1.
    let mut waveform = new_waveform(Model::Am2302);
    let start = waveform.position().get();
    let fall = start + TO_BITS + 5 * 77;
    waveform.start_signal();
    waveform.response();
    for _ in 0..5 {
        waveform.bit(false);
    }
    waveform.low(50_000);
    waveform.high(45_000);
    waveform.bytes(&[0; 5]);
    waveform.end();
    waveform.idle_us(500);
    let lines = decode(Model::Am2302, &waveform.finish());
    assert_eq!(
        lines[0],
        format!(
            "error Bit [{start},{})@{}",
            fall + 50 + 45 + 1,
            fall + 50 + 45
        )
    );
    assert_eq!(lines.len(), 2, "{lines:?}");

    // The third bit's low lasts 90 µs.
    let mut waveform = new_waveform(Model::Am2302);
    waveform.start_signal();
    waveform.response();
    waveform.bit(true);
    waveform.bit(false);
    waveform.low(90_000);
    waveform.high(27_000);
    waveform.idle_us(500);
    let fall = start + TO_BITS + 120 + 77;
    let lines = decode(Model::Am2302, &waveform.finish());
    assert_eq!(
        lines[0],
        format!("error Bit [{start},{})@{}", fall + 66, fall + 65)
    );
}

#[test]
fn a_sensor_that_stops_early_sends_too_few_bits() {
    let mut waveform = new_waveform(Model::Am2302);
    let start = waveform.position().get();
    waveform.start_signal();
    waveform.response();
    waveform.bytes(&[0xff, 0x00]);
    waveform.low(50_000);
    waveform.idle_us(1_000);
    let next = waveform.position().get();
    waveform.reading(with_checksum([9, 9, 9, 9]));
    waveform.idle_us(500);
    let lines = decode(Model::Am2302, &waveform.finish());
    // The seventeenth bit's low rises and the line stays high past the window of a 1.
    let rise = start + TO_BITS + 8 * 120 + 8 * 77 + 50;
    assert_eq!(
        lines[0],
        format!("error Count [{start},{})@{}", rise + 92, rise + 91)
    );
    assert!(
        lines[1].starts_with(&format!(
            "reading 0909090924 checksum ok rh 2313 t 2313 [{next},"
        )),
        "{lines:?}"
    );
}

#[test]
fn a_reading_cut_off_is_not_reported() {
    let bytes = with_checksum([0x03, 0x20, 0x00, 0xfa]);
    let mut waveform = new_waveform(Model::Am2302);
    let start = waveform.position().get();
    waveform.reading(bytes);
    waveform.idle_us(1_500);
    let next = waveform.position().get();
    waveform.reading(bytes);
    waveform.idle_us(500);
    let block = waveform.finish();
    let reference = decode(Model::Am2302, &block);
    assert_eq!(reference.len(), 3, "{reference:?}");

    // The samples end in the middle of the bits.
    let cut = Tick(start + TO_BITS + 1_000);
    let head = block
        .slice(TickRange::new(Tick::ZERO, cut))
        .expect("a slice");
    assert_eq!(decode(Model::Am2302, &head), vec![format!("eof {cut}")]);

    // A gap in the middle of the first reading: only the second is reported.
    let gap = TickRange::new(Tick(start + TO_BITS + 300), Tick(start + TO_BITS + 900));
    let mut harness = build(Model::Am2302, RATE, GraphConfig::default());
    harness.feed_with_gaps(&block, &[gap], &[333]);
    harness.end();
    assert_eq!(
        describe(&harness.items),
        vec![
            format!("gap {gap}"),
            reference[1].clone(),
            reference[2].clone()
        ]
    );
    assert!(reference[1].contains(&format!("[{next},")));
}

#[test]
fn a_start_signal_under_way_at_the_first_sample() {
    let bytes = with_checksum([0x02, 0x10, 0x00, 0xe1]);
    let mut waveform = new_waveform(Model::Am2302);
    let start = waveform.position().get();
    waveform.reading(bytes);
    waveform.idle_us(1_500);
    let next = waveform.position().get();
    waveform.reading(bytes);
    waveform.idle_us(500);
    let block = waveform.finish();
    let release = final_release(start, 1_000, bytes);
    // 800 µs of the start signal seen: long enough, the reading spans from the first sample.
    let from = start + 200;
    let tail = block
        .slice(TickRange::new(Tick(from), block.end()))
        .expect("a slice");
    let lines = decode(Model::Am2302, &tail);
    assert_eq!(
        lines[0],
        format!(
            "reading 021000e1f3 checksum ok rh 528 t 225 [{from},{})@{release}",
            release + 1
        )
    );
    assert_eq!(lines.len(), 3, "{lines:?}");
    // 500 µs seen: too little to tell; the reading is skipped without an error.
    let tail = block
        .slice(TickRange::new(Tick(start + 500), block.end()))
        .expect("a slice");
    let lines = decode(Model::Am2302, &tail);
    assert_eq!(lines.len(), 2, "{lines:?}");
    assert!(lines[0].contains(&format!("[{next},")), "{lines:?}");
}

/// Readings and errors of every kind, and where each exchange begins.
fn busy_line() -> (LogicBlock, Vec<u64>) {
    let mut waveform = new_waveform(Model::Am2302);
    let mut starts = Vec::new();
    for n in 0..8u8 {
        starts.push(waveform.position().get());
        let bytes = with_checksum([n, n.wrapping_mul(31), 0x80 | n, n ^ 0x5a]);
        match n % 4 {
            0 => waveform.reading(bytes),
            1 => {
                // The response alone: a pulse without a start signal.
                waveform.response();
                waveform.bytes(&bytes[..2]);
                waveform.end();
            }
            2 => {
                waveform.start_signal();
                waveform.response();
                waveform.bytes(&bytes[..3]);
            }
            _ => {
                let mut bad = bytes;
                bad[4] ^= 0x10;
                waveform.reading(bad);
            }
        }
        waveform.idle_us(700 + 37 * u64::from(n));
    }
    (waveform.finish(), starts)
}

#[test]
fn every_cut_and_pause_yields_the_same_events() {
    let (block, starts) = busy_line();
    let reference = decode(Model::Am2302, &block);
    for kind in ["checksum ok", "checksum bad", "error Start", "error Count"] {
        assert!(
            reference.iter().any(|line| line.contains(kind)),
            "{kind} in {reference:?}"
        );
    }
    let gaps = [
        // In a reading's bits, in its response, and on an idle line.
        TickRange::new(Tick(starts[3] + 2_000), Tick(starts[3] + 2_400)),
        TickRange::new(Tick(starts[4] + 1_050), Tick(starts[4] + 1_070)),
        TickRange::new(Tick(starts[6] - 300), Tick(starts[6] - 100)),
    ];
    assert_block_invariant(|graph| build(Model::Am2302, RATE, graph), &block, &gaps);
}

#[test]
fn progress_never_passes_a_reading_not_yet_reported() {
    let (block, _) = busy_line();
    for sizes in [&[1u64][..], &[257], &[64, 3_001]] {
        let mut harness = build(Model::Am2302, RATE, GraphConfig::default());
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
        assert!(seen > Tick::ZERO, "blocks of {sizes:?} announce progress");
    }
}

#[test]
fn a_faster_source_decodes_alike_and_a_slower_one_is_refused() {
    let bytes = with_checksum([0x01, 0xf4, 0x00, 0xc8]);
    let rate = 24_000_000;
    let mut waveform = Am230xWaveform::new(
        config(Model::Am2302),
        CHANNELS,
        TimeBase::new(rate),
        Tick::ZERO,
    );
    waveform.idle_us(1_000);
    waveform.reading(bytes);
    waveform.idle_us(100);
    let block = waveform.finish();
    let mut harness = build(Model::Am2302, rate, GraphConfig::default());
    harness.run_all(&block);
    let lines = describe(&harness.items);
    assert!(
        lines[0].starts_with("reading 01f400c8bd checksum ok rh 500 t 200 [24000,"),
        "{lines:?}"
    );

    let result = Harness::build(
        CHANNELS,
        TimeBase::new(500_000),
        GraphConfig::default(),
        |graph, source| {
            let sensor =
                graph.add(Am230x::new(config(Model::Am2302)).expect("a valid configuration"));
            graph.connect(source.samples, Am230x::input(sensor))?;
            Ok(Am230x::events(sensor))
        },
    );
    match result {
        Err(GraphError::Requirement { key, .. }) => assert_eq!(key, "rateHz"),
        other => panic!("expected the rate to be refused, got {other:?}"),
    }
}
