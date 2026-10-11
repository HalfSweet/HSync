// SPDX-License-Identifier: Apache-2.0

//! The `midi.events` contract: the messages of a MIDI 1.0 link, as a MIDI decoder emits them and
//! decoders of an instrument's or a sequencer's traffic read them.
//!
//! Every message is one event, sent once its last byte has come, except system exclusive
//! messages, which may be long: their data goes out in batches as it comes, and a `sysex-end`
//! closes them. Real-time messages, single bytes that may come anywhere, are events of their
//! own. Data bytes are reported as sent, not interpreted: what a controller number or a
//! manufacturer's identifier means is the next layer's business.

use serde::{Deserialize, Serialize};
use uiharu_core::{EventSchema, SchemaId};

/// The schema of MIDI event streams.
pub const MIDI_EVENTS: SchemaId = SchemaId::new("midi.events", 1);

/// The most data bytes one `sysex` event carries.
pub const SYSEX_BATCH: usize = 32;

/// A message, numbered from 1 by the decoder; real-time messages are not numbered.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct MessageId(pub u64);

/// A channel message, named by the high nibble of its status byte.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ChannelStatus {
    /// 0x8n, note off: a note number and a velocity.
    NoteOff,
    /// 0x9n, note on: a note number and a velocity, which is 0 for a note off.
    NoteOn,
    /// 0xAn, polyphonic key pressure: a note number and a pressure.
    PolyPressure,
    /// 0xBn, control change: a controller number and a value; controllers 120 to 127 are the
    /// channel mode messages.
    ControlChange,
    /// 0xCn, program change: a program number.
    ProgramChange,
    /// 0xDn, channel pressure: a pressure.
    ChannelPressure,
    /// 0xEn, pitch bend change: the low seven bits of a 14-bit value, then its high seven bits.
    PitchBend,
}

impl ChannelStatus {
    /// The message a channel status byte, 0x80 to 0xEF, names; `None` for any other byte.
    pub const fn from_status(status: u8) -> Option<Self> {
        match status >> 4 {
            0x8 => Some(ChannelStatus::NoteOff),
            0x9 => Some(ChannelStatus::NoteOn),
            0xa => Some(ChannelStatus::PolyPressure),
            0xb => Some(ChannelStatus::ControlChange),
            0xc => Some(ChannelStatus::ProgramChange),
            0xd => Some(ChannelStatus::ChannelPressure),
            0xe => Some(ChannelStatus::PitchBend),
            _ => None,
        }
    }

    /// The high nibble of the message's status byte, 0x8 to 0xE.
    pub const fn code(self) -> u8 {
        match self {
            ChannelStatus::NoteOff => 0x8,
            ChannelStatus::NoteOn => 0x9,
            ChannelStatus::PolyPressure => 0xa,
            ChannelStatus::ControlChange => 0xb,
            ChannelStatus::ProgramChange => 0xc,
            ChannelStatus::ChannelPressure => 0xd,
            ChannelStatus::PitchBend => 0xe,
        }
    }

    /// The data bytes the message carries: one for a program change or a channel pressure, two
    /// for the others.
    pub const fn data_bytes(self) -> u8 {
        match self {
            ChannelStatus::ProgramChange | ChannelStatus::ChannelPressure => 1,
            _ => 2,
        }
    }
}

/// A system common message.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CommonStatus {
    /// 0xF1, MIDI time code quarter frame: one data byte, a message type and a value.
    TimeCodeQuarterFrame,
    /// 0xF2, song position pointer: the low seven bits of a 14-bit count of MIDI beats, then its
    /// high seven bits.
    SongPositionPointer,
    /// 0xF3, song select: a song number.
    SongSelect,
    /// 0xF6, tune request: no data byte.
    TuneRequest,
}

