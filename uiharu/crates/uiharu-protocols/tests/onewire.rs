// SPDX-License-Identifier: Apache-2.0

//! The 1-Wire link and network layers on synthesized waveforms.

use std::sync::Arc;

use proptest::prelude::*;
use uiharu_core::testing::{self, Tap, without_progress};
use uiharu_core::{
    GraphConfig, GraphError, InputRef, Item, Levels, LogicBlock, LogicBlockBuilder, OutputRef,
    SignalItem, Tick, TickRange, TimeBase,
};
use uiharu_protocols::onewire::rom::{
    ALARM_SEARCH, MATCH_ROM, OVERDRIVE_MATCH_ROM, OVERDRIVE_SKIP_ROM, READ_ROM, SEARCH_ROM,
    SKIP_ROM,
};
use uiharu_protocols::onewire::synth::{OneWireWaveform, Slot, search_rom};
use uiharu_protocols::onewire::{
    EndReason, LinkConfig, LinkEvent, OneWireEvent, OneWireLink, OneWireNetwork, Speed,
};
use uiharu_protocols::spi::{Spi, SpiConfig, SpiEvent};

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

/// Source → link layer → network layer; the link events are a tap.
fn network(
    speed: Speed,
    rate: u64,
    graph: GraphConfig,
) -> (testing::Harness<OneWireEvent>, Tap<LinkEvent>) {
    let mut exposed = None;
    let mut harness =
        testing::Harness::build(CHANNELS, TimeBase::new(rate), graph, |graph, source| {
            let link = graph.add(OneWireLink::new(config(speed)).expect("a valid configuration"));
            graph.connect(source.samples, OneWireLink::input(link))?;
            let net = graph.add(OneWireNetwork::new());
            graph.connect(OneWireLink::events(link), OneWireNetwork::input(net))?;
            exposed = Some(graph.expose("link", OneWireLink::events(link))?);
            Ok(OneWireNetwork::events(net))
        })
        .expect("builds");
    let tap = harness.tap(exposed.expect("the link layer is exposed"));
    (harness, tap)
}

fn at(span: TickRange, settled: Tick) -> String {
    if span.is_empty() {
        format!("@{settled}")
    } else {
        format!("[{},{})@{settled}", span.start, span.end)
    }
}

/// The line of one item.
fn link_line(item: &Item<LinkEvent>) -> String {
    match item {
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
    }
}

/// One line per item, progress left out: it depends on how the samples were cut.
fn link_lines(items: &[Item<LinkEvent>]) -> Vec<String> {
    without_progress(items).iter().map(link_line).collect()
}

fn describe(event: &OneWireEvent) -> String {
    match *event {
        OneWireEvent::Begin { tx, presence } => {
            format!("begin tx{}{}", tx, if presence { "+" } else { "-" })
        }
        OneWireEvent::RomCommand { tx, code } => format!("rom-command tx{} {code:02x}", tx),
        OneWireEvent::Rom {
            tx,
            id,
            family,
            crc_ok,
        } => format!("rom tx{} {id:016x} family {family:02x} crc {crc_ok}", tx),
        OneWireEvent::Search {
            tx,
            id,
            crc_ok,
            conflicts,
            unanswered,
        } => format!(
            "search tx{} {id:016x} crc {crc_ok} conflicts {conflicts} unanswered {unanswered}",
            tx
        ),
        OneWireEvent::Byte { tx, value } => format!("byte tx{} {value:02x}", tx),
        OneWireEvent::Bits { tx, value, count } => {
            format!("bits tx{} {value:02x}/{count}", tx)
        }
        OneWireEvent::End { tx, reason } => format!("end tx{} {reason:?}", tx),
    }
}

