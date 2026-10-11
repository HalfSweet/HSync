// SPDX-License-Identifier: Apache-2.0

//! The DMX512 decoder on synthesized UART waveforms.

use std::collections::BTreeMap;

use proptest::prelude::*;
use uiharu_core::testing::{Harness, assert_block_invariant, without_progress};
use uiharu_core::{GraphConfig, GraphError, Item, LogicBlock, Tick, TickRange, TimeBase};
use uiharu_protocols::dmx512::synth::{PacketTiming, packet};
use uiharu_protocols::dmx512::{Dmx512, Dmx512Event, EndReason, SLOT_BATCH};
use uiharu_protocols::uart::synth::{Faults, UartWaveform};
use uiharu_protocols::uart::{Parity, StopBits, Uart, UartConfig};

const CHANNELS: u8 = 1;
/// 1 MHz: a bit of 4 µs is four ticks, a character of eleven bits 44.
const RATE: u64 = 1_000_000;
const CHAR: u64 = 44;
/// Where the UART settles a character: its second stop bit's sample point.
const SETTLED: u64 = 42;
const BREAK: u64 = 100;
const MAB: u64 = 12;

fn timing() -> PacketTiming {
    PacketTiming {
        break_samples: BREAK,
        mark_after_break: MAB,
        between_slots: 0,
    }
}

/// Source → UART → DMX512, the packets being the output.
fn build(uart: UartConfig, graph_config: GraphConfig) -> Result<Harness<Dmx512Event>, GraphError> {
    Harness::build(
        CHANNELS,
        TimeBase::new(RATE),
        graph_config,
        |graph, source| {
            let uart = graph.add(Uart::new(uart).expect("a valid configuration"));
            graph.connect(source.samples, Uart::input(uart))?;
            let dmx = graph.add(Dmx512::new());
            graph.connect(Uart::events(uart), Dmx512::input(dmx))?;
            Ok(Dmx512::events(dmx))
        },
    )
}

