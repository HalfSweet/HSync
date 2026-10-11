// SPDX-License-Identifier: Apache-2.0

//! The 1-Wire link layer on synthesized waveforms.

use uiharu_core::testing::{self, without_progress};
use uiharu_core::{
    GraphConfig, GraphError, Item, Levels, LogicBlock, LogicBlockBuilder, Tick, TickRange, TimeBase,
};
use uiharu_protocols::onewire::rom::{MATCH_ROM, READ_ROM, SEARCH_ROM};
use uiharu_protocols::onewire::synth::OneWireWaveform;
use uiharu_protocols::onewire::{LinkConfig, LinkEvent, OneWireLink, Speed};

const CHANNELS: u8 = 2;
/// The data line; channel 0 stays low throughout.
const DATA: u8 = 1;
const MHZ: u64 = 1_000_000;

fn config(speed: Speed) -> LinkConfig {
    LinkConfig { data: DATA, speed }
}

fn waveform(speed: Speed, rate: u64) -> OneWireWaveform {
    let mut waveform =
        OneWireWaveform::new(config(speed), CHANNELS, TimeBase::new(rate), Tick::ZERO);
    waveform.idle(100);
    waveform
}

/// Source → link layer.
fn link(speed: Speed, rate: u64, graph: GraphConfig) -> testing::Harness<LinkEvent> {
    testing::Harness::build(CHANNELS, TimeBase::new(rate), graph, |graph, source| {
        let link = graph.add(OneWireLink::new(config(speed)).expect("a valid configuration"));
        graph.connect(source.samples, OneWireLink::input(link))?;
        Ok(OneWireLink::events(link))
    })
    .expect("builds")
}

fn at(span: TickRange, settled: Tick) -> String {
    if span.is_empty() {
        format!("@{settled}")
    } else {
        format!("[{},{})@{settled}", span.start, span.end)
    }
}

/// One line per item, progress left out: it depends on how the samples were cut.
fn link_lines(items: &[Item<LinkEvent>]) -> Vec<String> {
    without_progress(items)
        .iter()
        .map(|item| match item {
            Item::Event(event) => {
                let what = match event.payload {
                    LinkEvent::Reset { presence: true } => "reset+".to_owned(),
                    LinkEvent::Reset { presence: false } => "reset-".to_owned(),
                    LinkEvent::Bit { value } => format!("bit {}", u8::from(value)),
                    LinkEvent::BadSlot { low_ticks } => format!("bad {low_ticks}"),
                };
                format!("{what} {}", at(event.span, event.settled))
            }
            Item::Gap(range) => format!("gap {range}"),
            Item::Progress(tick) => format!("progress {tick}"),
            Item::End(tick) => format!("eof {tick}"),
        })
        .collect()
}

/// The lines without their ticks.
fn what(lines: &[String]) -> Vec<String> {
    lines
        .iter()
        .map(|line| {
            let cut = line.find(" [").or_else(|| line.find(" @"));
            cut.map_or(line.clone(), |cut| line[..cut].to_owned())
        })
        .collect()
}

/// A ROM code with its CRC: the family code, the serial number, then the CRC-8 of both,
/// computed here independently, most significant bit first on reflected bytes.
fn rom_code(family: u8, serial: u64) -> u64 {
    let mut bytes = [0u8; 8];
    bytes[0] = family;
    bytes[1..7].copy_from_slice(&serial.to_le_bytes()[..6]);
    bytes[7] = reflected_crc(&bytes[..7]);
    u64::from_le_bytes(bytes)
}

fn reflected_crc(bytes: &[u8]) -> u8 {
    let mut crc = 0u8;
    for byte in bytes {
        crc ^= byte.reverse_bits();
        for _ in 0..8 {
            crc = if crc & 0x80 != 0 {
                (crc << 1) ^ 0x31
            } else {
                crc << 1
            };
        }
    }
    crc.reverse_bits()
}

#[test]
fn the_tests_crc_is_the_standards() {
    assert_eq!(reflected_crc(b"123456789"), 0xa1);
}

// The link layer.

#[test]
fn slots_are_told_apart_at_the_window_edges() {
    // 10 MHz: 15 µs is 150 ticks, 120 µs 1200 and 480 µs 4800.
    let mut waveform = waveform(Speed::Standard, 10 * MHZ);
    let mut expected = Vec::new();
    let mut fall = 100;
    for (low, line) in [
        (150, "bit 1"),
        (151, "bit 0"),
        (1200, "bit 0"),
        (1201, "bad 1201"),
        (4799, "bad 4799"),
    ] {
        waveform.pulse(low, 1000);
        // A 1 is settled at its sampling point, the rest where the line rose.
        let settled = if line == "bit 1" {
            fall + 150
        } else {
            fall + low
        };
        expected.push(format!("{line} [{fall},{})@{settled}", settled + 1));
        fall += low + 1000;
    }
    waveform.pulse(4800, 8000);
    // No presence pulse: the reset runs through the end of the presence window, 60 µs.
    let window_end = fall + 4800 + 600;
    expected.push(format!("reset- [{fall},{})@{window_end}", window_end + 1));
    expected.push(format!("eof {}", fall + 4800 + 8000));
    let mut harness = link(Speed::Standard, 10 * MHZ, GraphConfig::default());
    harness.run_all(&waveform.finish());
    assert_eq!(link_lines(&harness.items), expected);
}

