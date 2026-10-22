// SPDX-License-Identifier: Apache-2.0

//! Exact MIDI contract types are generated from `dsl/contracts/midi.ts`.
use uiharu_core::SchemaId;
pub use uiharu_dsl_protocols::midi::{
    ChannelStatus, CommonStatus, EndReason, ErrorReason, MidiEvent, RealtimeStatus,
};
/// The schema of MIDI event streams.
pub const MIDI_EVENTS: SchemaId = SchemaId::new("midi.events", 1);
/// The maximum SysEx batch size.
pub const SYSEX_BATCH: usize = 32;
/// A message number, starting at one.
pub type MessageId = u64;
/// Helpers for the generated ChannelStatus wire type.
pub trait ChannelStatusExt: Sized {
    /// Read or describe the MIDI wire value.
    fn from_status(status: u8) -> Option<Self>;
    /// Read or describe the MIDI wire value.
    fn code(self) -> u8;
    /// Read or describe the MIDI wire value.
    fn data_bytes(self) -> u8;
}
impl ChannelStatusExt for ChannelStatus {
    /// The message a channel status byte, 0x80 to 0xEF, names; `None` for any other byte.
    fn from_status(status: u8) -> Option<Self> {
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
    fn code(self) -> u8 {
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
    fn data_bytes(self) -> u8 {
        match self {
            ChannelStatus::ProgramChange | ChannelStatus::ChannelPressure => 1,
            _ => 2,
        }
    }
}

/// Helpers for the generated CommonStatus wire type.
pub trait CommonStatusExt: Sized {
    /// Read or describe the MIDI wire value.
    fn from_status(status: u8) -> Option<Self>;
    /// Read or describe the MIDI wire value.
    fn byte(self) -> u8;
    /// Read or describe the MIDI wire value.
    fn data_bytes(self) -> u8;
}
impl CommonStatusExt for CommonStatus {
    /// The message a system common status byte names; `None` for any other byte, and for the
    /// undefined 0xF4 and 0xF5 and the end of exclusive, 0xF7.
    fn from_status(status: u8) -> Option<Self> {
        match status {
            0xf1 => Some(CommonStatus::TimeCodeQuarterFrame),
            0xf2 => Some(CommonStatus::SongPositionPointer),
            0xf3 => Some(CommonStatus::SongSelect),
            0xf6 => Some(CommonStatus::TuneRequest),
            _ => None,
        }
    }

    /// The message's status byte.
    fn byte(self) -> u8 {
        match self {
            CommonStatus::TimeCodeQuarterFrame => 0xf1,
            CommonStatus::SongPositionPointer => 0xf2,
            CommonStatus::SongSelect => 0xf3,
            CommonStatus::TuneRequest => 0xf6,
        }
    }

    /// The data bytes the message carries.
    fn data_bytes(self) -> u8 {
        match self {
            CommonStatus::TimeCodeQuarterFrame | CommonStatus::SongSelect => 1,
            CommonStatus::SongPositionPointer => 2,
            CommonStatus::TuneRequest => 0,
        }
    }
}

/// Helpers for the generated RealtimeStatus wire type.
pub trait RealtimeStatusExt: Sized {
    /// Read or describe the MIDI wire value.
    fn from_status(status: u8) -> Option<Self>;
}
impl RealtimeStatusExt for RealtimeStatus {
    /// The message a real-time status byte, 0xF8 to 0xFF, names; `None` for any other byte.
    fn from_status(status: u8) -> Option<Self> {
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

/// Helpers for the generated MidiEvent wire type.
pub trait MidiEventExt: Sized {
    /// Read or describe the MIDI wire value.
    fn message(&self) -> Option<MessageId>;
}
impl MidiEventExt for MidiEvent {
    /// The message the event belongs to; `None` for real-time messages and errors.
    fn message(&self) -> Option<MessageId> {
        match self {
            MidiEvent::ChannelMessage { message, .. }
            | MidiEvent::SystemCommon { message, .. }
            | MidiEvent::Sysex { message, .. }
            | MidiEvent::SysexEnd { message, .. } => Some(*message),
            MidiEvent::Realtime { .. } | MidiEvent::Error { .. } => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Serialize;

    fn json(event: &MidiEvent) -> String {
        serde_json::to_string(event).expect("serializes")
    }

    #[test]
    fn serializes_as_the_contract_spells_it() {
        assert_eq!(
            json(&MidiEvent::ChannelMessage {
                message: 4,
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
                message: 5,
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
                message: 6,
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
                message: 7,
                offset: 32,
                bytes: vec![0x43, 0x10],
            }),
            r#"{"kind":"sysex","message":7,"offset":32,"bytes":[67,16]}"#
        );
        assert_eq!(
            json(&MidiEvent::SysexEnd {
                message: 7,
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
                message: 3,
                bytes: 0,
                reason: EndReason::Aborted,
            }
            .message(),
            Some(3)
        );
    }
}
