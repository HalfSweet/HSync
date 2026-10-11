// SPDX-License-Identifier: Apache-2.0

//! The PS/2 decoder on synthesized waveforms.

use uiharu_core::testing::{Harness, assert_block_invariant, without_progress};
use uiharu_core::{GraphConfig, GraphError, Item, LogicBlock, Tick, TickRange, TimeBase};
use uiharu_protocols::ps2::synth::{Faults, Ps2Waveform, Timing, frame_bits};
use uiharu_protocols::ps2::{Direction, Ps2, Ps2Config, Ps2Event};

const CHANNELS: u8 = 3;
/// 1 MHz: a tick is a microsecond.
const RATE: u64 = 1_000_000;
/// The synthesizer's clock period, in ticks.
const PERIOD: u64 = 80;
/// From where a device frame is laid out to its first clock fall.
const SETUP: u64 = 20;
/// Where a device frame's last fall lies after its first.
const DEVICE_LAST: u64 = 10 * PERIOD;
/// Where a host frame's acknowledge fall lies after its inhibit's fall.
const HOST_LAST: u64 = 1_040;

/// The clock on channel 2 and the data line on channel 1; channel 0 stays low.
fn config() -> Ps2Config {
    Ps2Config { clk: 2, data: 1 }
}

fn build(rate: u64, graph: GraphConfig) -> Harness<Ps2Event> {
    Harness::build(CHANNELS, TimeBase::new(rate), graph, |graph, source| {
        let ps2 = graph.add(Ps2::new(config()).expect("a valid configuration"));
        graph.connect(source.samples, Ps2::input(ps2))?;
        Ok(Ps2::events(ps2))
    })
    .expect("builds")
}