#[test]
fn overdrive_slots_are_told_apart_at_the_window_edges() {
    // 10 MHz: 2 µs is 20 ticks, 16 µs 160, 48 µs 480, 80 µs 800 and 480 µs 4800.
    let mut waveform = waveform(Speed::Overdrive, 10 * MHZ);
    let mut expected = Vec::new();
    let mut fall = 100;
    for (low, high, line) in [
        (20, 200, "bit 1"),
        (21, 200, "bit 0"),
        (160, 200, "bit 0"),
        (161, 200, "bad 161"),
        (479, 200, "bad 479"),
        (480, 1000, "reset-"),
        (800, 1000, "reset-"),
        (801, 1000, "bad 801"),
        (4799, 1000, "bad 4799"),
        (4800, 8000, "reset-"),
    ] {
        waveform.pulse(low, high);
        let settled = match line {
            "bit 1" => fall + 20,
            // An overdrive reset's presence window closes 6 µs after the release, a
            // standard-speed reset's 60 µs after.
            "reset-" if low < 4800 => fall + low + 60,
            "reset-" => fall + low + 600,
            _ => fall + low,
        };
        expected.push(format!("{line} [{fall},{})@{settled}", settled + 1));
        fall += low + high;
    }
    expected.push(format!("eof {fall}"));
    let mut harness = link(Speed::Overdrive, 10 * MHZ, GraphConfig::default());
    harness.run_all(&waveform.finish());
    assert_eq!(link_lines(&harness.items), expected);
}

/// The link events of a reset of 500 µs released at 5100, followed after `wait` ticks by a
/// low of `low` ticks, at 10 MHz.
fn after_reset(wait: u64, low: u64) -> Vec<String> {
    let mut waveform = waveform(Speed::Standard, 10 * MHZ);
    waveform.low(5000);
    waveform.idle(wait);
    waveform.pulse(low, 8000);
    let mut harness = link(Speed::Standard, 10 * MHZ, GraphConfig::default());
    harness.run_all(&waveform.finish());
    let mut lines = link_lines(&harness.items);
    lines.pop();
    lines
}

#[test]
fn a_presence_pulse_must_fit_its_window() {
    // Released at 5100: the presence pulse may fall 150 to 600 ticks later, must last at
    // least 600 ticks and must be released no later than 3000 ticks after 5100.
    assert_eq!(after_reset(150, 600), vec!["reset+ [100,5851)@5850"]);
    assert_eq!(after_reset(600, 600), vec!["reset+ [100,6301)@6300"]);
    assert_eq!(after_reset(150, 2850), vec!["reset+ [100,8101)@8100"]);
    // Too early: the reset had no presence, and the pulse is no slot.
    assert_eq!(
        after_reset(149, 600),
        vec!["reset- [100,5249)@5249", "bad 600 [5249,5850)@5849"]
    );
    // Too late: the window closed; the pulse is a slot of its own.
    assert_eq!(
        after_reset(601, 600),
        vec!["reset- [100,5701)@5700", "bit 0 [5701,6302)@6301"]
    );
    // Too short.
    assert_eq!(
        after_reset(150, 599),
        vec!["reset- [100,5250)@5849", "bad 599 [5250,5850)@5849"]
    );
    // Too long: known once it outlasts the latest release.
    assert_eq!(
        after_reset(150, 2851),
        vec!["reset- [100,5250)@8100", "bad 2851 [5250,8102)@8101"]
    );
    // Long enough to be a reset of its own, whose presence window then passes.
    assert_eq!(
        after_reset(150, 5000),
        vec!["reset- [100,5250)@8100", "reset- [5250,10851)@10850"]
    );
}

