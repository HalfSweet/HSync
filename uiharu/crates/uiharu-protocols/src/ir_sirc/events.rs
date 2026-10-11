// SPDX-License-Identifier: Apache-2.0

//! The `ir_sirc.events` contract: what a SIRC decoder emits for one infrared receiver.
//!
//! The stream is unframed: a frame, or a frame that went wrong, is one event, decided once the
//! silence after its last bit, or what went wrong, is certain. There is no `begin` or `end`,
//! and nothing to abort: a frame cut off by a gap or by the end of the samples, its silence
//! unseen, is not reported.

use serde::{Deserialize, Serialize};
use uiharu_core::{EventSchema, SchemaId};

/// The schema of SIRC event streams.
pub const IR_SIRC_EVENTS: SchemaId = SchemaId::new("ir_sirc.events", 1);

/// Where a frame that began with a start mark went wrong.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ErrorReason {
    /// The start: its mark longer than 2.4 ms allows, or the space after it not 600 µs.
    Start,
    /// A bit: its mark neither the 600 µs of a 0 nor the 1.2 ms of a 1, or the space after it
    /// shorter than 600 µs.
    Bit,
    /// The number of bits: the frame ended after other than 12, 15 or 20 bits, or a 21st bit
    /// began.
    Count,
}

/// What travels on a SIRC event stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum SircEvent {
    /// A whole frame: the start, then 12, 15 or 20 bits, least significant bit first.
    Frame {
        /// The length of the frame: 12, 15 or 20 bits.
        bits: u8,
        /// The command, the first seven bits.
        command: u8,
        /// The address, the next five bits (12 and 20 bits) or eight (15 bits).
        address: u8,
        /// The extended field of a 20-bit frame, its last eight bits; `None` in the shorter
        /// frames.
        extended: Option<u8>,
    },
    /// A frame whose start mark was seen but which could not be read to its end.
    Error {
        /// Where it went wrong.
        reason: ErrorReason,
        /// The bits read before it went wrong, 0 to 21: a bit is read at the end of its mark.
        bits: u8,
        /// Those bits, the first as bit 0.
        raw: u32,
    },
}

impl EventSchema for SircEvent {
    const SCHEMA: SchemaId = IR_SIRC_EVENTS;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn json(event: &SircEvent) -> String {
        serde_json::to_string(event).expect("serializes")
    }

    #[test]
    fn serializes_as_the_contract_spells_it() {
        assert_eq!(
            json(&SircEvent::Frame {
                bits: 12,
                command: 21,
                address: 1,
                extended: None,
            }),
            r#"{"kind":"frame","bits":12,"command":21,"address":1,"extended":null}"#
        );
        assert_eq!(
            json(&SircEvent::Frame {
                bits: 20,
                command: 0x7f,
                address: 0x1a,
                extended: Some(0xe2),
            }),
            r#"{"kind":"frame","bits":20,"command":127,"address":26,"extended":226}"#
        );
        assert_eq!(
            json(&SircEvent::Error {
                reason: ErrorReason::Count,
                bits: 13,
                raw: 0x1095,
            }),
            r#"{"kind":"error","reason":"count","bits":13,"raw":4245}"#
        );
        for (reason, name) in [(ErrorReason::Start, "start"), (ErrorReason::Bit, "bit")] {
            let text = json(&SircEvent::Error {
                reason,
                bits: 0,
                raw: 0,
            });
            assert!(text.contains(&format!(r#""reason":"{name}""#)), "{text}");
        }
    }

    #[test]
    fn reads_back_what_it_writes() {
        let event = SircEvent::Frame {
            bits: 15,
            command: 0x2a,
            address: 0x97,
            extended: None,
        };
        let back: SircEvent = serde_json::from_str(&json(&event)).expect("parses");
        assert_eq!(back, event);
    }
}
