// SPDX-License-Identifier: Apache-2.0

//! The MIDI decoder on synthesized UART waveforms.

use std::collections::BTreeMap;
use std::sync::Arc;

use proptest::prelude::*;
use uiharu_core::testing::{Harness, assert_block_invariant, without_progress};
use uiharu_core::{
    GraphConfig, GraphError, Item, LogicBlock, SignalItem, Tick, TickRange, TimeBase,
};
use uiharu_protocols::midi::synth::{
    channel_message, channel_status, common_message, fourteen_bits, sysex,
};
use uiharu_protocols::midi::{
    ChannelStatus, ChannelStatusExt, CommonStatus, CommonStatusExt, EndReason, ErrorReason,
    MessageId, Midi, MidiEvent, MidiEventExt, SYSEX_BATCH,
};
use uiharu_protocols::uart::synth::UartWaveform;
use uiharu_protocols::uart::{BitOrder, Parity, StopBits, Uart, UartConfig};

const CHANNELS: u8 = 1;
/// Ten samples per bit: 31.25 kbaud sampled at 312.5 kHz.
const BIT: u64 = 10;
const RATE: u64 = 312_500;
/// A character: a start bit, eight data bits and a stop bit.
const CHAR: u64 = 10 * BIT;
/// Where in a character its stop bit is read: nine and a half bits after its start.
const STOP: u64 = 95;

/// MIDI's characters: 8N1 at 31.25 kbaud.
fn line() -> UartConfig {
    UartConfig {
        baud: Midi::BAUD,
        ..UartConfig::default()
    }
}

/// Source → UART → MIDI, the MIDI messages being the output.
fn build(uart: UartConfig, graph_config: GraphConfig) -> Result<Harness<MidiEvent>, GraphError> {
    Harness::build(
        CHANNELS,
        TimeBase::new(RATE),
        graph_config,
        |graph, source| {
            let uart = graph.add(Uart::new(uart).expect("a valid configuration"));
            graph.connect(source.samples, Uart::input(uart))?;
            let midi = graph.add(Midi::new());
            graph.connect(Uart::events(uart), Midi::input(midi))?;
            Ok(Midi::events(midi))
        },
    )
}