fn harness(graph_config: GraphConfig) -> Harness<Dmx512Event> {
    build(Dmx512::uart_config(0), graph_config).expect("builds")
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// One line per item, readable in assertions; every span is written out, empty or not.
fn describe(items: &[Item<Dmx512Event>]) -> Vec<String> {
    items
        .iter()
        .map(|item| match item {
            Item::Event(event) => {
                let what = match &event.payload {
                    Dmx512Event::PacketBegin {
                        packet,
                        break_ticks,
                    } => format!("packet-begin p{} {break_ticks}", packet.0),
                    Dmx512Event::StartCode { packet, code } => {
                        format!("start-code p{} {code:02x}", packet.0)
                    }
                    Dmx512Event::Slots {
                        packet,
                        offset,
                        values,
                    } => format!("slots p{} +{offset} {}", packet.0, hex(values)),
                    Dmx512Event::PacketEnd {
                        packet,
                        slots,
                        reason,
                    } => format!("packet-end p{} {slots} {reason:?}", packet.0),
                    Dmx512Event::Error { packet, slot } => {
                        format!("error p{} slot {slot}", packet.0)
                    }
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

/// Decodes `block` less `gaps` in one piece: the lines, progress left out.
fn decode_with(block: &LogicBlock, gaps: &[TickRange]) -> Vec<String> {
    let mut harness = harness(GraphConfig::default());
    harness.feed_with_gaps(block, gaps, &[u64::MAX]);
    harness.end();
    describe(&harness.without_progress())
}

fn decode(block: &LogicBlock) -> Vec<String> {
    decode_with(block, &[])
}

/// A DMX512 line idle (at mark) until `idle`.
fn waveform(idle: u64) -> UartWaveform {
    let mut waveform = UartWaveform::new(Dmx512::uart_config(0), CHANNELS, 4, Tick::ZERO);
    waveform.idle(idle);
    waveform
}

/// Where character `k` of the packet whose break begins at `at` starts, the start code being
/// character 0, sent with [`timing`].
fn char_at(at: u64, k: u64) -> u64 {
    at + BREAK + MAB + k * CHAR
}

/// The span and settled tick of characters `first..=last` of that packet.
fn chars(at: u64, first: u64, last: u64) -> String {
    format!(
        "[{},{})@{}",
        char_at(at, first),
        char_at(at, last) + CHAR,
        char_at(at, last) + SETTLED
    )
}

fn begin(id: u64, at: u64) -> String {
    format!(
        "packet-begin p{id} {BREAK} [{at},{})@{}",
        at + BREAK,
        at + BREAK
    )
}

fn values(count: usize, seed: u8) -> Vec<u8> {
    (0..count)
        .map(|i| (i as u8).wrapping_mul(29).wrapping_add(seed))
        .collect()
}

#[test]
fn a_full_packet_goes_out_in_batches_of_32() {
    let data = values(512, 3);
    let mut waveform = waveform(200);
    packet(&mut waveform, timing(), 0x00, &data);
    waveform.idle(50);
    let second = waveform.position().0;
    packet(&mut waveform, timing(), 0x00, &[1, 2, 3]);
    waveform.idle(100);
    let block = waveform.finish();
    let end = block.end().0;
    let mut expected = vec![
        begin(1, 200),
        format!("start-code p1 00 {}", chars(200, 0, 0)),
    ];
    for batch in 0..16u64 {
        let from = usize::try_from(32 * batch).expect("small");
        expected.push(format!(
            "slots p1 +{} {} {}",
            32 * batch,
            hex(&data[from..from + 32]),
            chars(200, 1 + 32 * batch, 32 + 32 * batch)
        ));
    }
    // The next break ends the packet where it begins, and is settled with it.
    expected.push(format!(
        "packet-end p1 512 Break [{second},{second})@{}",
        second + BREAK
    ));
    expected.extend([
        begin(2, second),
        format!("start-code p2 00 {}", chars(second, 0, 0)),
        // The rest of a batch goes out with the end that decides it.
        format!(
            "slots p2 +0 010203 [{},{})@{end}",
            char_at(second, 1),
            char_at(second, 3) + CHAR
        ),
        format!("packet-end p2 3 Aborted [{end},{end})@{end}"),
        format!("eof {end}"),
    ]);
    assert_eq!(decode(&block), expected);
}

#[test]
fn breaks_end_packets_however_short() {
    let mut waveform = waveform(200);
    let mut starts = Vec::new();
    for slots in [&[10u8, 20, 30, 40, 50][..], &[7, 8], &[]] {
        starts.push(waveform.position().0);
        packet(&mut waveform, timing(), 0x00, slots);
        waveform.idle(30);
    }
    // A break right after the mark after break: a packet without a start code.
    starts.push(waveform.position().0);
    waveform.break_(BREAK);
    waveform.idle(MAB);
    starts.push(waveform.position().0);
    packet(&mut waveform, timing(), 0x00, &[0xff]);
    waveform.idle(20);
    let block = waveform.finish();
    let end = block.end().0;
    let &[a, b, c, d, e] = starts.as_slice() else {
        panic!("five packets")
    };
    assert_eq!(
        decode(&block),
        vec![
            begin(1, a),
            format!("start-code p1 00 {}", chars(a, 0, 0)),
            format!(
                "slots p1 +0 0a141e2832 [{},{})@{}",
                char_at(a, 1),
                char_at(a, 5) + CHAR,
                b + BREAK
            ),
            format!("packet-end p1 5 Break [{b},{b})@{}", b + BREAK),
            begin(2, b),
            format!("start-code p2 00 {}", chars(b, 0, 0)),
            format!(
                "slots p2 +0 0708 [{},{})@{}",
                char_at(b, 1),
                char_at(b, 2) + CHAR,
                c + BREAK
            ),
            format!("packet-end p2 2 Break [{c},{c})@{}", c + BREAK),
            begin(3, c),
            format!("start-code p3 00 {}", chars(c, 0, 0)),
            format!("packet-end p3 0 Break [{d},{d})@{}", d + BREAK),
            begin(4, d),
            format!("packet-end p4 0 Break [{e},{e})@{}", e + BREAK),
            begin(5, e),
            format!("start-code p5 00 {}", chars(e, 0, 0)),
            format!(
                "slots p5 +0 ff [{},{})@{end}",
                char_at(e, 1),
                char_at(e, 1) + CHAR
            ),
            format!("packet-end p5 1 Aborted [{end},{end})@{end}"),
            format!("eof {end}"),
        ]
    );
}

#[test]
fn start_codes_are_reported_as_they_are() {
    let mut waveform = waveform(200);
    packet(&mut waveform, timing(), 0xcc, &[0x01, 0x10]);
    waveform.idle(30);
    packet(&mut waveform, timing(), 0x17, b"hi");
    waveform.idle(30);
    let lines = decode(&waveform.finish());
    assert_eq!(lines[1], format!("start-code p1 cc {}", chars(200, 0, 0)));
    assert!(lines[2].starts_with("slots p1 +0 0110 "), "{lines:?}");
    assert!(lines[5].starts_with("start-code p2 17 "), "{lines:?}");
    assert!(lines[6].starts_with("slots p2 +0 6869 "), "{lines:?}");
}

#[test]
fn a_framing_error_is_flagged_and_its_value_kept() {
    let mut waveform = waveform(200);
    waveform.break_(BREAK);
    waveform.idle(MAB);
    // The start code with its stop bits at space; the line returns to mark after it.
    waveform.frame(0x02, Faults::FRAMING);
    waveform.idle(8);
    let after_start = waveform.position().0;
    waveform.bytes([0x11, 0x22]);
    waveform.frame(0x42, Faults::FRAMING);
    waveform.idle(8);
    let after_error = waveform.position().0;
    waveform.bytes([0x33]);
    waveform.idle(20);
    let block = waveform.finish();
    let end = block.end().0;
    let start = 200 + BREAK + MAB;
    let bad = after_start + 2 * CHAR;
    assert_eq!(
        decode(&block),
        vec![
            begin(1, 200),
            format!(
                "start-code p1 02 [{start},{})@{}",
                start + CHAR,
                start + SETTLED
            ),
            format!(
                "error p1 slot 0 [{start},{})@{}",
                start + CHAR,
                start + SETTLED
            ),
            // The batch that holds the bad slot goes out with it, then the error names it.
            format!(
                "slots p1 +0 112242 [{after_start},{})@{}",
                bad + CHAR,
                bad + SETTLED
            ),
            format!("error p1 slot 3 [{bad},{})@{}", bad + CHAR, bad + SETTLED),
            format!(
                "slots p1 +3 33 [{after_error},{})@{end}",
                after_error + CHAR
            ),
            format!("packet-end p1 4 Aborted [{end},{end})@{end}"),
            format!("eof {end}"),
        ]
    );
}

#[test]
fn a_short_break_still_begins_a_packet() {
    // 60 µs: shorter than E1.11 asks for, longer than a character, so the UART calls it a
    // break; the packet shows how long it was.
    let mut waveform = waveform(200);
    let short = PacketTiming {
        break_samples: 60,
        ..timing()
    };
    packet(&mut waveform, short, 0x00, &[9]);
    waveform.idle(30);
    let lines = decode(&waveform.finish());
    assert_eq!(lines[0], "packet-begin p1 60 [200,260)@260");
    assert!(lines[1].starts_with("start-code p1 00 [272,"), "{lines:?}");
}

#[test]
fn the_end_of_the_input_aborts_the_packet() {
    let data = values(40, 11);
    let mut waveform = waveform(200);
    packet(&mut waveform, timing(), 0x00, &data);
    waveform.idle(100);
    let block = waveform.finish();
    // Cut in the middle of slot 38 (character 38): it is not reported.
    let cut = char_at(200, 38) + 20;
    let head = block
        .slice(TickRange::new(Tick::ZERO, Tick(cut)))
        .expect("a slice");
    assert_eq!(
        decode(&head),
        vec![
            begin(1, 200),
            format!("start-code p1 00 {}", chars(200, 0, 0)),
            format!("slots p1 +0 {} {}", hex(&data[..32]), chars(200, 1, 32)),
            format!(
                "slots p1 +32 {} [{},{})@{cut}",
                hex(&data[32..37]),
                char_at(200, 33),
                char_at(200, 37) + CHAR
            ),
            format!("packet-end p1 37 Aborted [{cut},{cut})@{cut}"),
            format!("eof {cut}"),
        ]
    );
}

#[test]
fn a_gap_aborts_the_packet_and_the_next_break_resynchronizes() {
    let first = values(10, 1);
    let second = values(4, 2);
    let mut waveform = waveform(200);
    packet(&mut waveform, timing(), 0x00, &first);
    waveform.idle(40);
    let next = waveform.position().0;
    packet(&mut waveform, timing(), 0x00, &second);
    waveform.idle(40);
    let block = waveform.finish();
    // Inside slot 6: the slots after it cannot be told from a start code, so they are dropped
    // until the next break.
    let gap = TickRange::new(Tick(char_at(200, 6) + 10), Tick(char_at(200, 6) + 30));
    let lines = decode_with(&block, &[gap]);
    assert_eq!(
        lines,
        vec![
            begin(1, 200),
            format!("start-code p1 00 {}", chars(200, 0, 0)),
            format!(
                "slots p1 +0 {} [{},{})@{}",
                hex(&first[..5]),
                char_at(200, 1),
                char_at(200, 5) + CHAR,
                gap.start
            ),
            format!("packet-end p1 5 Aborted [{0},{0})@{0}", gap.start),
            format!("gap {gap}"),
            begin(2, next),
            format!("start-code p2 00 {}", chars(next, 0, 0)),
            format!(
                "slots p2 +0 {} [{},{})@{}",
                hex(&second),
                char_at(next, 1),
                char_at(next, 4) + CHAR,
                block.end()
            ),
            format!("packet-end p2 4 Aborted [{0},{0})@{0}", block.end()),
            format!("eof {}", block.end()),
        ]
    );
}

#[test]
fn characters_before_the_first_break_belong_to_no_packet() {
    let mut waveform = waveform(100);
    waveform.bytes([0x00, 0x01, 0x02]);
    waveform.idle(30);
    let at = waveform.position().0;
    packet(&mut waveform, timing(), 0x00, &[0x05]);
    waveform.idle(30);
    let lines = decode(&waveform.finish());
    assert_eq!(lines[0], begin(1, at));
    assert_eq!(lines.len(), 5, "{lines:?}");
}

#[test]
fn a_full_batch_shows_up_with_its_last_slot() {
    let data = values(40, 5);
    let mut waveform = waveform(200);
    packet(&mut waveform, timing(), 0x00, &data);
    waveform.idle(100);
    let block = waveform.finish();
    let settled = char_at(200, 32) + SETTLED;
    let mut harness = harness(GraphConfig::default());
    let mut from = Tick::ZERO;
    for (cut, seen) in [(settled, 2), (settled + 1, 3)] {
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
fn the_uart_stream_must_carry_the_dmx512_character_format() {
    let dmx = Dmx512::uart_config(0);
    let error = build(
        UartConfig {
            baud: 115_200,
            ..dmx
        },
        GraphConfig::default(),
    )
    .expect_err("refused");
    assert_eq!(
        error,
        GraphError::Requirement {
            from: "uart#0.events".into(),
            to: "dmx512#1.uart".into(),
            key: "baud",
            requirement: "= 250000".into(),
            found: "115200".into(),
        }
    );
    for (config, key, found) in [
        (
            UartConfig {
                parity: Parity::Even,
                ..dmx
            },
            "parity",
            "\"even\"",
        ),
        (
            UartConfig {
                stop_bits: StopBits::One,
                ..dmx
            },
            "stopBits",
            "\"one\"",
        ),
        (
            UartConfig {
                data_bits: 9,
                ..dmx
            },
            "dataBits",
            "9",
        ),
    ] {
        match build(config, GraphConfig::default()).err() {
            Some(GraphError::Requirement {
                key: refused,
                found: declared,
                ..
            }) => {
                assert_eq!((refused, declared.as_str()), (key, found));
            }
            other => panic!("{key}: {other:?}"),
        }
    }
    assert!(build(dmx, GraphConfig::default()).is_ok());
}

/// Packets of every kind: data before the first break, packets of all sizes, start codes,
/// framing errors, short and long breaks, marks between slots. Returns where each break
/// begins.
fn busy_line(waveform: &mut UartWaveform) -> Vec<u64> {
    waveform.bytes([0x55, 0xaa]);
    waveform.idle(25);
    let mut breaks = Vec::new();
    for n in 0..9u8 {
        breaks.push(waveform.position().0);
        let timing = PacketTiming {
            break_samples: [100, 60, 176, 1000][usize::from(n % 4)],
            mark_after_break: [12, 8, 40][usize::from(n % 3)],
            between_slots: [0, 0, 3, 17][usize::from(n % 4)],
        };
        let data = values(usize::from(n) * 7 % 45, n);
        let start_code = if n == 4 { 0xcc } else { 0x00 };
        if n % 3 == 2 {
            // A slot with a framing error in the middle.
            let half = data.len() / 2;
            packet(waveform, timing, start_code, &data[..half]);
            waveform.frame(0x81, Faults::FRAMING);
            waveform.idle(6);
            waveform.bytes(data[half..].iter().copied());
        } else {
            packet(waveform, timing, start_code, &data);
        }
        waveform.idle(u64::from(n) * 13 % 50);
    }
    waveform.idle(60);
    breaks
}

#[test]
fn blocks_pauses_and_gaps_do_not_change_the_packets() {
    let mut waveform = waveform(150);
    let breaks = busy_line(&mut waveform);
    let block = waveform.finish();
    // Inside packet 2's slots, inside packet 5's break, and in the mark after packet 7's.
    let gaps = [
        TickRange::new(
            Tick(char_at(breaks[1], 3) + 5),
            Tick(char_at(breaks[1], 3) + 60),
        ),
        TickRange::new(Tick(breaks[4] + 20), Tick(breaks[4] + 40)),
        TickRange::new(Tick(breaks[6] + 180), Tick(breaks[6] + 183)),
    ];
    let reference = decode_with(&block, &gaps);
    for wanted in [" Aborted ", " Break ", "error p", "start-code p", "slots p"] {
        assert!(
            reference.iter().any(|line| line.contains(wanted)),
            "{wanted} in {reference:?}"
        );
    }
    assert_block_invariant(harness, &block, &gaps);
}

#[test]
fn progress_never_passes_a_slot_not_yet_reported() {
    let mut waveform = waveform(150);
    busy_line(&mut waveform);
    let block = waveform.finish();
    for sizes in [&[1u64][..], &[13], &[64, 7], &[500]] {
        let mut harness = harness(GraphConfig::default());
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

/// One packet of a random scenario.
#[derive(Clone, Debug)]
struct Scenario {
    break_samples: u64,
    mark_after_break: u64,
    start_code: u8,
    slots: Vec<u8>,
    /// A slot sent with a framing error: after which slot, and its value.
    bad: Option<(usize, u8)>,
    idle: u64,
}

fn scenario() -> impl Strategy<Value = Scenario> {
    (
        prop_oneof![3 => 88u64..200, 1 => 45u64..88, 1 => 200u64..3000],
        prop_oneof![3 => 8u64..20, 1 => 1u64..8],
        prop_oneof![4 => Just(0u8), 1 => any::<u8>()],
        prop::collection::vec(any::<u8>(), 0..80),
        prop::option::weighted(0.2, (0usize..80, 1u8..=255)),
        0u64..300,
    )
        .prop_map(
            |(break_samples, mark_after_break, start_code, slots, bad, idle)| Scenario {
                break_samples,
                mark_after_break,
                start_code,
                slots,
                bad,
                idle,
            },
        )
}

fn scenario_waveform(scenarios: &[Scenario]) -> LogicBlock {
    let mut waveform = waveform(97);
    for scenario in scenarios {
        let timing = PacketTiming {
            break_samples: scenario.break_samples,
            mark_after_break: scenario.mark_after_break,
            between_slots: 0,
        };
        match scenario.bad {
            Some((after, value)) if after < scenario.slots.len() => {
                packet(
                    &mut waveform,
                    timing,
                    scenario.start_code,
                    &scenario.slots[..after],
                );
                waveform.frame(u16::from(value), Faults::FRAMING);
                waveform.idle(5);
                waveform.bytes(scenario.slots[after..].iter().copied());
            }
            _ => packet(&mut waveform, timing, scenario.start_code, &scenario.slots),
        }
        waveform.idle(scenario.idle);
    }
    waveform.idle(10);
    waveform.finish()
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 40, ..ProptestConfig::default() })]

    /// Blocks of any size, a reader that pauses and tiny queues and budgets all yield the
    /// packets of one uninterrupted run; every packet that begins ends once, its slots in
    /// order and in batches no larger than the bound.
    #[test]
    fn packets_are_the_same_however_the_samples_come(
        scenarios in prop::collection::vec(scenario(), 1..6),
        sizes in prop::collection::vec(1u64..500, 1..6),
        pauses in prop::collection::vec(0usize..12, 1..8),
        queue_capacity in 1usize..6,
        budget in 1u32..40,
        gap in prop::option::of((0.05f64..0.95, 1u64..400)),
    ) {
        let block = scenario_waveform(&scenarios);
        let gaps: Vec<TickRange> = gap
            .and_then(|(at, len)| {
                let start = ((block.len() as f64) * at) as u64;
                let end = (start + len).min(block.len());
                (start < end).then(|| TickRange::new(Tick(start), Tick(end)))
            })
            .into_iter()
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
        prop_assert_eq!(without_progress(&harness.items), reference.clone());
        let stats = harness.pipeline().stats();
        prop_assert!(stats.within_capacity(), "{:?}", stats);

        // Packet → (whether the start code came, data slots seen).
        let mut open: BTreeMap<u64, (bool, u32)> = BTreeMap::new();
        let mut settled = Tick::ZERO;
        for event in reference.iter().filter_map(Item::event) {
            prop_assert!(event.settled >= settled, "{:?} settled before {:?}", event, settled);
            settled = event.settled;
            let id = event.payload.packet().0;
            match &event.payload {
                Dmx512Event::PacketBegin { .. } => {
                    prop_assert!(open.is_empty(), "two packets open");
                    open.insert(id, (false, 0));
                }
                Dmx512Event::StartCode { .. } => {
                    let state = open.get_mut(&id).expect("an open packet");
                    prop_assert!(!state.0);
                    state.0 = true;
                }
                Dmx512Event::Slots { offset, values, .. } => {
                    let state = open.get_mut(&id).expect("an open packet");
                    prop_assert!(state.0, "slots before the start code");
                    prop_assert_eq!(*offset, state.1);
                    prop_assert!(!values.is_empty() && values.len() <= SLOT_BATCH);
                    state.1 += u32::try_from(values.len()).expect("small");
                }
                Dmx512Event::Error { slot, .. } => {
                    // The bad slot was the last one reported: slot n is data slot n, and the
                    // start code, before any data, is slot 0.
                    let state = open.get(&id).expect("an open packet");
                    prop_assert_eq!(*slot, state.1);
                }
                Dmx512Event::PacketEnd { slots, reason, .. } => {
                    let state = open.remove(&id).expect("an open packet");
                    prop_assert_eq!(*slots, state.1);
                    prop_assert!(matches!(reason, EndReason::Break | EndReason::Aborted));
                }
            }
        }
        prop_assert!(open.is_empty(), "packets left open: {:?}", open);
    }
}