#[test]
fn resets_and_slots_from_the_synthesizer() {
    let mut waveform = waveform(Speed::Standard, MHZ);
    waveform.reset(true);
    waveform.reset(false);
    waveform.bit(true);
    waveform.bit(false);
    waveform.read_bit(true);
    waveform.read_bit(false);
    waveform.bad_slot(200_000);
    waveform.idle(10);
    let mut harness = link(Speed::Standard, MHZ, GraphConfig::default());
    harness.run_all(&waveform.finish());
    let lines = link_lines(&harness.items);
    // The reset falls at 100 and is released at 600; the slave answers 30 µs later for 120 µs.
    assert_eq!(lines[0], "reset+ [100,751)@750");
    // Released at 1580, no answer: the window closes at 1640.
    assert_eq!(lines[1], "reset- [1080,1641)@1640");
    // Four slots of 70 µs from 2060, a bad slot of 200 µs released for one tick, 10 µs idle.
    assert_eq!(
        what(&lines[2..]),
        vec!["bit 1", "bit 0", "bit 1", "bit 0", "bad 200", "eof 2551"]
    );
}

#[test]
fn a_slave_holding_a_zero_past_the_sampling_point_reads_zero() {
    let mut waveform = waveform(Speed::Standard, MHZ);
    // A read-0 the slave releases at 16 µs, one at 30 µs, and a write-1 released at 15 µs.
    waveform.pulse(16, 54);
    waveform.pulse(30, 40);
    waveform.pulse(15, 55);
    let mut harness = link(Speed::Standard, MHZ, GraphConfig::default());
    harness.run_all(&waveform.finish());
    assert_eq!(
        link_lines(&harness.items),
        vec![
            "bit 0 [100,117)@116",
            "bit 0 [170,201)@200",
            "bit 1 [240,256)@255",
            "eof 310"
        ]
    );
}

#[test]
fn a_one_sampled_inside_the_next_pulse_is_a_bad_slot() {
    let mut waveform = waveform(Speed::Standard, 10 * MHZ);
    // Released after 6 µs, the next pulse falls 4 µs later, before the sampling point.
    waveform.pulse(60, 40);
    waveform.pulse(600, 1000);
    let mut harness = link(Speed::Standard, 10 * MHZ, GraphConfig::default());
    harness.run_all(&waveform.finish());
    assert_eq!(
        link_lines(&harness.items),
        vec!["bad 60 [100,200)@200", "bit 0 [200,801)@800", "eof 1800"]
    );
}

#[test]
fn a_low_seen_from_the_first_sample_counts_only_as_a_reset() {
    let starting_low = |low: u64| {
        let mut builder = LogicBlockBuilder::new(Tick::ZERO, CHANNELS);
        builder.push_repeated(Levels(0), low);
        builder.push_repeated(Levels(1 << DATA), 1000);
        builder.push_repeated(Levels(0), 6);
        builder.push_repeated(Levels(1 << DATA), 64);
        let mut harness = link(Speed::Standard, MHZ, GraphConfig::default());
        harness.run_all(&builder.finish().expect("a block"));
        link_lines(&harness.items)
    };
    // What was seen of it is a reset: it is reported from the first sample.
    let end = 480 + 1000 + 70;
    assert_eq!(
        starting_low(480),
        vec![
            "reset- [0,541)@540".to_owned(),
            "bit 1 [1480,1496)@1495".to_owned(),
            format!("eof {end}"),
        ]
    );
    // Too short to tell: dropped.
    assert_eq!(
        what(&starting_low(479)),
        vec!["bit 1".to_owned(), format!("eof {}", end - 1)]
    );
}

#[test]
fn gaps_and_the_end_drop_the_pulses_they_cut() {
    let mut waveform = waveform(Speed::Standard, 10 * MHZ);
    waveform.pulse(600, 1000); // bit 0, falls at 100
    waveform.pulse(600, 1000); // falls at 1700: cut by the gap
    waveform.pulse(60, 940); // bit 1, falls at 3300
    waveform.low(5000); // a reset falling at 4300 ...
    waveform.idle(100); // ... whose presence window the end of the samples cuts
    let block = waveform.finish();
    let gap = TickRange::new(Tick(1800), Tick(1900));
    let mut harness = link(Speed::Standard, 10 * MHZ, GraphConfig::default());
    harness.feed_with_gaps(&block, &[gap], &[777]);
    harness.end();
    assert_eq!(
        link_lines(&harness.items),
        vec![
            "bit 0 [100,701)@700",
            "gap 1800..1900",
            "bit 1 [3300,3451)@3450",
            "eof 9400",
        ]
    );
}

