// SPDX-License-Identifier: Apache-2.0

//! The `lin.events` contract: the frames of a LIN bus, as a LIN decoder emits them and decoders
//! of the signals or of the diagnostic transport layer read them.
//!
//! A frame is framed like any bus transfer: every event carries the frame's id, and an `end`
//! closes it with the reason. Its first event is the header — or an `error`, when the byte after
//! the break field is not a sync byte — then come the response's data and checksum. Bytes are
//! not interpreted: which signals a frame carries is in the cluster's description, not on the
//! bus.

use serde::{Deserialize, Serialize};
use uiharu_core::{EventSchema, SchemaId};

/// The schema of LIN event streams.
pub const LIN_EVENTS: SchemaId = SchemaId::new("lin.events", 1);

/// The most data bytes a frame carries, and so the most one `data` event carries.
pub const DATA_BYTES: usize = 8;

/// A frame, numbered from 1 by the decoder.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct FrameId(pub u64);

/// Something wrong in a frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ErrorReason {
    /// The byte after the break field is not a sync byte field, the data 0x55 with its stop bit:
    /// the header is not recognized, and the frame ends.
    Sync,
    /// The parity bits of the protected identifier do not match its frame identifier.
    Parity,
    /// A byte of the frame has a framing error: its stop bit was dominant.
    Framing,
    /// The checksum is neither the classic nor the enhanced one; in a diagnostic frame (60 or
    /// 61), which always carries the classic checksum, it is not that one.
    Checksum,
    /// The response ended too soon: after one byte, which leaves no room for a data byte and a
    /// checksum, or, with the length taken from the identifier, before that many data bytes and
    /// a checksum.
    TooShort,
    /// A tenth byte came in the response: more than eight data bytes and a checksum. The frame
    /// ends.
    TooLong,
}

/// Why a frame ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EndReason {
    /// The response, its data and its checksum, came without an error.
    Complete,
    /// No byte followed the header before the next break field.
    NoResponse,
    /// An `error` was reported for the frame.
    Error,
    /// The samples ended or were lost before the frame did.
    Aborted,
}

/// What travels on a LIN event stream.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum LinEvent {
    /// A frame's header: its break field, sync byte and protected identifier.
    Header {
        /// The frame.
        frame: FrameId,
        /// The protected identifier as it was sent.
        pid: u8,
        /// The frame identifier, bits 0 to 5 of the protected identifier: 0 to 59 for frames
        /// that carry signals, 60 for the master request frame, 61 for the slave response frame,
        /// 62 and 63 reserved.
        id: u8,
        /// Whether the parity bits, bits 6 and 7, match the frame identifier.
        pid_ok: bool,
    },
    /// Data bytes of the response, in the order they were sent: data 1 first.
    Data {
        /// The frame.
        frame: FrameId,
        /// The position of the first byte, from 0 for data 1.
        offset: u8,
        /// At most [`DATA_BYTES`] bytes.
        bytes: Vec<u8>,
    },
    /// The last byte of the response, its checksum, reported when the frame ends.
    Checksum {
        /// The frame.
        frame: FrameId,
        /// The checksum as it was sent.
        value: u8,
        /// Whether it is the classic checksum, over the data bytes.
        classic_ok: bool,
        /// Whether it is the enhanced checksum, over the protected identifier and the data
        /// bytes.
        enhanced_ok: bool,
    },
    /// Something wrong in the frame, reported as soon as it is seen.
    Error {
        /// The frame.
        frame: FrameId,
        /// What is wrong.
        reason: ErrorReason,
    },
    /// The frame ended.
    End {
        /// The frame.
        frame: FrameId,
        /// Why it ended.
        reason: EndReason,
    },
}

impl LinEvent {
    /// The frame the event belongs to.
    pub fn frame(&self) -> FrameId {
        match self {
            LinEvent::Header { frame, .. }
            | LinEvent::Data { frame, .. }
            | LinEvent::Checksum { frame, .. }
            | LinEvent::Error { frame, .. }
            | LinEvent::End { frame, .. } => *frame,
        }
    }
}

impl EventSchema for LinEvent {
    const SCHEMA: SchemaId = LIN_EVENTS;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_as_the_contract_spells_it() {
        let json = |event: &LinEvent| serde_json::to_string(event).expect("serializes");
        assert_eq!(
            json(&LinEvent::Header {
                frame: FrameId(3),
                pid: 0x50,
                id: 0x10,
                pid_ok: true,
            }),
            r#"{"kind":"header","frame":3,"pid":80,"id":16,"pidOk":true}"#
        );
        assert_eq!(
            json(&LinEvent::Data {
                frame: FrameId(3),
                offset: 0,
                bytes: vec![1, 2],
            }),
            r#"{"kind":"data","frame":3,"offset":0,"bytes":[1,2]}"#
        );
        assert_eq!(
            json(&LinEvent::Checksum {
                frame: FrameId(3),
                value: 0xe6,
                classic_ok: true,
                enhanced_ok: false,
            }),
            r#"{"kind":"checksum","frame":3,"value":230,"classicOk":true,"enhancedOk":false}"#
        );
        let errors = [
            (ErrorReason::Sync, "sync"),
            (ErrorReason::Parity, "parity"),
            (ErrorReason::Framing, "framing"),
            (ErrorReason::Checksum, "checksum"),
            (ErrorReason::TooShort, "too-short"),
            (ErrorReason::TooLong, "too-long"),
        ];
        for (reason, name) in errors {
            assert_eq!(
                json(&LinEvent::Error {
                    frame: FrameId(3),
                    reason,
                }),
                format!(r#"{{"kind":"error","frame":3,"reason":"{name}"}}"#)
            );
        }
        let ends = [
            (EndReason::Complete, "complete"),
            (EndReason::NoResponse, "no-response"),
            (EndReason::Error, "error"),
            (EndReason::Aborted, "aborted"),
        ];
        for (reason, name) in ends {
            assert_eq!(
                json(&LinEvent::End {
                    frame: FrameId(3),
                    reason,
                }),
                format!(r#"{{"kind":"end","frame":3,"reason":"{name}"}}"#)
            );
        }
        assert_eq!(
            LinEvent::End {
                frame: FrameId(9),
                reason: EndReason::Aborted
            }
            .frame(),
            FrameId(9)
        );
    }
}