/// A waveform that starts with the bus idle long enough for the decoder to trust it.
fn new_waveform() -> Ps2Waveform {
    let mut waveform = Ps2Waveform::new(config(), CHANNELS, TimeBase::new(RATE), Tick::ZERO);
    waveform.idle_us(500);
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
fn describe(items: &[Item<Ps2Event>]) -> Vec<String> {
    without_progress(items)
        .iter()
        .map(|item| match item {
            Item::Event(event) => {
                let what = match event.payload {
                    Ps2Event::Frame {
                        direction,
                        value,
                        parity_ok,
                        stop_ok,
                        ack,
                    } => {
                        let direction = match direction {
                            Direction::DeviceToHost => "device",
                            Direction::HostToDevice => "host",
                        };
                        let mut what = format!("{direction} {value:02x}");
                        if !parity_ok {
                            what.push_str(" parity-error");
                        }
                        if !stop_ok {
                            what.push_str(" stop-error");
                        }
                        match ack {
                            Some(true) => what.push_str(" ack"),
                            Some(false) => what.push_str(" nack"),
                            None => {}
                        }
                        what
                    }
                    Ps2Event::Error { reason } => format!("error {reason:?}"),
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
    let mut harness = build(RATE, GraphConfig::default());
    harness.run_all(block);
    describe(&harness.items)
}

/// The line of a device frame laid out at `at`.
fn device_line(what: &str, at: u64) -> String {
    let first = at + SETUP;
    let last = first + DEVICE_LAST;
    format!("device {what} [{first},{})@{last}", last + 1)
}

/// The line of a host frame whose request began at `at`.
fn host_line(what: &str, at: u64) -> String {
    let last = at + HOST_LAST;
    format!("host {what} [{at},{})@{last}", last + 1)
}

#[test]
fn device_bytes_are_read_on_the_falling_edges() {
    let mut waveform = new_waveform();
    let first = waveform.position().get();
    // The make code of A, then its break code, back to back.
    waveform.device_byte(0x1c);
    let second = waveform.position().get();
    waveform.device_byte(0xf0);
    waveform.idle_us(300);
    let block = waveform.finish();
    assert_eq!(
        decode(&block),
        vec![
            device_line("1c", first),
            device_line("f0", second),
            format!("eof {}", block.end()),
        ]
    );
}

#[test]
fn a_host_byte_and_its_acknowledge() {
    let mut waveform = new_waveform();
    let request = waveform.position().get();
    // Set LEDs, which the keyboard acknowledges with 0xfa.
    waveform.host_byte(0xed);
    waveform.idle_us(200);
    let answer = waveform.position().get();
    waveform.device_byte(0xfa);
    waveform.idle_us(200);
    let block = waveform.finish();
    assert_eq!(
        decode(&block),
        vec![
            host_line("ed ack", request),
            device_line("fa", answer),
            format!("eof {}", block.end()),
        ]
    );

    // Not acknowledged: the data line stays high on the last pulse.
    let mut waveform = new_waveform();
    let request = waveform.position().get();
    waveform.host_byte_with(0xf4, Faults::NONE, false);
    waveform.idle_us(200);
    assert_eq!(decode(&waveform.finish())[0], host_line("f4 nack", request));

    // No last pulse at all: the frame is reported unacknowledged once the pause has passed.
    let mut waveform = new_waveform();
    let request = waveform.position().get();
    waveform.request_to_send();
    waveform.host_bits(frame_bits(0xff, Faults::NONE));
    waveform.idle_us(3_000);
    let stop = request + 220 + 9 * PERIOD + 40;
    assert_eq!(
        decode(&waveform.finish())[0],
        format!("host ff nack [{request},{})@{}", stop + 1, stop + 2_000)
    );
}

#[test]
fn parity_and_stop_errors_are_reported() {
    let parity = Faults {
        parity: true,
        ..Faults::NONE
    };
    let stop = Faults {
        stop: true,
        ..Faults::NONE
    };
    let mut waveform = new_waveform();
    let first = waveform.position().get();
    waveform.device_byte_with(0x5a, parity);
    waveform.idle_us(300);
    let second = waveform.position().get();
    waveform.device_byte_with(0x5a, stop);
    waveform.idle_us(300);
    let third = waveform.position().get();
    waveform.host_byte_with(0x33, parity, true);
    waveform.idle_us(300);
    let lines = decode(&waveform.finish());
    assert_eq!(
        lines[..3],
        [
            device_line("5a parity-error", first),
            device_line("5a stop-error", second),
            host_line("33 parity-error ack", third),
        ]
    );
}

#[test]
fn a_start_bit_of_one_is_an_error() {
    let mut waveform = new_waveform();
    let first = waveform.position().get();
    waveform.device_byte_with(
        0x12,
        Faults {
            start: true,
            ..Faults::NONE
        },
    );
    waveform.idle_us(300);
    let next = waveform.position().get();
    waveform.device_byte(0x12);
    waveform.idle_us(300);
    let lines = decode(&waveform.finish());
    let fall = first + SETUP;
    // The rest of the broken frame is skipped until the bus idles.
    assert_eq!(
        lines[..2],
        [
            format!("error Start [{fall},{})@{}", fall + 41, fall + 40),
            device_line("12", next),
        ]
    );
    assert_eq!(lines.len(), 3, "{lines:?}");
}

#[test]
fn the_host_inhibiting_mid_frame_aborts_it() {
    // The device has clocked its start bit and four data bits when the host takes the clock to
    // send a command of its own.
    let mut waveform = new_waveform();
    let first = waveform.position().get();
    waveform.device_bits([false, true, true, false, true]);
    let request = waveform.position().get();
    waveform.host_byte(0xff);
    waveform.idle_us(300);
    // Then a plain inhibit, after which the device sends its byte again.
    let again = waveform.position().get();
    waveform.device_bits([false, true, true]);
    let inhibit = waveform.position().get();
    waveform.inhibit_us(250);
    waveform.idle_us(200);
    let retry = waveform.position().get();
    waveform.device_byte(0xaa);
    waveform.idle_us(300);
    let lines = decode(&waveform.finish());
    assert_eq!(
        lines[..4],
        [
            format!(
                "error Aborted [{},{})@{}",
                first + SETUP,
                request + 1,
                request + 99
            ),
            host_line("ff ack", request),
            format!(
                "error Aborted [{},{})@{}",
                again + SETUP,
                inhibit + 1,
                inhibit + 99
            ),
            device_line("aa", retry),
        ]
    );
}

#[test]
fn a_clock_pause_of_2_ms_aborts_the_frame() {
    let mut waveform = new_waveform();
    let first = waveform.position().get();
    waveform.device_bits([false, false, true, true, false, true]);
    waveform.idle_us(2_500);
    let next = waveform.position().get();
    waveform.device_byte(0x29);
    waveform.idle_us(300);
    let lines = decode(&waveform.finish());
    let last_rise = first + 5 * PERIOD + 60;
    assert_eq!(
        lines[..2],
        [
            format!(
                "error Aborted [{},{})@{}",
                first + SETUP,
                last_rise + 1,
                last_rise + 2_000
            ),
            device_line("29", next),
        ]
    );

    // The device has 15 ms to begin clocking after a request to send.
    let mut waveform = new_waveform();
    let request = waveform.position().get();
    let timing = Timing {
        first_clock: 20_000_000,
        ..Timing::NOMINAL
    };
    waveform.set_timing(timing);
    waveform.request_to_send();
    let release = request + 120;
    let lines = decode(&waveform.finish());
    assert_eq!(
        lines[0],
        format!(
            "error Aborted [{request},{})@{}",
            release + 1,
            release + 15_000
        )
    );
    let mut waveform = new_waveform();
    let request = waveform.position().get();
    waveform.set_timing(Timing {
        first_clock: 14_000_000,
        ..Timing::NOMINAL
    });
    waveform.host_byte(0xf2);
    waveform.idle_us(300);
    let lines = decode(&waveform.finish());
    assert!(
        lines[0].starts_with(&format!("host f2 ack [{request},")),
        "{lines:?}"
    );
}

#[test]
fn frames_cut_off_by_the_end_or_a_gap_are_aborted() {
    let mut waveform = new_waveform();
    let first = waveform.position().get();
    waveform.device_byte(0x76);
    waveform.idle_us(400);
    let next = waveform.position().get();
    waveform.device_byte(0x77);
    waveform.idle_us(400);
    let block = waveform.finish();

    // The samples end in the middle of the first frame.
    let cut = Tick(first + 4 * PERIOD + 70);
    let head = block
        .slice(TickRange::new(Tick::ZERO, cut))
        .expect("a slice");
    assert_eq!(
        decode(&head),
        vec![
            format!(
                "error Aborted [{},{})@{cut}",
                first + SETUP,
                first + 4 * PERIOD + 61
            ),
            format!("eof {cut}"),
        ]
    );

    // A gap swallows bits 3 to 6; the rest of the frame is skipped, and the next frame comes
    // after the bus has idled.
    let gap = TickRange::new(Tick(first + 3 * PERIOD + 10), Tick(first + 6 * PERIOD + 30));
    let mut harness = build(RATE, GraphConfig::default());
    harness.feed_with_gaps(&block, &[gap], &[97]);
    harness.end();
    let lines = describe(&harness.items);
    assert_eq!(
        lines,
        vec![
            format!(
                "error Aborted [{},{})@{}",
                first + SETUP,
                first + 2 * PERIOD + 61,
                gap.start
            ),
            format!("gap {gap}"),
            device_line("77", next),
            format!("eof {}", block.end()),
        ]
    );
}

#[test]
fn a_frame_under_way_at_the_start_is_skipped() {
    let mut waveform = new_waveform();
    let first = waveform.position().get();
    waveform.device_byte(0x1b);
    waveform.idle_us(300);
    let next = waveform.position().get();
    waveform.device_byte(0x1c);
    waveform.idle_us(300);
    let block = waveform.finish();
    // 0x1b goes out as 0, 1101 1000, 1, 1. Start in bit 3 with the clock low, in bit 5 with both
    // lines high, and in bit 3 with the data line low.
    for from in [
        first + 3 * PERIOD + 30,
        first + 5 * PERIOD + 70,
        first + 3 * PERIOD + 70,
    ] {
        let tail = block
            .slice(TickRange::new(Tick(from), block.end()))
            .expect("a slice");
        assert_eq!(
            decode(&tail),
            vec![device_line("1c", next), format!("eof {}", block.end())],
            "from {from}"
        );
    }
    // A host request whose inhibit was under way: its release is no request the decoder saw.
    let mut waveform = new_waveform();
    let request = waveform.position().get();
    waveform.host_byte(0xee);
    waveform.idle_us(300);
    let echo = waveform.position().get();
    waveform.device_byte(0xee);
    waveform.idle_us(300);
    let block = waveform.finish();
    let tail = block
        .slice(TickRange::new(Tick(request + 50), block.end()))
        .expect("a slice");
    assert_eq!(decode(&tail)[0], device_line("ee", echo));
}

/// Traffic of every kind, and where each piece begins.
fn busy_bus() -> (LogicBlock, Vec<u64>) {
    let mut waveform = new_waveform();
    let mut starts = Vec::new();
    for n in 0..5u8 {
        starts.push(waveform.position().get());
        waveform.host_byte(0xf0 | n);
        waveform.idle_us(150 + u64::from(n));
        starts.push(waveform.position().get());
        waveform.device_byte(0xfa);
        waveform.device_byte(n.wrapping_mul(37));
        waveform.idle_us(120);
        starts.push(waveform.position().get());
        match n {
            0 => waveform.device_byte_with(
                0x41,
                Faults {
                    start: true,
                    ..Faults::NONE
                },
            ),
            1 => waveform.device_byte_with(
                0x42,
                Faults {
                    parity: true,
                    stop: true,
                    ..Faults::NONE
                },
            ),
            2 => {
                waveform.device_bits([false, true, false]);
                waveform.inhibit_us(300);
            }
            3 => waveform.host_byte_with(0x44, Faults::NONE, false),
            _ => {
                waveform.device_bits([false, true, true, true]);
                waveform.idle_us(2_200);
            }
        }
        waveform.idle_us(400);
    }
    (waveform.finish(), starts)
}

#[test]
fn every_cut_and_pause_yields_the_same_events() {
    let (block, starts) = busy_bus();
    let reference = decode(&block);
    for kind in [
        "host ",
        "device ",
        "error Start",
        "error Aborted",
        " stop-error",
        " nack",
    ] {
        assert!(
            reference.iter().any(|line| line.contains(kind)),
            "{kind} in {reference:?}"
        );
    }
    let gaps = [
        // Inside a host frame, inside a device frame, and on an idle bus.
        TickRange::new(Tick(starts[3] + 500), Tick(starts[3] + 700)),
        TickRange::new(Tick(starts[7] + 300), Tick(starts[7] + 345)),
        TickRange::new(Tick(starts[12] - 100), Tick(starts[12] - 60)),
    ];
    assert_block_invariant(|graph| build(RATE, graph), &block, &gaps);
}

#[test]
fn progress_never_passes_a_frame_not_yet_reported() {
    let (block, _) = busy_bus();
    for sizes in [&[1u64][..], &[173], &[64, 1_001]] {
        let mut harness = build(RATE, GraphConfig::default());
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
fn the_slowest_rate_and_clock_still_decode() {
    // 100 kHz, the least the decoder takes, and a 10 kHz clock: 5 samples per half period.
    let rate = 100_000;
    let mut waveform = Ps2Waveform::new(config(), CHANNELS, TimeBase::new(rate), Tick::ZERO);
    waveform.set_timing(Timing {
        half_period: 50_000,
        setup: 20_000,
        ..Timing::NOMINAL
    });
    waveform.idle_us(500);
    waveform.device_byte(0x5a);
    waveform.idle_us(300);
    waveform.host_byte(0xa5);
    waveform.idle_us(300);
    let block = waveform.finish();
    let mut harness = build(rate, GraphConfig::default());
    harness.run_all(&block);
    let lines = describe(&harness.items);
    assert!(lines[0].starts_with("device 5a ["), "{lines:?}");
    assert!(lines[1].starts_with("host a5 ack ["), "{lines:?}");
    assert_eq!(lines.len(), 3, "{lines:?}");
}

#[test]
fn a_source_too_slow_for_the_clock_is_refused() {
    let result = Harness::build(
        CHANNELS,
        TimeBase::new(50_000),
        GraphConfig::default(),
        |graph, source| {
            let ps2 = graph.add(Ps2::new(config()).expect("a valid configuration"));
            graph.connect(source.samples, Ps2::input(ps2))?;
            Ok(Ps2::events(ps2))
        },
    );
    match result {
        Err(GraphError::Requirement { key, .. }) => assert_eq!(key, "rateHz"),
        other => panic!("expected the rate to be refused, got {other:?}"),
    }
}