impl CommonStatus {
    /// The message a system common status byte names; `None` for any other byte, and for the
    /// undefined 0xF4 and 0xF5 and the end of exclusive, 0xF7.
    pub const fn from_status(status: u8) -> Option<Self> {
        match status {
            0xf1 => Some(CommonStatus::TimeCodeQuarterFrame),
            0xf2 => Some(CommonStatus::SongPositionPointer),
            0xf3 => Some(CommonStatus::SongSelect),
            0xf6 => Some(CommonStatus::TuneRequest),
            _ => None,
        }
    }

    /// The message's status byte.
    pub const fn byte(self) -> u8 {
        match self {
            CommonStatus::TimeCodeQuarterFrame => 0xf1,
            CommonStatus::SongPositionPointer => 0xf2,
            CommonStatus::SongSelect => 0xf3,
            CommonStatus::TuneRequest => 0xf6,
        }
    }

    /// The data bytes the message carries.
    pub const fn data_bytes(self) -> u8 {
        match self {
            CommonStatus::TimeCodeQuarterFrame | CommonStatus::SongSelect => 1,
            CommonStatus::SongPositionPointer => 2,
            CommonStatus::TuneRequest => 0,
        }
    }
}

/// A system real-time message.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RealtimeStatus {
    /// 0xF8, timing clock.
    Clock,
    /// 0xFA, start.
    Start,
    /// 0xFB, continue.
    Continue,
    /// 0xFC, stop.
    Stop,
    /// 0xFE, active sensing.
    ActiveSensing,
    /// 0xFF, system reset.
    Reset,
    /// 0xF9 or 0xFD, which the specification leaves undefined.
    Undefined,
}

impl RealtimeStatus {
    /// The message a real-time status byte, 0xF8 to 0xFF, names; `None` for any other byte.
    pub const fn from_status(status: u8) -> Option<Self> {
        match status {
            0xf8 => Some(RealtimeStatus::Clock),
            0xfa => Some(RealtimeStatus::Start),
            0xfb => Some(RealtimeStatus::Continue),
            0xfc => Some(RealtimeStatus::Stop),
            0xfe => Some(RealtimeStatus::ActiveSensing),
            0xff => Some(RealtimeStatus::Reset),
            0xf9 | 0xfd => Some(RealtimeStatus::Undefined),
            _ => None,
        }
    }
}

/// How a system exclusive message ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EndReason {
    /// An end of exclusive, 0xF7.
    EndOfExclusive,
    /// Another status byte, other than real-time, which ends it as well.
    Interrupted,
    /// The samples ended or were lost before it did.
    Aborted,
}

/// What is wrong.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ErrorReason {
    /// A data byte with no status in effect: no running status, since a system message cleared
    /// it, or because none was seen since the samples began or resumed after a gap.
    UnexpectedData,
    /// A message cut short: a status byte, other than real-time, came before its last data
    /// byte.
    Incomplete,
    /// A message cut off: the samples ended or were lost before its last data byte.
    Aborted,
    /// An undefined system common status byte, 0xF4 or 0xF5.
    UndefinedStatus,
    /// An end of exclusive, 0xF7, with no system exclusive message to end.
    UnexpectedEndOfExclusive,
}

/// What travels on a MIDI event stream.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum MidiEvent {
    /// A channel voice or mode message.
    ChannelMessage {
        /// The message.
        message: MessageId,
        /// The low nibble of the status byte, 0 to 15, for MIDI channels 1 to 16.
        channel: u8,
        /// The message, from the high nibble of the status byte.
        status: ChannelStatus,
        /// The first data byte.
        data1: u8,
        /// The second data byte; `None` for a message of one.
        data2: Option<u8>,
        /// Whether the message was sent without its status byte, under running status.
        running: bool,
    },
    /// A system common message.
    SystemCommon {
        /// The message.
        message: MessageId,
        /// The message, from its status byte.
        status: CommonStatus,
        /// The first data byte; `None` for a message without data.
        data1: Option<u8>,
        /// The second data byte; `None` for a message of fewer than two.
        data2: Option<u8>,
    },
    /// A system real-time message.
    Realtime {
        /// The message.
        status: RealtimeStatus,
        /// Its status byte, 0xF8 to 0xFF.
        byte: u8,
    },
    /// Data bytes of a system exclusive message, in order.
    Sysex {
        /// The message.
        message: MessageId,
        /// The position of the first byte, from 0 for the byte after the start of exclusive.
        offset: u32,
        /// At most [`SYSEX_BATCH`] bytes.
        bytes: Vec<u8>,
    },
    /// The end of a system exclusive message.
    SysexEnd {
        /// The message.
        message: MessageId,
        /// The data bytes it carried.
        bytes: u32,
        /// How it ended.
        reason: EndReason,
    },
    /// Bytes that make no message.
    Error {
        /// What is wrong.
        reason: ErrorReason,
        /// The byte the error is about: the data byte without a status, the undefined status
        /// byte, the stray end of exclusive, or the status of the message cut short or off —
        /// its running status, for a message under running status.
        byte: u8,
    },
}

