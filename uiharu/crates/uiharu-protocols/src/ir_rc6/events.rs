// SPDX-License-Identifier: Apache-2.0

//! The `rc6.events` contract: what an RC-6 decoder emits for one infrared receiver.
//!
//! The stream is unframed: a frame, or a frame that went wrong, is one event, decided when its
//! last half bit is certain. There is no `begin` or `end`, and nothing to abort: a frame cut off
//! by a gap or by the end of the samples is not reported.

use serde::{Deserialize, Serialize};
use uiharu_core::{EventSchema, SchemaId};

/// The schema of RC-6 event streams.
pub const IR_RC6_EVENTS: SchemaId = SchemaId::new("rc6.events", 1);

/// Where a frame went wrong: the field of the bit being read, or the mode it announced.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ErrorReason {
    /// The leader: its mark longer than 2.667 ms allows, or its space not 889 µs.
    Leader,
    /// The start bit.
    Start,
    /// The three mode bits.
    Mode,
    /// The trailer bit, the double-length bit that carries the toggle.
    Toggle,
    /// The eight control bits, the address.
    Address,
    /// The eight information bits, the command, the last one's second half included.
    Command,
    /// The mode bits name a mode other than 0, which the decoder does not read: the mode is
    /// the low three bits of `raw`.
    UnsupportedMode,
}

/// What travels on an RC-6 event stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum Rc6Event {
    /// A whole frame: the leader, the start bit, three mode bits, the trailer bit, eight
    /// control bits and eight information bits.
    Frame {
        /// The mode, from the mode bits: 0, the only mode the decoder reads.
        mode: u8,
        /// The trailer bit, which the remote control inverts at every new key press.
        toggle: bool,
        /// The control field: the address of the device.
        address: u8,
        /// The information field: the command.
        command: u8,
    },
    /// A frame whose leader mark was seen but which could not be read to its end.
    Error {
        /// Where it went wrong.
        reason: ErrorReason,
        /// The bits read before it went wrong, 0 to 21: a bit is read at its middle.
        bits: u8,
        /// Those bits, in the order received from the most significant: the start bit is bit
        /// `bits - 1`, the last bit read is bit 0.
        raw: u32,
    },
}

impl EventSchema for Rc6Event {
    const SCHEMA: SchemaId = IR_RC6_EVENTS;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn json(event: &Rc6Event) -> String {
        serde_json::to_string(event).expect("serializes")
    }

    #[test]
    fn serializes_as_the_contract_spells_it() {
        assert_eq!(
            json(&Rc6Event::Frame {
                mode: 0,
                toggle: true,
                address: 0,
                command: 12,
            }),
            r#"{"kind":"frame","mode":0,"toggle":true,"address":0,"command":12}"#
        );
        assert_eq!(
            json(&Rc6Event::Error {
                reason: ErrorReason::UnsupportedMode,
                bits: 4,
                raw: 0b1110,
            }),
            r#"{"kind":"error","reason":"unsupported-mode","bits":4,"raw":14}"#
        );
        for (reason, name) in [
            (ErrorReason::Leader, "leader"),
            (ErrorReason::Start, "start"),
            (ErrorReason::Mode, "mode"),
            (ErrorReason::Toggle, "toggle"),
            (ErrorReason::Address, "address"),
            (ErrorReason::Command, "command"),
        ] {
            let text = json(&Rc6Event::Error {
                reason,
                bits: 0,
                raw: 0,
            });
            assert!(text.contains(&format!(r#""reason":"{name}""#)), "{text}");
        }
    }

    #[test]
    fn reads_back_what_it_writes() {
        let event = Rc6Event::Frame {
            mode: 0,
            toggle: false,
            address: 4,
            command: 0x5c,
        };
        let back: Rc6Event = serde_json::from_str(&json(&event)).expect("parses");
        assert_eq!(back, event);
    }
}