/// One line per item, progress left out.
fn net_lines(items: &[Item<OneWireEvent>]) -> Vec<String> {
    without_progress(items)
        .iter()
        .map(|item| match item {
            Item::Event(event) => {
                format!(
                    "{} {}",
                    describe(&event.payload),
                    at(event.span, event.settled)
                )
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

#[test]
fn progress_waits_at_the_fall_of_the_pulse_under_way() {
    // The reset falls at 100, is released at 600 and answered from 630 to 750; the 1 falls at
    // 1140 and is sampled at 1155. Blocks of 50 ticks, each read as it comes, end inside both.
    let mut waveform = waveform(Speed::Standard, MHZ);
    waveform.reset(true);
    waveform.idle(60);
    waveform.bit(true);
    let mut harness = link(Speed::Standard, MHZ, GraphConfig::default());
    harness.feed_cut(&waveform.finish(), &[], &[50], &[usize::MAX]);
    harness.end();
    let mut lines: Vec<String> = Vec::new();
    for line in harness.items.iter().map(link_line) {
        // Progress that repeats the line before makes no new line.
        if lines.last() != Some(&line) {
            lines.push(line);
        }
    }
    // Progress follows the blocks up to a fall and waits there until the pulse is reported —
    // the reset with its presence pulse — then follows them again.
    assert_eq!(
        lines,
        [
            "progress 50",
            "progress 100",
            "reset+ [100,751)@750",
            "progress 800",
            "progress 850",
            "progress 900",
            "progress 950",
            "progress 1000",
            "progress 1050",
            "progress 1100",
            "progress 1140",
            "bit 1 [1140,1156)@1155",
            "progress 1200",
            "progress 1210",
            "eof 1210",
        ]
    );
}

#[test]
fn progress_never_passes_a_pulse_under_way_however_it_ends() {
    // A reset already under way at the first sample; then resets released at 600 whose
    // presence window closes every way: answered, unanswered (the pulse comes too late), or by
    // a pulse too early, too short, too long, or long enough to be a reset of its own.
    let mut builder = LogicBlockBuilder::new(Tick::ZERO, CHANNELS);
    builder.push_repeated(Levels(0), 480);
    builder.push_repeated(Levels(1 << DATA), 1000);
    let mut blocks = vec![builder.finish().expect("a block")];
    for (wait, low) in [(15, 60), (14, 60), (61, 60), (15, 59), (15, 286), (15, 500)] {
        let mut waveform = waveform(Speed::Standard, MHZ);
        waveform.low(500);
        waveform.idle(wait);
        waveform.pulse(low, 800);
        blocks.push(waveform.finish());
    }
    for block in &blocks {
        testing::assert_block_invariant(|graph| link(Speed::Standard, MHZ, graph), block, &[]);
    }
}

// The network layer.

#[test]
fn read_rom_and_match_rom_carry_a_rom_code() {
    let id = rom_code(0x28, 0x0000_0612_3456);
    let mut waveform = waveform(Speed::Standard, MHZ);
    waveform.reset(true);
    waveform.byte(READ_ROM);
    waveform.read_rom(id);
    waveform.reset(true);
    waveform.byte(MATCH_ROM);
    waveform.rom(id ^ 1 << 60);
    waveform.byte(0x44);
    waveform.reset(false);
    waveform.idle(10);
    let (mut harness, _) = network(Speed::Standard, MHZ, GraphConfig::default());
    harness.run_all(&waveform.finish());
    let lines = net_lines(&harness.items);
    // A reset is 980 ticks, a slot 70.
    let first = 100 + 980;
    assert_eq!(lines[0], "begin tx1+ [100,751)@750");
    // The command's eight slots: the last is a 0 settled where the line rose.
    assert_eq!(
        lines[1],
        format!(
            "rom-command tx1 33 [{first},{})@{}",
            first + 7 * 70 + 61,
            first + 7 * 70 + 60
        )
    );
    assert_eq!(
        what(&lines[2..]),
        vec![
            format!("rom tx1 {id:016x} family 28 crc true"),
            "end tx1 Reset".to_owned(),
            "begin tx2+".to_owned(),
            "rom-command tx2 55".to_owned(),
            format!("rom tx2 {:016x} family 28 crc false", id ^ 1 << 60),
            "byte tx2 44".to_owned(),
            "end tx2 Reset".to_owned(),
            "begin tx3-".to_owned(),
            "end tx3 Aborted".to_owned(),
            format!(
                "eof {}",
                harness.pipeline().next_tick(harness.source()).expect("fed")
            ),
        ]
    );
    // The ROM code spans its 64 slots, from the first slot after the command, and settles
    // with the last: a read-1 at its sampling point, a read-0 where the slave released it.
    let rom_start = first + 8 * 70;
    let last_slot = rom_start + 63 * 70;
    let settled = last_slot + if id >> 63 == 1 { 15 } else { 30 };
    assert!(
        lines[2].ends_with(&format!("[{rom_start},{})@{settled}", settled + 1)),
        "{}",
        lines[2]
    );
    // The end of a transaction is placed at the reset that ends it, and settles with it.
    let second = rom_start + 64 * 70;
    assert_eq!(lines[3], format!("end tx1 Reset @{}", second + 650));
    assert_eq!(
        lines[4],
        format!("begin tx2+ [{second},{})@{}", second + 651, second + 650)
    );
}

#[test]
fn skip_rom_bytes_and_the_bits_a_transaction_ends_with() {
    let mut waveform = waveform(Speed::Standard, MHZ);
    waveform.reset(true);
    waveform.byte(SKIP_ROM);
    waveform.byte(0xb4);
    waveform.read_bit(false);
    waveform.read_bit(true);
    waveform.read_bit(true);
    waveform.reset(true);
    waveform.idle(10);
    let (mut harness, _) = network(Speed::Standard, MHZ, GraphConfig::default());
    harness.run_all(&waveform.finish());
    let lines = net_lines(&harness.items);
    let bits = 100 + 980 + 16 * 70;
    let reset = bits + 3 * 70;
    assert_eq!(
        what(&lines),
        vec![
            "begin tx1+".to_owned(),
            "rom-command tx1 cc".to_owned(),
            "byte tx1 b4".to_owned(),
            "bits tx1 06/3".to_owned(),
            "end tx1 Reset".to_owned(),
            "begin tx2+".to_owned(),
            "end tx2 Aborted".to_owned(),
            format!("eof {}", reset + 980 + 10),
        ]
    );
    // The bits span their three slots and settle with the reset that ended them.
    assert_eq!(
        lines[3],
        format!(
            "bits tx1 06/3 [{bits},{})@{}",
            bits + 2 * 70 + 16,
            reset + 650
        )
    );
}

#[test]
fn a_search_finds_the_rom_its_directions_chose() {
    let first = rom_code(0x28, 0x0000_0000_1234);
    let second = rom_code(0x28, 0x0000_0000_1236);
    let mut waveform = waveform(Speed::Standard, MHZ);
    waveform.reset(true);
    waveform.byte(SEARCH_ROM);
    let found = waveform.search_rom(&[first, second]);
    waveform.byte(0x44);
    waveform.reset(true);
    waveform.byte(ALARM_SEARCH);
    waveform.search_rom(&[]);
    waveform.reset(false);
    waveform.idle(10);
    let (mut harness, _) = network(Speed::Standard, MHZ, GraphConfig::default());
    harness.run_all(&waveform.finish());
    // The serial numbers first differ in bit 1 of the second byte, where the search takes the
    // device with a 0; their CRCs differ too, in bits that are no longer contested.
    assert_eq!(found, Some(first));
    let lines = net_lines(&harness.items);
    assert_eq!(
        what(&lines),
        vec![
            "begin tx1+".to_owned(),
            "rom-command tx1 f0".to_owned(),
            format!("search tx1 {first:016x} crc true conflicts 1 unanswered 0"),
            "byte tx1 44".to_owned(),
            "end tx1 Reset".to_owned(),
            "begin tx2+".to_owned(),
            "rom-command tx2 ec".to_owned(),
            format!(
                "search tx2 {:016x} crc {} conflicts 0 unanswered 64",
                u64::MAX,
                reflected_crc(&[0xff; 7]) == 0xff
            ),
            "end tx2 Reset".to_owned(),
            "begin tx3-".to_owned(),
            "end tx3 Aborted".to_owned(),
            lines[11].clone(),
        ]
    );
    // The search spans its 192 slots; the last writes the direction of bit 63.
    let start = 100 + 980 + 8 * 70;
    let end = start + 191 * 70 + if first >> 63 == 1 { 16 } else { 61 };
    assert!(
        lines[2].contains(&format!("[{start},{end})")),
        "{}",
        lines[2]
    );
}

#[test]
fn overdrive_rom_commands() {
    let id = rom_code(0x28, 0x0000_0612_3456);
    let mut waveform = waveform(Speed::Overdrive, 10 * MHZ);
    waveform.reset(true);
    waveform.byte(OVERDRIVE_SKIP_ROM);
    waveform.byte(0xbe);
    waveform.reset(true);
    waveform.byte(OVERDRIVE_MATCH_ROM);
    waveform.rom(id);
    waveform.byte(0x44);
    waveform.reset(true);
    waveform.byte(0xa5);
    waveform.byte(0x01);
    waveform.reset(false);
    let (mut harness, _) = network(Speed::Overdrive, 10 * MHZ, GraphConfig::default());
    harness.run_all(&waveform.finish());
    let lines = what(&net_lines(&harness.items));
    assert_eq!(
        lines[..lines.len() - 1],
        [
            "begin tx1+".to_owned(),
            "rom-command tx1 3c".to_owned(),
            "byte tx1 be".to_owned(),
            "end tx1 Reset".to_owned(),
            "begin tx2+".to_owned(),
            "rom-command tx2 69".to_owned(),
            format!("rom tx2 {id:016x} family 28 crc true"),
            "byte tx2 44".to_owned(),
            "end tx2 Reset".to_owned(),
            // An unknown ROM command: what follows is transport bytes.
            "begin tx3+".to_owned(),
            "rom-command tx3 a5".to_owned(),
            "byte tx3 01".to_owned(),
            "end tx3 Reset".to_owned(),
            "begin tx4-".to_owned(),
            "end tx4 Aborted".to_owned(),
        ]
    );
}

#[test]
fn bits_before_the_first_reset_are_skipped() {
    let mut waveform = waveform(Speed::Standard, MHZ);
    waveform.byte(0x55);
    waveform.bit(true);
    waveform.reset(true);
    waveform.byte(SKIP_ROM);
    waveform.reset(true);
    waveform.idle(10);
    let (mut harness, link) = network(Speed::Standard, MHZ, GraphConfig::default());
    harness.run_all(&waveform.finish());
    assert_eq!(
        what(&net_lines(&harness.items))[..4],
        [
            "begin tx1+",
            "rom-command tx1 cc",
            "end tx1 Reset",
            "begin tx2+"
        ]
    );
    // The link layer saw the bits.
    assert_eq!(
        link_lines(&link.items())
            .iter()
            .filter(|line| line.starts_with("bit "))
            .count(),
        17
    );
}

#[test]
fn a_gap_aborts_the_transaction_and_the_next_reset_resyncs() {
    let mut waveform = waveform(Speed::Standard, MHZ);
    waveform.reset(true);
    waveform.byte(SKIP_ROM);
    waveform.byte(0xbe);
    waveform.read_byte(0x50);
    waveform.read_byte(0x05);
    waveform.read_byte(0x4b);
    waveform.reset(true);
    waveform.byte(SKIP_ROM);
    waveform.byte(0x44);
    waveform.reset(true);
    waveform.idle(10);
    let block = waveform.finish();
    // Lose the middle of the second data byte and the start of the third.
    let second_byte = 100 + 980 + 3 * 8 * 70;
    let gap = TickRange::new(Tick(second_byte + 3 * 70 + 2), Tick(second_byte + 9 * 70));
    let (mut harness, _) = network(Speed::Standard, MHZ, GraphConfig::default());
    harness.feed_with_gaps(&block, &[gap], &[123]);
    harness.end();
    let lines = net_lines(&harness.items);
    assert_eq!(
        what(&lines)[..10],
        [
            "begin tx1+".to_owned(),
            "rom-command tx1 cc".to_owned(),
            "byte tx1 be".to_owned(),
            "byte tx1 50".to_owned(),
            // The three bits of the second byte read before the gap.
            "bits tx1 05/3".to_owned(),
            "end tx1 Aborted".to_owned(),
            format!("gap {gap}"),
            "begin tx2+".to_owned(),
            "rom-command tx2 cc".to_owned(),
            "byte tx2 44".to_owned(),
        ]
    );
    assert_eq!(lines[5], format!("end tx1 Aborted @{}", gap.start));
}

#[test]
fn the_end_of_the_samples_aborts_the_transaction() {
    let mut waveform = waveform(Speed::Standard, MHZ);
    waveform.reset(true);
    waveform.byte(MATCH_ROM);
    for n in 0..20 {
        waveform.bit(n % 3 == 0);
    }
    let block = waveform.finish();
    let (mut harness, _) = network(Speed::Standard, MHZ, GraphConfig::default());
    harness.run_all(&block);
    let lines = net_lines(&harness.items);
    // The ROM code was cut short: no ROM, and the end covers its twenty slots.
    let rom_start = 100 + 980 + 8 * 70;
    assert_eq!(
        lines[2..],
        [
            format!(
                "end tx1 Aborted [{rom_start},{})@{}",
                rom_start + 19 * 70 + 61,
                block.end()
            ),
            format!("eof {}", block.end()),
        ]
    );
}

#[test]
fn a_bad_slot_ends_the_transaction() {
    let mut waveform = waveform(Speed::Standard, MHZ);
    waveform.reset(true);
    waveform.byte(SKIP_ROM);
    waveform.byte(0x4e);
    waveform.bit(true);
    waveform.bad_slot(200_000);
    waveform.byte(0x12);
    waveform.reset(true);
    waveform.idle(10);
    let (mut harness, _) = network(Speed::Standard, MHZ, GraphConfig::default());
    harness.run_all(&waveform.finish());
    let lines = net_lines(&harness.items);
    let bad = 100 + 980 + 17 * 70;
    assert_eq!(
        what(&lines)[..6],
        [
            "begin tx1+",
            "rom-command tx1 cc",
            "byte tx1 4e",
            "bits tx1 01/1",
            "end tx1 BadSlot",
            "begin tx2+"
        ]
    );
    // The end covers the bad slot; the bits settle with it.
    assert_eq!(
        lines[4],
        format!("end tx1 BadSlot [{bad},{})@{}", bad + 201, bad + 200)
    );
    assert!(
        lines[3].ends_with(&format!(")@{}", bad + 200)),
        "{}",
        lines[3]
    );
}

#[test]
fn network_events_do_not_depend_on_blocks_pauses_or_queues() {
    for (speed, rate) in [(Speed::Standard, MHZ), (Speed::Overdrive, 10 * MHZ)] {
        let (block, gaps) = rich_traffic(speed, rate);
        let (mut whole, _) = network(speed, rate, GraphConfig::default());
        whole.feed_with_gaps(&block, &gaps, &[u64::MAX]);
        whole.end();
        let lines = what(&net_lines(&whole.items));
        for kind in ["rom ", "search ", "byte ", "bits ", "end tx1 Reset"] {
            assert!(
                lines.iter().any(|line| line.starts_with(kind)),
                "{speed:?} has no {kind}: {lines:?}"
            );
        }
        assert!(lines.iter().any(|line| line.ends_with("BadSlot")));
        assert!(lines.iter().any(|line| line.ends_with("Aborted")));
        testing::assert_block_invariant(|graph| network(speed, rate, graph).0, &block, &gaps);
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

#[test]
fn the_spi_stream_is_refused_by_the_network_layer() {
    // Typed ports keep the SPI output from a 1-Wire input at compile time; a graph built at
    // run time, as the DSL's will be, is checked by schema when it is connected.
    let refused = |add: &dyn Fn(&mut uiharu_core::Graph) -> uiharu_core::NodeId,
                   to: &str,
                   input: &'static str| {
        let mut graph = uiharu_core::Graph::new();
        let source = graph
            .source(uiharu_core::SourceSpec::new("logic", 4, TimeBase::new(MHZ)))
            .expect("a source");
        let spi = graph.add(Spi::new(SpiConfig::default()).expect("valid"));
        graph
            .connect(source.samples, Spi::input(spi))
            .expect("connects");
        let node = add(&mut graph);
        let error = graph
            .connect(
                OutputRef::<SpiEvent>::of_node(spi, 0),
                InputRef::<SpiEvent>::of_node(node, 0),
            )
            .expect_err("refused");
        assert_eq!(
            error,
            GraphError::SchemaMismatch {
                from: "spi#0.events".into(),
                to: to.into(),
                output: uiharu_protocols::spi::SPI_EVENTS,
                input: uiharu_core::SchemaId::new(input, 1),
            }
        );
    };
    refused(
        &|graph| graph.add(OneWireNetwork::new()),
        "onewire#1.link",
        "onewire.link",
    );
}

// Random traffic.

/// One transaction of a random scenario: its ROM level and what follows.
#[derive(Clone, Debug)]
enum RomLevel {
    Read(u64),
    Match(u64),
    Skip,
    Search(Vec<u64>),
    Other(u8),
}

#[derive(Clone, Debug)]
struct Scenario {
    presence: bool,
    rom: RomLevel,
    bytes: Vec<u8>,
    bits: Vec<bool>,
}

fn rom_level() -> impl Strategy<Value = RomLevel> {
    let id = (any::<u8>(), any::<u64>()).prop_map(|(family, serial)| rom_code(family, serial));
    prop_oneof![
        id.clone().prop_map(RomLevel::Read),
        id.clone().prop_map(RomLevel::Match),
        Just(RomLevel::Skip),
        prop::collection::vec(id, 0..4).prop_map(RomLevel::Search),
        any::<u8>()
            .prop_filter("an unknown ROM command", |code| ![
                READ_ROM,
                MATCH_ROM,
                SKIP_ROM,
                SEARCH_ROM,
                ALARM_SEARCH,
                OVERDRIVE_SKIP_ROM,
                OVERDRIVE_MATCH_ROM
            ]
            .contains(code))
            .prop_map(RomLevel::Other),
    ]
}

fn scenario() -> impl Strategy<Value = Scenario> {
    (
        any::<bool>(),
        rom_level(),
        prop::collection::vec(any::<u8>(), 0..4),
        prop::collection::vec(any::<bool>(), 0..8),
    )
        .prop_map(|(presence, rom, bytes, bits)| Scenario {
            presence,
            rom,
            bytes,
            bits,
        })
}

/// The waveform of `scenarios`, a final reset closing the last one, and the network events
/// it must yield, ticks aside.
fn synthesize(scenarios: &[Scenario], speed: Speed, rate: u64) -> (LogicBlock, Vec<String>) {
    let mut waveform = waveform(speed, rate);
    let mut expected = Vec::new();
    for (n, scenario) in scenarios.iter().enumerate() {
        let tx = n as u64 + 1;
        waveform.reset(scenario.presence);
        expected.push(describe(&OneWireEvent::Begin {
            tx,
            presence: scenario.presence,
        }));
        let code = match &scenario.rom {
            RomLevel::Read(_) => READ_ROM,
            RomLevel::Match(_) => MATCH_ROM,
            RomLevel::Skip => SKIP_ROM,
            RomLevel::Search(_) => SEARCH_ROM,
            RomLevel::Other(code) => *code,
        };
        waveform.byte(code);
        expected.push(describe(&OneWireEvent::RomCommand { tx, code }));
        let rom = |id: u64| {
            describe(&OneWireEvent::Rom {
                tx,
                id,
                family: id.to_le_bytes()[0],
                crc_ok: reflected_crc(&id.to_le_bytes()[..7]) == id.to_le_bytes()[7],
            })
        };
        match &scenario.rom {
            RomLevel::Read(id) => {
                waveform.read_rom(*id);
                expected.push(rom(*id));
            }
            RomLevel::Match(id) => {
                waveform.rom(*id);
                expected.push(rom(*id));
            }
            RomLevel::Search(ids) => {
                let (slots, _) = search_rom(ids);
                let mut id = 0u64;
                let (mut conflicts, mut unanswered) = (0, 0);
                for (n, triplet) in slots.chunks(3).enumerate() {
                    match triplet {
                        [Slot::Read(false), Slot::Read(false), _] => conflicts += 1,
                        [Slot::Read(true), Slot::Read(true), _] => unanswered += 1,
                        _ => {}
                    }
                    if triplet[2] == Slot::Write(true) {
                        id |= 1 << n;
                    }
                }
                waveform.slots(slots);
                expected.push(describe(&OneWireEvent::Search {
                    tx,
                    id,
                    crc_ok: reflected_crc(&id.to_le_bytes()[..7]) == id.to_le_bytes()[7],
                    conflicts,
                    unanswered,
                }));
            }
            RomLevel::Skip | RomLevel::Other(_) => {}
        }
        for byte in &scenario.bytes {
            if n % 2 == 0 {
                waveform.byte(*byte);
            } else {
                waveform.read_byte(*byte);
            }
            expected.push(describe(&OneWireEvent::Byte { tx, value: *byte }));
        }
        // Fewer than eight read slots after the bytes: they end the transaction as bits.
        for bit in &scenario.bits {
            waveform.read_bit(*bit);
        }
        if !scenario.bits.is_empty() {
            let value = scenario
                .bits
                .iter()
                .enumerate()
                .fold(0u8, |value, (n, bit)| value | u8::from(*bit) << n);
            expected.push(describe(&OneWireEvent::Bits {
                tx,
                value,
                count: scenario.bits.len() as u8,
            }));
        }
        expected.push(describe(&OneWireEvent::End {
            tx,
            reason: EndReason::Reset,
        }));
    }
    let last = scenarios.len() as u64 + 1;
    waveform.reset(true);
    expected.push(describe(&OneWireEvent::Begin {
        tx: last,
        presence: true,
    }));
    expected.push(describe(&OneWireEvent::End {
        tx: last,
        reason: EndReason::Aborted,
    }));
    (waveform.finish(), expected)
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 24, ..ProptestConfig::default() })]

    /// Random transactions decode to what made them, and blocks of any size, a reader that
    /// pauses at random and tiny queues and budgets yield the events of one uninterrupted run.
    #[test]
    fn random_traffic_decodes_alike_however_it_is_cut(
        scenarios in prop::collection::vec(scenario(), 1..5),
        overdrive in any::<bool>(),
        sizes in prop::collection::vec(1u64..3000, 1..5),
        pauses in prop::collection::vec(0usize..12, 1..6),
        queue_capacity in 1usize..5,
        budget in 1u32..40,
    ) {
        let (speed, rate) = if overdrive {
            (Speed::Overdrive, 10 * MHZ)
        } else {
            (Speed::Standard, MHZ)
        };
        let (block, expected) = synthesize(&scenarios, speed, rate);
        let (mut whole, _) = network(speed, rate, GraphConfig::default());
        whole.push(SignalItem::Data(Arc::new(block.clone())));
        whole.end();
        let reference = without_progress(&whole.items);
        let mut decoded: Vec<String> = reference
            .iter()
            .filter_map(Item::event)
            .map(|event| describe(&event.payload))
            .collect();
        prop_assert_eq!(&decoded, &expected);
        decoded.clear();

        let (mut harness, _) = network(speed, rate, GraphConfig { queue_capacity, budget });
        harness.drain_on_full = 1;
        harness.feed_cut(&block, &[], &sizes, &pauses);
        harness.end();
        prop_assert_eq!(without_progress(&harness.items), reference);
        prop_assert!(harness.pipeline().stats().within_capacity());
    }
}