impl MidiEvent {
    /// The message the event belongs to; `None` for real-time messages and errors.
    pub fn message(&self) -> Option<MessageId> {
        match self {
            MidiEvent::ChannelMessage { message, .. }
            | MidiEvent::SystemCommon { message, .. }
            | MidiEvent::Sysex { message, .. }
            | MidiEvent::SysexEnd { message, .. } => Some(*message),
            MidiEvent::Realtime { .. } | MidiEvent::Error { .. } => None,
        }
    }
}

impl EventSchema for MidiEvent {
    const SCHEMA: SchemaId = MIDI_EVENTS;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn json(event: &MidiEvent) -> String {
        serde_json::to_string(event).expect("serializes")
    }

    #[test]
    fn serializes_as_the_contract_spells_it() {
        assert_eq!(
            json(&MidiEvent::ChannelMessage {
                message: MessageId(4),
                channel: 9,
                status: ChannelStatus::NoteOn,
                data1: 60,
                data2: Some(100),
                running: true,
            }),
            r#"{"kind":"channel-message","message":4,"channel":9,"status":"note-on","data1":60,"data2":100,"running":true}"#
        );
        assert_eq!(
            json(&MidiEvent::ChannelMessage {
                message: MessageId(5),
                channel: 0,
                status: ChannelStatus::ProgramChange,
                data1: 5,
                data2: None,
                running: false,
            }),
            r#"{"kind":"channel-message","message":5,"channel":0,"status":"program-change","data1":5,"data2":null,"running":false}"#
        );
        assert_eq!(
            json(&MidiEvent::SystemCommon {
                message: MessageId(6),
                status: CommonStatus::SongPositionPointer,
                data1: Some(0x10),
                data2: Some(0x02),
            }),
            r#"{"kind":"system-common","message":6,"status":"song-position-pointer","data1":16,"data2":2}"#
        );
        assert_eq!(
            json(&MidiEvent::Realtime {
                status: RealtimeStatus::ActiveSensing,
                byte: 0xfe,
            }),
            r#"{"kind":"realtime","status":"active-sensing","byte":254}"#
        );
        assert_eq!(
            json(&MidiEvent::Sysex {
                message: MessageId(7),
                offset: 32,
                bytes: vec![0x43, 0x10],
            }),
            r#"{"kind":"sysex","message":7,"offset":32,"bytes":[67,16]}"#
        );
        assert_eq!(
            json(&MidiEvent::SysexEnd {
                message: MessageId(7),
                bytes: 34,
                reason: EndReason::EndOfExclusive,
            }),
            r#"{"kind":"sysex-end","message":7,"bytes":34,"reason":"end-of-exclusive"}"#
        );
        assert_eq!(
            json(&MidiEvent::Error {
                reason: ErrorReason::UnexpectedEndOfExclusive,
                byte: 0xf7,
            }),
            r#"{"kind":"error","reason":"unexpected-end-of-exclusive","byte":247}"#
        );
    }

    fn name(value: impl Serialize) -> String {
        serde_json::to_string(&value).expect("serializes")
    }