#[test]
fn a_slot_seen_through_its_sampling_point_survives_a_cut_right_after() {
    // The 1 falls at 100 and is sampled at 250: a gap or the end from 251 on leaves it whole.
    let mut waveform = waveform(Speed::Standard, 10 * MHZ);
    waveform.pulse(60, 1000);
    let block = waveform.finish();
    let head = block
        .slice(TickRange::new(Tick::ZERO, Tick(251)))
        .expect("a slice");
    let mut harness = link(Speed::Standard, 10 * MHZ, GraphConfig::default());
    harness.run_all(&head);
    assert_eq!(
        link_lines(&harness.items),
        vec!["bit 1 [100,251)@250", "eof 251"]
    );
    let gap = TickRange::new(Tick(251), Tick(400));
    let mut harness = link(Speed::Standard, 10 * MHZ, GraphConfig::default());
    harness.feed_with_gaps(&block, &[gap], &[100]);
    harness.end();
    assert_eq!(
        link_lines(&harness.items),
        vec!["bit 1 [100,251)@250", "gap 251..400", "eof 1160"]
    );
    // Cut one tick earlier, its sampling point was not seen.
    let head = block
        .slice(TickRange::new(Tick::ZERO, Tick(250)))
        .expect("a slice");
    let mut harness = link(Speed::Standard, 10 * MHZ, GraphConfig::default());
    harness.run_all(&head);
    assert_eq!(link_lines(&harness.items), vec!["eof 250"]);
}

/// Traffic of every kind — resets with and without presence, write and read slots, a search
/// and bad slots — with two gaps: one in the bytes read after a ROM code, one in a ROM code
/// the master sends.
fn rich_traffic(speed: Speed, rate: u64) -> (LogicBlock, [TickRange; 2]) {
    let mut waveform = waveform(speed, rate);
    let slot = waveform.ticks(waveform.timing().slot);
    let device = rom_code(0x28, 0x0000_0612_3456);
    let other = rom_code(0x28, 0x0000_0612_3476);
    let mut gaps = [TickRange::default(); 2];
    waveform.reset(true);
    waveform.byte(SEARCH_ROM);
    waveform.search_rom(&[device, other]);
    for round in 0..2u8 {
        waveform.reset(true);
        waveform.byte(READ_ROM);
        waveform.read_rom(device);
        waveform.byte(0xbe);
        if round == 0 {
            let at = waveform.position().offset(2 * slot + slot / 3);
            gaps[0] = TickRange::new(at, at.offset(slot));
        }
        waveform.read_byte(round.wrapping_mul(31) ^ 0x5a);
        waveform.read_byte(round);
        waveform.read_bit(round == 0);
        waveform.reset(round != 1);
        waveform.byte(MATCH_ROM);
        if round == 1 {
            let at = waveform.position().offset(20 * slot + 7);
            gaps[1] = TickRange::new(at, at.offset(5 * slot));
        }
        waveform.rom(other);
        waveform.byte(0x44);
        // Longer than a slot, shorter than a reset.
        waveform.bad_slot(if speed == Speed::Standard {
            300_000
        } else {
            30_000
        });
        waveform.bit(true);
        waveform.idle(u64::from(round) * 37 + 5);
    }
    waveform.reset(true);
    waveform.idle(50);
    (waveform.finish(), gaps)
}

#[test]
fn link_events_do_not_depend_on_blocks_pauses_or_queues() {
    for (speed, rate) in [(Speed::Standard, MHZ), (Speed::Overdrive, 10 * MHZ)] {
        let (block, gaps) = rich_traffic(speed, rate);
        let mut whole = link(speed, rate, GraphConfig::default());
        whole.feed_with_gaps(&block, &gaps, &[u64::MAX]);
        whole.end();
        let lines = link_lines(&whole.items);
        assert!(lines.len() > 400, "{speed:?}: {}", lines.len());
        assert!(lines.iter().any(|line| line.starts_with("bad ")));
        assert!(lines.iter().any(|line| line.starts_with("reset+")));
        assert!(lines.iter().any(|line| line.starts_with("reset-")));
        testing::assert_block_invariant(|graph| link(speed, rate, graph), &block, &gaps);
    }
}

// The graph.

#[test]
fn the_source_must_meet_the_link_layers_requirements() {
    let build = |speed: Speed, channels: u8, rate: u64| {
        testing::Harness::build(
            channels,
            TimeBase::new(rate),
            GraphConfig::default(),
            |graph, source| {
                let link = graph.add(OneWireLink::new(config(speed)).expect("valid"));
                graph.connect(source.samples, OneWireLink::input(link))?;
                Ok(OneWireLink::events(link))
            },
        )
        .err()
    };
    assert_eq!(build(Speed::Standard, 2, MHZ), None);
    assert_eq!(
        build(Speed::Overdrive, 2, 4 * MHZ),
        Some(GraphError::Requirement {
            from: "logic#0.samples".into(),
            to: "onewire-link#0.logic".into(),
            key: "rateHz",
            requirement: ">= 10000000".into(),
            found: "4000000".into(),
        })
    );
    assert!(matches!(
        build(Speed::Standard, 1, MHZ),
        Some(GraphError::Requirement {
            key: "channels",
            ..
        })
    ));
    assert!(matches!(
        build(Speed::Standard, 2, 500_000),
        Some(GraphError::Requirement { key: "rateHz", .. })
    ));
}
