// SPDX-License-Identifier: Apache-2.0

//! The `can.events` contract: what a CAN decoder emits and what decoders built on CAN read.

use serde::{Deserialize, Serialize};
use uiharu_core::{EventSchema, SchemaId};

/// The schema of CAN event streams.
pub const CAN_EVENTS: SchemaId = SchemaId::new("can.events", 1);

/// The most bytes one data event carries.
pub const DATA_BATCH: usize = 32;

/// A frame: numbered from 1 by the decoder, in the order frames begin on the bus.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct FrameId(pub u64);

/// Why a frame ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EndReason {
    /// The end of frame was sent: the frame is whole.
    Eof,
    /// Six consecutive dominant bits, an active error flag, destroyed the frame.
    ErrorFrame,
    /// An overload frame: a dominant bit at the last bit of the end of frame or in the first
    /// two bits of the intermission. Overload frames are frames of their own, with nothing
    /// between their begin and their end.
    Overload,
    /// A sixth equal bit where a stuff bit was due, or a fixed stuff bit equal to the bit
    /// before it.
    StuffError,
    /// A dominant bit where the frame's form calls for a recessive one: the CRC delimiter, the
    /// ACK delimiter or the end of frame.
    FormError,
    /// A frame the decoder cannot follow: an FD frame with a recessive `res` bit (a protocol
    /// exception), or one that switches its bit rate when no data bit rate is configured.
    Unsupported,
    /// The samples ended or were lost while the frame was under way, or the frame was already
    /// under way when they began.
    Aborted,
}

/// What travels on a CAN event stream.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum CanEvent {
    /// A frame began.
    Begin {
        /// The frame that begins.
        frame: FrameId,
        /// The frame was already under way when the samples began or resumed after a gap: none
        /// of it is decoded, and its end is `aborted`.
        partial: bool,
    },
    /// The identifier.
    Id {
        /// The frame.
        frame: FrameId,
        /// The identifier: 11 bits in the base format, 29 in the extended format.
        id: u32,
        /// Whether the frame is in the extended format (IDE recessive).
        extended: bool,
    },
    /// The control field, with the remote request bit that precedes it.
    Control {
        /// The frame.
        frame: FrameId,
        /// A remote frame (RTR recessive in a classic frame): it has no data field.
        remote: bool,
        /// An FD frame (FDF recessive).
        fd: bool,
        /// The data phase runs at the data bit rate (BRS recessive); FD frames only.
        brs: bool,
        /// The transmitter is error passive (ESI recessive); FD frames only.
        esi: bool,
        /// The data length code.
        dlc: u8,
        /// The number of data bytes the code stands for; in a remote frame, the length of the
        /// data frame it requests.
        length: u8,
    },
    /// Data bytes, in order.
    Data {
        /// The frame.
        frame: FrameId,
        /// The position of the first byte in the data field.
        offset: u32,
        /// At most [`DATA_BATCH`] bytes.
        bytes: Vec<u8>,
    },
    /// The CRC field as it was received, and whether it matches the frame.
    Crc {
        /// The frame.
        frame: FrameId,
        /// The CRC sequence: 15 bits in classic frames, 17 or 21 in FD frames.
        value: u32,
        /// Whether the sequence matches the one computed from the frame and, in FD frames,
        /// the stuff count matches the dynamic stuff bits and its parity is right.
        ok: bool,
        /// The stuff count of an FD frame, decoded from its Gray code: the number of dynamic
        /// stuff bits modulo 8 as the transmitter counted them.
        stuff_count: Option<u8>,
    },
    /// The ACK slot.
    Ack {
        /// The frame.
        frame: FrameId,
        /// Whether a receiver acknowledged the frame with a dominant bit.
        acked: bool,
    },
    /// The frame ended.
    End {
        /// The frame that ends.
        frame: FrameId,
        /// Why it ended.
        reason: EndReason,
    },
}

impl CanEvent {
    /// The frame the event belongs to.
    pub fn frame(&self) -> FrameId {
        match self {
            CanEvent::Begin { frame, .. }
            | CanEvent::Id { frame, .. }
            | CanEvent::Control { frame, .. }
            | CanEvent::Data { frame, .. }
            | CanEvent::Crc { frame, .. }
            | CanEvent::Ack { frame, .. }
            | CanEvent::End { frame, .. } => *frame,
        }
    }
}

impl EventSchema for CanEvent {
    const SCHEMA: SchemaId = CAN_EVENTS;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn json(event: &CanEvent) -> String {
        serde_json::to_string(event).expect("serializes")
    }

    #[test]
    fn serializes_as_the_contract_spells_it() {
        let frame = FrameId(7);
        assert_eq!(
            json(&CanEvent::Begin {
                frame,
                partial: false
            }),
            r#"{"kind":"begin","frame":7,"partial":false}"#
        );
        assert_eq!(
            json(&CanEvent::Id {
                frame,
                id: 0x123,
                extended: false
            }),
            r#"{"kind":"id","frame":7,"id":291,"extended":false}"#
        );
        assert_eq!(
            json(&CanEvent::Control {
                frame,
                remote: false,
                fd: true,
                brs: true,
                esi: false,
                dlc: 9,
                length: 12
            }),
            r#"{"kind":"control","frame":7,"remote":false,"fd":true,"brs":true,"esi":false,"dlc":9,"length":12}"#
        );
        assert_eq!(
            json(&CanEvent::Data {
                frame,
                offset: 32,
                bytes: vec![1, 2]
            }),
            r#"{"kind":"data","frame":7,"offset":32,"bytes":[1,2]}"#
        );
        assert_eq!(
            json(&CanEvent::Crc {
                frame,
                value: 0x4599,
                ok: true,
                stuff_count: Some(3)
            }),
            r#"{"kind":"crc","frame":7,"value":17817,"ok":true,"stuffCount":3}"#
        );
        assert_eq!(
            json(&CanEvent::Crc {
                frame,
                value: 1,
                ok: false,
                stuff_count: None
            }),
            r#"{"kind":"crc","frame":7,"value":1,"ok":false,"stuffCount":null}"#
        );
        assert_eq!(
            json(&CanEvent::Ack { frame, acked: true }),
            r#"{"kind":"ack","frame":7,"acked":true}"#
        );
        assert_eq!(
            json(&CanEvent::End {
                frame,
                reason: EndReason::ErrorFrame
            }),
            r#"{"kind":"end","frame":7,"reason":"error-frame"}"#
        );
    }

    #[test]
    fn end_reasons_use_the_names_of_the_bus() {
        let names: Vec<String> = [
            EndReason::Eof,
            EndReason::ErrorFrame,
            EndReason::Overload,
            EndReason::StuffError,
            EndReason::FormError,
            EndReason::Unsupported,
            EndReason::Aborted,
        ]
        .iter()
        .map(|reason| serde_json::to_string(reason).expect("serializes"))
        .collect();
        assert_eq!(
            names,
            [
                r#""eof""#,
                r#""error-frame""#,
                r#""overload""#,
                r#""stuff-error""#,
                r#""form-error""#,
                r#""unsupported""#,
                r#""aborted""#
            ]
        );
    }

    #[test]
    fn every_event_names_its_frame() {
        let event = CanEvent::Ack {
            frame: FrameId(3),
            acked: false,
        };
        assert_eq!(event.frame(), FrameId(3));
        let back: CanEvent =
            serde_json::from_str(r#"{"kind":"crc","frame":2,"value":5,"ok":true,"stuffCount":1}"#)
                .expect("parses");
        assert_eq!(back.frame(), FrameId(2));
    }
}