    #[test]
    fn channel_messages_follow_their_status_byte() {
        let statuses = [
            (0x80, ChannelStatus::NoteOff, "note-off", 2),
            (0x91, ChannelStatus::NoteOn, "note-on", 2),
            (0xa2, ChannelStatus::PolyPressure, "poly-pressure", 2),
            (0xb3, ChannelStatus::ControlChange, "control-change", 2),
            (0xc4, ChannelStatus::ProgramChange, "program-change", 1),
            (0xd5, ChannelStatus::ChannelPressure, "channel-pressure", 1),
            (0xef, ChannelStatus::PitchBend, "pitch-bend", 2),
        ];
        for (byte, status, kebab, data_bytes) in statuses {
            assert_eq!(ChannelStatus::from_status(byte), Some(status));
            assert_eq!(status.code(), byte >> 4);
            assert_eq!(status.data_bytes(), data_bytes);
            assert_eq!(name(status), format!(r#""{kebab}""#));
        }
        for byte in [0x00, 0x7f, 0xf0, 0xf8, 0xff] {
            assert_eq!(ChannelStatus::from_status(byte), None);
        }
    }

    #[test]
    fn system_messages_follow_their_status_byte() {
        let common = [
            (
                0xf1,
                CommonStatus::TimeCodeQuarterFrame,
                "time-code-quarter-frame",
                1,
            ),
            (
                0xf2,
                CommonStatus::SongPositionPointer,
                "song-position-pointer",
                2,
            ),
            (0xf3, CommonStatus::SongSelect, "song-select", 1),
            (0xf6, CommonStatus::TuneRequest, "tune-request", 0),
        ];
        for (byte, status, kebab, data_bytes) in common {
            assert_eq!(CommonStatus::from_status(byte), Some(status));
            assert_eq!(status.byte(), byte);
            assert_eq!(status.data_bytes(), data_bytes);
            assert_eq!(name(status), format!(r#""{kebab}""#));
        }
        for byte in [0xf0, 0xf4, 0xf5, 0xf7, 0xf8, 0x90] {
            assert_eq!(CommonStatus::from_status(byte), None);
        }
        let realtime = [
            (0xf8, RealtimeStatus::Clock, "clock"),
            (0xf9, RealtimeStatus::Undefined, "undefined"),
            (0xfa, RealtimeStatus::Start, "start"),
            (0xfb, RealtimeStatus::Continue, "continue"),
            (0xfc, RealtimeStatus::Stop, "stop"),
            (0xfd, RealtimeStatus::Undefined, "undefined"),
            (0xfe, RealtimeStatus::ActiveSensing, "active-sensing"),
            (0xff, RealtimeStatus::Reset, "reset"),
        ];
        for (byte, status, kebab) in realtime {
            assert_eq!(RealtimeStatus::from_status(byte), Some(status));
            assert_eq!(name(status), format!(r#""{kebab}""#));
        }
        assert_eq!(RealtimeStatus::from_status(0xf7), None);
    }

    #[test]
    fn reasons_are_kebab_case() {
        assert_eq!(name(EndReason::EndOfExclusive), r#""end-of-exclusive""#);
        assert_eq!(name(EndReason::Interrupted), r#""interrupted""#);
        assert_eq!(name(EndReason::Aborted), r#""aborted""#);
        assert_eq!(name(ErrorReason::UnexpectedData), r#""unexpected-data""#);
        assert_eq!(name(ErrorReason::Incomplete), r#""incomplete""#);
        assert_eq!(name(ErrorReason::Aborted), r#""aborted""#);
        assert_eq!(name(ErrorReason::UndefinedStatus), r#""undefined-status""#);
        assert_eq!(
            MidiEvent::Error {
                reason: ErrorReason::Incomplete,
                byte: 0x90,
            }
            .message(),
            None
        );
        assert_eq!(
            MidiEvent::SysexEnd {
                message: MessageId(3),
                bytes: 0,
                reason: EndReason::Aborted,
            }
            .message(),
            Some(MessageId(3))
        );
    }
}