fn harness() -> Harness<MidiEvent> {
    build(line(), GraphConfig::default()).expect("builds")
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn optional(byte: Option<u8>) -> String {
    byte.map_or(String::new(), |byte| format!(" {byte:02x}"))
}

/// One line per item, readable in assertions.
fn describe(items: &[Item<MidiEvent>]) -> Vec<String> {
    items
        .iter()
        .map(|item| match item {
            Item::Event(event) => {
                let what = match &event.payload {
                    MidiEvent::ChannelMessage {
                        message,
                        channel,
                        status,
                        data1,
                        data2,
                        running,
                    } => format!(
                        "m{} {status:?} ch{channel} {data1:02x}{}{}",
                        message,
                        optional(*data2),
                        if *running { " running" } else { "" }
                    ),
                    MidiEvent::SystemCommon {
                        message,
                        status,
                        data1,
                        data2,
                    } => format!(
                        "m{} {status:?}{}{}",
                        message,
                        optional(*data1),
                        optional(*data2)
                    ),
                    MidiEvent::Realtime { status, byte } => {
                        format!("realtime {status:?} {byte:02x}")
                    }
                    MidiEvent::Sysex {
                        message,
                        offset,
                        bytes,
                    } => format!("m{} sysex +{offset} {}", message, hex(bytes)),
                    MidiEvent::SysexEnd {
                        message,
                        bytes,
                        reason,
                    } => format!("m{} sysex-end {bytes} {reason:?}", message),
                    MidiEvent::Error { reason, byte } => format!("error {reason:?} {byte:02x}"),
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

/// Decodes `block` in one piece: the lines, progress left out.
fn decode(block: &LogicBlock) -> Vec<String> {
    let mut harness = harness();
    harness.run_all(block);
    describe(&harness.without_progress())
}

/// A MIDI cable being synthesized, which keeps where each byte began.
struct Cable {
    waveform: UartWaveform,
    starts: Vec<u64>,
}

impl Cable {
    /// A cable idle for `idle` samples.
    fn new(idle: u64) -> Self {
        let mut waveform = UartWaveform::new(line(), CHANNELS, BIT, Tick::ZERO);
        waveform.idle(idle);
        Self {
            waveform,
            starts: Vec::new(),
        }
    }

    /// Sends `bytes`, each right after the one before.
    fn send(&mut self, bytes: impl IntoIterator<Item = u8>) {
        for byte in bytes {
            self.starts.push(self.waveform.position().0);
            self.waveform.byte(u16::from(byte));
        }
    }

    fn idle(&mut self, samples: u64) {
        self.waveform.idle(samples);
    }

    /// The samples, and where each byte began.
    fn finish(self) -> (LogicBlock, Vec<u64>) {
        (self.waveform.finish(), self.starts)
    }
}

/// Bytes `from..to` as the lines spell them, settled at `settled`.
fn over_until(starts: &[u64], from: usize, to: usize, settled: u64) -> String {
    format!("[{},{})@{settled}", starts[from], starts[to - 1] + CHAR)
}

/// Bytes `from..to`, settled with the last.
fn over(starts: &[u64], from: usize, to: usize) -> String {
    over_until(starts, from, to, starts[to - 1] + STOP)
}

/// Where byte `n` settles.
fn settled(starts: &[u64], n: usize) -> u64 {
    starts[n] + STOP
}

#[test]
fn every_channel_message() {
    let mut cable = Cable::new(200);
    for message in [
        channel_message(ChannelStatus::NoteOff, 0, &[0x3c, 0x40]),
        channel_message(ChannelStatus::NoteOn, 1, &[0x3c, 0x64]),
        channel_message(ChannelStatus::PolyPressure, 2, &[0x3c, 0x20]),
        channel_message(ChannelStatus::ControlChange, 3, &[0x07, 0x7f]),
        channel_message(ChannelStatus::ProgramChange, 4, &[0x05]),
        channel_message(ChannelStatus::ChannelPressure, 5, &[0x30]),
        // The middle of the pitch bend range, 8192: 0x00, then 0x40.
        channel_message(ChannelStatus::PitchBend, 15, &fourteen_bits(8192)),
    ] {
        cable.send(message);
    }
    cable.idle(100);
    let (block, starts) = cable.finish();
    assert_eq!(
        decode(&block),
        vec![
            format!("m1 NoteOff ch0 3c 40 {}", over(&starts, 0, 3)),
            format!("m2 NoteOn ch1 3c 64 {}", over(&starts, 3, 6)),
            format!("m3 PolyPressure ch2 3c 20 {}", over(&starts, 6, 9)),
            format!("m4 ControlChange ch3 07 7f {}", over(&starts, 9, 12)),
            format!("m5 ProgramChange ch4 05 {}", over(&starts, 12, 14)),
            format!("m6 ChannelPressure ch5 30 {}", over(&starts, 14, 16)),
            format!("m7 PitchBend ch15 00 40 {}", over(&starts, 16, 19)),
            format!("eof {}", block.end()),
        ]
    );
}

#[test]
fn running_status_repeats_the_last_channel_status() {
    let mut cable = Cable::new(200);
    // Three notes on, the third a note off by velocity 0, then three programs.
    cable.send([0x90, 0x3c, 0x64, 0x3e, 0x64, 0x3c, 0x00]);
    cable.send([0xc1, 0x05, 0x06]);
    cable.idle(150);
    cable.send([0x07]);
    cable.idle(100);
    let (block, starts) = cable.finish();
    assert_eq!(
        decode(&block),
        vec![
            format!("m1 NoteOn ch0 3c 64 {}", over(&starts, 0, 3)),
            format!("m2 NoteOn ch0 3e 64 running {}", over(&starts, 3, 5)),
            format!("m3 NoteOn ch0 3c 00 running {}", over(&starts, 5, 7)),
            format!("m4 ProgramChange ch1 05 {}", over(&starts, 7, 9)),
            format!("m5 ProgramChange ch1 06 running {}", over(&starts, 9, 10)),
            format!("m6 ProgramChange ch1 07 running {}", over(&starts, 10, 11)),
            format!("eof {}", block.end()),
        ]
    );
}

#[test]
fn realtime_messages_come_anywhere() {
    let mut cable = Cable::new(200);
    // Inside a message, between messages under running status, and undefined ones; a system
    // reset leaves the running status alone.
    cable.send([0x90, 0xf8, 0x3c, 0xfe, 0x64]);
    cable.send([0x3e, 0xfa, 0x64]);
    cable.send([0xf9, 0xfd, 0xff, 0xfb, 0xfc]);
    cable.send([0x40, 0x64]);
    cable.idle(100);
    let (block, starts) = cable.finish();
    assert_eq!(
        decode(&block),
        vec![
            format!("realtime Clock f8 {}", over(&starts, 1, 2)),
            format!("realtime ActiveSensing fe {}", over(&starts, 3, 4)),
            format!("m1 NoteOn ch0 3c 64 {}", over(&starts, 0, 5)),
            format!("realtime Start fa {}", over(&starts, 6, 7)),
            format!("m2 NoteOn ch0 3e 64 running {}", over(&starts, 5, 8)),
            format!("realtime Undefined f9 {}", over(&starts, 8, 9)),
            format!("realtime Undefined fd {}", over(&starts, 9, 10)),
            format!("realtime Reset ff {}", over(&starts, 10, 11)),
            format!("realtime Continue fb {}", over(&starts, 11, 12)),
            format!("realtime Stop fc {}", over(&starts, 12, 13)),
            format!("m3 NoteOn ch0 40 64 running {}", over(&starts, 13, 15)),
            format!("eof {}", block.end()),
        ]
    );
}

#[test]
fn a_system_exclusive_message_goes_out_in_batches() {
    let data: Vec<u8> = (0..100).collect();
    let mut cable = Cable::new(200);
    // A timing clock in the middle.
    cable.send([0xf0]);
    cable.send(data[..50].iter().copied());
    cable.send([0xf8]);
    cable.send(data[50..].iter().copied());
    cable.send([0xf7]);
    cable.idle(100);
    let (block, starts) = cable.finish();
    // Byte 0 is the start of exclusive, data bytes 0 to 49 are bytes 1 to 50, the clock byte
    // 51, data bytes 50 to 99 bytes 52 to 101, the end of exclusive byte 102.
    assert_eq!(
        decode(&block),
        vec![
            format!("m1 sysex +0 {} {}", hex(&data[..32]), over(&starts, 0, 33)),
            format!("realtime Clock f8 {}", over(&starts, 51, 52)),
            format!(
                "m1 sysex +32 {} {}",
                hex(&data[32..64]),
                over(&starts, 33, 66)
            ),
            format!(
                "m1 sysex +64 {} {}",
                hex(&data[64..96]),
                over(&starts, 66, 98)
            ),
            format!(
                "m1 sysex +96 {} {}",
                hex(&data[96..]),
                over_until(&starts, 98, 102, settled(&starts, 102))
            ),
            format!(
                "m1 sysex-end 100 EndOfExclusive {}",
                over(&starts, 102, 103)
            ),
            format!("eof {}", block.end()),
        ]
    );
    assert_eq!(SYSEX_BATCH, 32);
}

#[test]
fn a_status_byte_interrupts_a_system_exclusive_message() {
    let mut cable = Cable::new(200);
    cable.send([0xf0, 0x01, 0x02, 0x03]);
    cable.send([0x90, 0x3c, 0x64]);
    // Without data, interrupted, and ended.
    cable.send([0xf0, 0xc0, 0x05]);
    cable.send(sysex(&[]));
    cable.idle(100);
    let (block, starts) = cable.finish();
    let interrupted =
        |at: usize| format!("[{},{})@{}", starts[at], starts[at], settled(&starts, at));
    assert_eq!(
        decode(&block),
        vec![
            format!(
                "m1 sysex +0 010203 {}",
                over_until(&starts, 0, 4, settled(&starts, 4))
            ),
            format!("m1 sysex-end 3 Interrupted {}", interrupted(4)),
            format!("m2 NoteOn ch0 3c 64 {}", over(&starts, 4, 7)),
            // No batch covered the start of exclusive: the end spans it.
            format!(
                "m3 sysex-end 0 Interrupted [{},{})@{}",
                starts[7],
                starts[8],
                settled(&starts, 8)
            ),
            format!("m4 ProgramChange ch0 05 {}", over(&starts, 8, 10)),
            format!("m5 sysex-end 0 EndOfExclusive {}", over(&starts, 10, 12)),
            format!("eof {}", block.end()),
        ]
    );
}

#[test]
fn system_common_messages_clear_the_running_status() {
    let mut cable = Cable::new(200);
    // Song position 272 MIDI beats.
    cable.send(common_message(
        CommonStatus::SongPositionPointer,
        &fourteen_bits(272),
    ));
    cable.send(common_message(CommonStatus::TimeCodeQuarterFrame, &[0x25]));
    cable.send(common_message(CommonStatus::SongSelect, &[0x07]));
    cable.send(common_message(CommonStatus::TuneRequest, &[]));
    cable.send([0x90, 0x3c, 0x64, 0xf6, 0x3e, 0x64]);
    cable.idle(100);
    let (block, starts) = cable.finish();
    assert_eq!(
        decode(&block),
        vec![
            format!("m1 SongPositionPointer 10 02 {}", over(&starts, 0, 3)),
            format!("m2 TimeCodeQuarterFrame 25 {}", over(&starts, 3, 5)),
            format!("m3 SongSelect 07 {}", over(&starts, 5, 7)),
            format!("m4 TuneRequest {}", over(&starts, 7, 8)),
            format!("m5 NoteOn ch0 3c 64 {}", over(&starts, 8, 11)),
            format!("m6 TuneRequest {}", over(&starts, 11, 12)),
            format!("error UnexpectedData 3e {}", over(&starts, 12, 13)),
            format!("error UnexpectedData 64 {}", over(&starts, 13, 14)),
            format!("eof {}", block.end()),
        ]
    );
}

#[test]
fn data_before_any_status_is_unexpected() {
    // The samples begin under a running status that was not seen.
    let mut cable = Cable::new(200);
    cable.send([0x3c, 0x64, 0x90, 0x3c, 0x64]);
    cable.idle(100);
    let (block, starts) = cable.finish();
    assert_eq!(
        decode(&block),
        vec![
            format!("error UnexpectedData 3c {}", over(&starts, 0, 1)),
            format!("error UnexpectedData 64 {}", over(&starts, 1, 2)),
            format!("m1 NoteOn ch0 3c 64 {}", over(&starts, 2, 5)),
            format!("eof {}", block.end()),
        ]
    );
}

#[test]
fn a_status_byte_cuts_a_message_short() {
    let mut cable = Cable::new(200);
    cable.send([0x90, 0x3c, 0xc0, 0x05]);
    cable.send([0xf2, 0x10, 0xb0, 0x07, 0x7f]);
    // An undefined status byte cuts a message too.
    cable.send([0x91, 0x40, 0xf4, 0x64]);
    cable.idle(100);
    let (block, starts) = cable.finish();
    assert_eq!(
        decode(&block),
        vec![
            format!(
                "error Incomplete 90 {}",
                over_until(&starts, 0, 2, settled(&starts, 2))
            ),
            format!("m1 ProgramChange ch0 05 {}", over(&starts, 2, 4)),
            format!(
                "error Incomplete f2 {}",
                over_until(&starts, 4, 6, settled(&starts, 6))
            ),
            format!("m2 ControlChange ch0 07 7f {}", over(&starts, 6, 9)),
            format!(
                "error Incomplete 91 {}",
                over_until(&starts, 9, 11, settled(&starts, 11))
            ),
            format!("error UndefinedStatus f4 {}", over(&starts, 11, 12)),
            format!("error UnexpectedData 64 {}", over(&starts, 12, 13)),
            format!("eof {}", block.end()),
        ]
    );
}

#[test]
fn undefined_and_stray_status_bytes_clear_the_running_status() {
    let mut cable = Cable::new(200);
    cable.send([0x90, 0x3c, 0x64, 0xf5, 0x3e]);
    cable.send([0x90, 0x3c, 0x64, 0xf7, 0x3e]);
    cable.idle(100);
    let (block, starts) = cable.finish();
    assert_eq!(
        decode(&block),
        vec![
            format!("m1 NoteOn ch0 3c 64 {}", over(&starts, 0, 3)),
            format!("error UndefinedStatus f5 {}", over(&starts, 3, 4)),
            format!("error UnexpectedData 3e {}", over(&starts, 4, 5)),
            format!("m2 NoteOn ch0 3c 64 {}", over(&starts, 5, 8)),
            format!("error UnexpectedEndOfExclusive f7 {}", over(&starts, 8, 9)),
            format!("error UnexpectedData 3e {}", over(&starts, 9, 10)),
            format!("eof {}", block.end()),
        ]
    );
}

#[test]
fn the_end_of_the_samples_cuts_what_is_under_way() {
    let decode_bytes = |bytes: &[u8]| {
        let mut cable = Cable::new(200);
        cable.send(bytes.iter().copied());
        cable.idle(50);
        let (block, starts) = cable.finish();
        (decode(&block), starts, block.end().0)
    };
    let (lines, starts, end) = decode_bytes(&[0x90, 0x3c]);
    assert_eq!(
        lines,
        vec![
            format!("error Aborted 90 {}", over_until(&starts, 0, 2, end)),
            format!("eof {end}"),
        ]
    );
    // Under running status, the message begins with its data byte.
    let (lines, starts, end) = decode_bytes(&[0x90, 0x3c, 0x64, 0x3e]);
    assert_eq!(
        lines[1],
        format!("error Aborted 90 {}", over_until(&starts, 3, 4, end))
    );
    let (lines, starts, end) = decode_bytes(&[0xf0, 0x01, 0x02]);
    assert_eq!(
        lines,
        vec![
            format!("m1 sysex +0 0102 {}", over_until(&starts, 0, 3, end)),
            format!("m1 sysex-end 2 Aborted [{end},{end})@{end}"),
            format!("eof {end}"),
        ]
    );
    let (lines, starts, end) = decode_bytes(&[0xf0]);
    assert_eq!(
        lines,
        vec![
            format!("m1 sysex-end 0 Aborted [{},{end})@{end}", starts[0]),
            format!("eof {end}"),
        ]
    );
}

#[test]
fn a_gap_cuts_what_is_under_way_and_clears_the_running_status() {
    let mut cable = Cable::new(200);
    cable.send([0x90, 0x3c, 0x64, 0x3e, 0x64]);
    cable.idle(300);
    cable.send([0x40, 0x64, 0x90, 0x40, 0x64]);
    cable.idle(300);
    cable.send([0xf0, 0x11, 0x22, 0x33]);
    cable.idle(300);
    cable.send([0xf0, 0x44, 0xf7]);
    cable.idle(100);
    let (block, starts) = cable.finish();
    // From the first data bit of byte 4 to its stop bit: the byte is lost and the line is idle
    // where the samples resume. And inside byte 13 of the system exclusive message.
    let gaps = [
        TickRange::new(Tick(starts[4] + 25), Tick(starts[4] + 95)),
        TickRange::new(Tick(starts[13] + 15), Tick(starts[13] + 95)),
    ];
    let mut harness = harness();
    harness.feed_with_gaps(&block, &gaps, &[u64::MAX]);
    harness.end();
    let (first, second) = (gaps[0].start.0, gaps[1].start.0);
    assert_eq!(
        describe(&harness.without_progress()),
        vec![
            format!("m1 NoteOn ch0 3c 64 {}", over(&starts, 0, 3)),
            // The second note under running status lost its velocity.
            format!("error Aborted 90 {}", over_until(&starts, 3, 4, first)),
            format!("gap {}", gaps[0]),
            // After the gap the running status is not known.
            format!("error UnexpectedData 40 {}", over(&starts, 5, 6)),
            format!("error UnexpectedData 64 {}", over(&starts, 6, 7)),
            format!("m2 NoteOn ch0 40 64 {}", over(&starts, 7, 10)),
            format!("m3 sysex +0 1122 {}", over_until(&starts, 10, 13, second)),
            format!("m3 sysex-end 2 Aborted [{second},{second})@{second}"),
            format!("gap {}", gaps[1]),
            format!(
                "m4 sysex +0 44 {}",
                over_until(&starts, 14, 16, settled(&starts, 16))
            ),
            format!("m4 sysex-end 1 EndOfExclusive {}", over(&starts, 16, 17)),
            format!("eof {}", block.end()),
        ]
    );
}

#[test]
fn the_uart_stream_must_carry_midi_characters() {
    let refused = |uart: UartConfig| build(uart, GraphConfig::default()).expect_err("refused");
    assert_eq!(
        refused(UartConfig {
            baud: 38_400,
            ..line()
        }),
        GraphError::Requirement {
            from: "uart#0.events".into(),
            to: "midi#1.uart".into(),
            key: "baud",
            requirement: "in 30938..=31562".into(),
            found: "38400".into(),
        }
    );
    for (uart, key) in [
        (
            UartConfig {
                data_bits: 7,
                ..line()
            },
            "dataBits",
        ),
        (
            UartConfig {
                parity: Parity::Even,
                ..line()
            },
            "parity",
        ),
        (
            UartConfig {
                stop_bits: StopBits::Two,
                ..line()
            },
            "stopBits",
        ),
        (
            UartConfig {
                bit_order: BitOrder::MsbFirst,
                ..line()
            },
            "bitOrder",
        ),
    ] {
        match refused(uart) {
            GraphError::Requirement { key: refused, .. } => assert_eq!(refused, key),
            other => panic!("refused for {other}"),
        }
    }
}

/// Traffic of every kind; returns the samples and where each byte began.
fn busy_cable() -> (LogicBlock, Vec<u64>) {
    let mut cable = Cable::new(300);
    let data: Vec<u8> = (0..70u8).map(|i| i.wrapping_mul(37) & 0x7f).collect();
    // Bytes 0 to 7: notes under running status, a clock inside one.
    cable.send([0x93, 0x3c, 0x64, 0xf8, 0x3e, 0x64, 0x40, 0x00]);
    // 8 to 12: a controller, and one under running status that waits on the idle line.
    cable.send([0xb3, 0x07, 0x7f, 0x0a]);
    cable.idle(120);
    cable.send([0x40]);
    // 13 to 22: a song position, data without a status, a tune request, pitch bends.
    cable.send(common_message(
        CommonStatus::SongPositionPointer,
        &[0x10, 0x02],
    ));
    cable.send([0x3c, 0xf6, 0xe0, 0x00, 0x40, 0x7f, 0x7f]);
    cable.idle(400);
    // 23 to 94: a system exclusive message of 70 bytes.
    cable.send(sysex(&data));
    // 95 to 141: one with active sensing inside, which a program change interrupts.
    cable.send([0xf0]);
    cable.send(data[..40].iter().copied());
    cable.send([0xfe]);
    cable.send(data[40..45].iter().copied());
    // 142 to 150: undefined and stray status bytes, a quarter frame, a song select cut short.
    cable.send([0xc5, 0x09, 0xf4, 0x01, 0xf9, 0xf7, 0xf1, 0x25, 0xf3]);
    cable.idle(250);
    // 151 to 158: a song select, notes and channel pressures.
    cable.send([0xf3, 0x02, 0x91, 0x30]);
    cable.idle(80);
    cable.send([0x31, 0xd2, 0x55, 0x56]);
    cable.idle(60);
    // 159 to 162: a system exclusive message the end of the samples cuts off.
    cable.send([0xf0, 0x01, 0x02, 0x03]);
    cable.idle(60);
    cable.finish()
}

/// The reasons of the errors and system exclusive ends of a run, and the messages in it.
fn outcomes(items: &[Item<MidiEvent>]) -> (Vec<ErrorReason>, Vec<EndReason>, Vec<MessageId>) {
    let mut errors = Vec::new();
    let mut ends = Vec::new();
    let mut messages = Vec::new();
    for event in items.iter().filter_map(Item::event) {
        match event.payload {
            MidiEvent::Error { reason, .. } => errors.push(reason),
            MidiEvent::SysexEnd { reason, .. } => ends.push(reason),
            _ => {}
        }
        if let Some(message) = event.payload.message()
            && messages.last() != Some(&message)
        {
            messages.push(message);
        }
    }
    (errors, ends, messages)
}

#[test]
fn every_cut_and_pause_yields_the_same_messages() {
    let (block, starts) = busy_cable();
    // Inside the second data byte of a note under running status, and on the idle line before
    // the first system exclusive message.
    let gaps = [
        TickRange::new(Tick(starts[5] + 25), Tick(starts[5] + 95)),
        TickRange::new(Tick(starts[22] + 200), Tick(starts[22] + 300)),
    ];
    let reference = {
        let mut harness = harness();
        harness.feed_with_gaps(&block, &gaps, &[u64::MAX]);
        harness.end();
        harness.without_progress()
    };
    let (errors, ends, messages) = outcomes(&reference);
    for reason in [
        ErrorReason::UnexpectedData,
        ErrorReason::Incomplete,
        ErrorReason::Aborted,
        ErrorReason::UndefinedStatus,
        ErrorReason::UnexpectedEndOfExclusive,
    ] {
        assert!(errors.contains(&reason), "{reason:?} in {errors:?}");
    }
    assert_eq!(
        ends,
        vec![
            EndReason::EndOfExclusive,
            EndReason::Interrupted,
            EndReason::Aborted
        ]
    );
    // Messages are numbered in order, from 1.
    let numbers: Vec<u64> = messages.to_vec();
    let expected: Vec<u64> = (1..=numbers.len() as u64).collect();
    assert_eq!(numbers, expected);
    assert_block_invariant(
        |graph_config| build(line(), graph_config).expect("builds"),
        &block,
        &gaps,
    );
}

#[test]
fn progress_never_passes_a_byte_not_yet_reported() {
    let (block, _) = busy_cable();
    for sizes in [&[1u64][..], &[13], &[64, 7]] {
        let mut harness = harness();
        harness.feed_cut(&block, &[], sizes, &[1]);
        harness.end();
        let mut seen = Tick::ZERO;
        let mut settled = Tick::ZERO;
        for item in &harness.items {
            match item {
                Item::Progress(tick) => seen = seen.max(*tick),
                Item::Event(event) => {
                    assert!(event.span.start >= seen, "{event:?} after {seen}");
                    assert!(event.settled >= settled, "{event:?} after {settled}");
                    settled = event.settled;
                }
                Item::Gap(_) | Item::End(_) => {}
            }
        }
    }
}

/// A piece of random MIDI traffic.
#[derive(Clone, Debug)]
enum Token {
    /// A channel message, sent with its status byte or under running status.
    Channel {
        status: ChannelStatus,
        channel: u8,
        data: [u8; 2],
        running: bool,
    },
    /// A system common message.
    Common { status: CommonStatus, data: [u8; 2] },
    /// A real-time status byte.
    Realtime(u8),
    /// A system exclusive message, ended by an end of exclusive or by whatever follows.
    Sysex { data: Vec<u8>, ended: bool },
    /// Any byte at all.
    Stray(u8),
    /// The idle line.
    Idle(u64),
}

fn token() -> impl Strategy<Value = Token> {
    let statuses = [
        ChannelStatus::NoteOff,
        ChannelStatus::NoteOn,
        ChannelStatus::PolyPressure,
        ChannelStatus::ControlChange,
        ChannelStatus::ProgramChange,
        ChannelStatus::ChannelPressure,
        ChannelStatus::PitchBend,
    ];
    let commons = [
        CommonStatus::TimeCodeQuarterFrame,
        CommonStatus::SongPositionPointer,
        CommonStatus::SongSelect,
        CommonStatus::TuneRequest,
    ];
    prop_oneof![
        5 => (
            prop::sample::select(statuses.to_vec()),
            0u8..16,
            any::<[u8; 2]>(),
            prop::bool::weighted(0.4),
        )
            .prop_map(|(status, channel, data, running)| Token::Channel {
                status,
                channel,
                data,
                running,
            }),
        1 => (prop::sample::select(commons.to_vec()), any::<[u8; 2]>())
            .prop_map(|(status, data)| Token::Common { status, data }),
        2 => (0xf8u8..=0xff).prop_map(Token::Realtime),
        1 => (prop::collection::vec(any::<u8>(), 0..80), prop::bool::weighted(0.8))
            .prop_map(|(data, ended)| Token::Sysex { data, ended }),
        1 => any::<u8>().prop_map(Token::Stray),
        1 => (1u64..300).prop_map(Token::Idle),
    ]
}

fn token_waveform(tokens: &[Token]) -> LogicBlock {
    let mut cable = Cable::new(37);
    for token in tokens {
        match token {
            Token::Channel {
                status,
                channel,
                data,
                running,
            } => {
                let bytes =
                    channel_message(*status, *channel, &data[..usize::from(status.data_bytes())]);
                cable.send(bytes.into_iter().skip(usize::from(*running)));
            }
            Token::Common { status, data } => {
                cable.send(common_message(
                    *status,
                    &data[..usize::from(status.data_bytes())],
                ));
            }
            Token::Realtime(byte) | Token::Stray(byte) => cable.send([*byte]),
            Token::Sysex { data, ended } => {
                let mut bytes = sysex(data);
                if !ended {
                    bytes.pop();
                }
                cable.send(bytes);
            }
            Token::Idle(samples) => cable.idle(*samples),
        }
    }
    cable.idle(11);
    cable.finish().0
}

/// Checks a run against the contract: events settle in order, messages are numbered from 1 as
/// they begin, a system exclusive message's batches follow each other within their bound and
/// one end counts them, and data bytes have their top bit clear.
fn check_messages(items: &[Item<MidiEvent>]) -> Result<(), TestCaseError> {
    let mut settled = Tick::ZERO;
    let mut last = 0;
    let mut exclusive: Option<(MessageId, u32)> = None;
    let mut ended = BTreeMap::new();
    for event in items.iter().filter_map(Item::event) {
        prop_assert!(
            event.settled >= settled,
            "{event:?} settled before {settled}"
        );
        settled = event.settled;
        if let Some(message) = event.payload.message()
            && message > last
        {
            prop_assert_eq!(message, last + 1, "{:?}", event);
            last = message;
        }
        match &event.payload {
            MidiEvent::ChannelMessage {
                channel,
                status,
                data1,
                data2,
                ..
            } => {
                prop_assert!(exclusive.is_none());
                prop_assert!(*channel < 16 && *data1 < 0x80);
                prop_assert_eq!(data2.is_some(), status.data_bytes() == 2);
                prop_assert!(data2.is_none_or(|byte| byte < 0x80));
            }
            MidiEvent::SystemCommon {
                status,
                data1,
                data2,
                ..
            } => {
                prop_assert!(exclusive.is_none());
                prop_assert_eq!(data1.is_some(), status.data_bytes() >= 1);
                prop_assert_eq!(data2.is_some(), status.data_bytes() == 2);
            }
            MidiEvent::Sysex {
                message,
                offset,
                bytes,
            } => {
                prop_assert!(!bytes.is_empty() && bytes.len() <= SYSEX_BATCH);
                prop_assert!(bytes.iter().all(|byte| *byte < 0x80));
                let count = match exclusive {
                    Some((open, count)) => {
                        prop_assert_eq!(open, *message);
                        count
                    }
                    None => 0,
                };
                prop_assert_eq!(*offset, count);
                exclusive = Some((*message, count + bytes.len() as u32));
            }
            MidiEvent::SysexEnd { message, bytes, .. } => {
                let count = match exclusive.take() {
                    Some((open, count)) => {
                        prop_assert_eq!(open, *message);
                        count
                    }
                    None => 0,
                };
                prop_assert_eq!(*bytes, count);
                prop_assert!(ended.insert(*message, *bytes).is_none(), "ended twice");
            }
            MidiEvent::Realtime { byte, .. } => prop_assert!(*byte >= 0xf8),
            MidiEvent::Error { .. } => {}
        }
    }
    prop_assert!(exclusive.is_none(), "a system exclusive message left open");
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 48, ..ProptestConfig::default() })]

    /// Blocks of any size, a reader that pauses and tiny queues and budgets all yield the
    /// messages of one uninterrupted run, and every run keeps to the contract.
    #[test]
    fn messages_are_the_same_however_the_samples_come(
        tokens in prop::collection::vec(token(), 1..40),
        sizes in prop::collection::vec(1u64..500, 1..6),
        pauses in prop::collection::vec(0usize..12, 1..8),
        queue_capacity in 1usize..6,
        budget in 1u32..40,
        cut in prop::option::of(0.05f64..1.0),
        gap in prop::option::of((0.05f64..0.95, 1u64..400)),
    ) {
        let whole = token_waveform(&tokens);
        let block = match cut {
            Some(cut) => {
                let end = Tick(((whole.len() as f64) * cut) as u64).max(Tick(1));
                whole.slice(TickRange::new(Tick::ZERO, end)).expect("a slice")
            }
            None => whole,
        };
        let gaps: Vec<TickRange> = gap
            .and_then(|(at, len)| {
                let start = ((block.len() as f64) * at) as u64;
                let end = (start + len).min(block.len());
                (start < end).then(|| TickRange::new(Tick(start), Tick(end)))
            })
            .into_iter()
            .collect();
        let reference = {
            let mut harness = harness();
            match gaps.first() {
                Some(_) => harness.feed_with_gaps(&block, &gaps, &[u64::MAX]),
                None => harness.push(SignalItem::Data(Arc::new(block.clone()))),
            }
            harness.end();
            harness.without_progress()
        };

        let mut harness = build(line(), GraphConfig { queue_capacity, budget })
            .expect("builds");
        harness.drain_on_full = 1;
        harness.feed_cut(&block, &gaps, &sizes, &pauses);
        harness.end();
        prop_assert_eq!(without_progress(&harness.items), reference.clone());
        let stats = harness.pipeline().stats();
        prop_assert!(stats.within_capacity(), "{stats:?}");
        check_messages(&reference)?;
    }
}

#[test]
fn channel_status_bytes_carry_the_channel() {
    let mut cable = Cable::new(100);
    for channel in [0, 7, 15] {
        cable.send([
            channel_status(ChannelStatus::ControlChange, channel),
            0x40,
            0x00,
        ]);
    }
    cable.idle(50);
    let (block, _) = cable.finish();
    let mut harness = harness();
    harness.run_all(&block);
    let channels: Vec<u8> = harness
        .events()
        .iter()
        .filter_map(|event| match event.payload {
            MidiEvent::ChannelMessage { channel, .. } => Some(channel),
            _ => None,
        })
        .collect();
    assert_eq!(channels, vec![0, 7, 15]);
}
