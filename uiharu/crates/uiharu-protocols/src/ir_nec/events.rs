// SPDX-License-Identifier: Apache-2.0

//! The `nec.events` contract: what an NEC decoder emits for one infrared receiver.
//!
//! The stream is unframed: a frame, a repeat code or a frame that could not be read is one
//! event, decided when its last mark or space is certain. There is no `begin` or `end`, and
//! nothing to abort: a frame cut off by a gap or by the end of the samples is not reported.

use serde::{Deserialize, Serialize};
use uiharu_core::{EventSchema, SchemaId};

/// The schema of NEC event streams.
pub const IR_NEC_EVENTS: SchemaId = SchemaId::new("nec.events", 1);

/// Where a frame that began with a leader went wrong.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ErrorReason {
    /// The leader: its mark longer than 9 ms allows, or its space neither the 4.5 ms of a frame
    /// nor the 2.25 ms of a repeat code.
    Leader,
    /// A data bit: its mark not 562.5 µs long, or its space shorter than a 0's or between a 0's
    /// and a 1's.
    Bit,
    /// The frame stopped before its 32nd bit: a space longer than a 1's.
    Count,
    /// The final mark of a frame or a repeat code, not 562.5 µs long.
    Trailer,
}

/// What travels on an NEC event stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum NecEvent {
    /// A whole frame: the leader, 32 data bits and the final mark.
    Frame {
        /// The address: the first byte, or with `extended` the first two bytes, the first one
        /// the low byte.
        address: u16,
        /// The command: the third byte.
        command: u8,
        /// The second byte is not the inverse of the first, so the two make a 16-bit address
        /// (extended NEC).
        extended: bool,
        /// The fourth byte is the inverse of the third, as the format calls for.
        inverse_ok: bool,
        /// The 32 data bits as they were received, the first as bit 0: the four bytes in order
        /// from the low byte up.
        raw: u32,
    },
    /// A repeat code: the leader mark, the short leader space and the final mark, which a
    /// remote control sends while a key stays pressed.
    Repeat,
    /// A frame or repeat code whose leader mark was seen but which could not be read to its
    /// end.
    Error {
        /// Where it went wrong.
        reason: ErrorReason,
        /// The data bits read before it went wrong, 0 to 32.
        bits: u8,
        /// Those bits, the first as bit 0.
        raw: u32,
    },
}

impl EventSchema for NecEvent {
    const SCHEMA: SchemaId = IR_NEC_EVENTS;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn json(event: &NecEvent) -> String {
        serde_json::to_string(event).expect("serializes")
    }

    #[test]
    fn serializes_as_the_contract_spells_it() {
        assert_eq!(
            json(&NecEvent::Frame {
                address: 0x00,
                command: 0x45,
                extended: false,
                inverse_ok: true,
                raw: 0xba45_ff00,
            }),
            r#"{"kind":"frame","address":0,"command":69,"extended":false,"inverseOk":true,"raw":3125149440}"#
        );
        assert_eq!(json(&NecEvent::Repeat), r#"{"kind":"repeat"}"#);
        assert_eq!(
            json(&NecEvent::Error {
                reason: ErrorReason::Count,
                bits: 13,
                raw: 0x1abc,
            }),
            r#"{"kind":"error","reason":"count","bits":13,"raw":6844}"#
        );
        for (reason, name) in [
            (ErrorReason::Leader, "leader"),
            (ErrorReason::Bit, "bit"),
            (ErrorReason::Trailer, "trailer"),
        ] {
            let text = json(&NecEvent::Error {
                reason,
                bits: 0,
                raw: 0,
            });
            assert!(text.contains(&format!(r#""reason":"{name}""#)), "{text}");
        }
    }

    #[test]
    fn reads_back_what_it_writes() {
        let event = NecEvent::Frame {
            address: 0x1234,
            command: 0x56,
            extended: true,
            inverse_ok: false,
            raw: 0x0056_1234,
        };
        let back: NecEvent = serde_json::from_str(&json(&event)).expect("parses");
        assert_eq!(back, event);
    }
}
