// SPDX-License-Identifier: Apache-2.0

//! The `ir_rc5.events` contract: what an RC-5 decoder emits for one infrared receiver.
//!
//! The stream is unframed: a frame, or a frame that went wrong, is one event, decided when its
//! last half bit is certain. There is no `begin` or `end`, and nothing to abort: a frame cut off
//! by a gap or by the end of the samples is not reported.

use serde::{Deserialize, Serialize};
use uiharu_core::{EventSchema, SchemaId};

/// The schema of RC-5 event streams.
pub const IR_RC5_EVENTS: SchemaId = SchemaId::new("ir_rc5.events", 1);

/// The field of a frame in which it went wrong.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ErrorReason {
    /// The two start bits.
    Start,
    /// The toggle bit.
    Toggle,
    /// The five address bits.
    Address,
    /// The six command bits, the last one's second half included.
    Command,
}

/// What travels on an RC-5 event stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum Rc5Event {
    /// A whole frame: two start bits, the toggle bit, five address bits and six command bits.
    Frame {
        /// The toggle bit, which the remote control inverts at every new key press.
        toggle: bool,
        /// The address (system), 0 to 31.
        address: u8,
        /// The command, 0 to 127: the six command bits, and as bit 6 the inverse of the second
        /// start bit (RC-5X).
        command: u8,
        /// The second start bit is 0: the command is one of 64 to 127, which only extended
        /// RC-5 (RC-5X) sends.
        extended: bool,
    },
    /// A frame whose first edge was seen but which could not be read to its end.
    Error {
        /// The field in which it went wrong.
        reason: ErrorReason,
        /// The bits read before it went wrong, 1 to 14: a bit is read at its middle.
        bits: u8,
        /// Those bits, in the order received from the most significant: the first start bit
        /// is bit `bits - 1`, the last bit read is bit 0.
        raw: u16,
    },
}

impl EventSchema for Rc5Event {
    const SCHEMA: SchemaId = IR_RC5_EVENTS;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn json(event: &Rc5Event) -> String {
        serde_json::to_string(event).expect("serializes")
    }

    #[test]
    fn serializes_as_the_contract_spells_it() {
        assert_eq!(
            json(&Rc5Event::Frame {
                toggle: true,
                address: 5,
                command: 77,
                extended: true,
            }),
            r#"{"kind":"frame","toggle":true,"address":5,"command":77,"extended":true}"#
        );
        assert_eq!(
            json(&Rc5Event::Error {
                reason: ErrorReason::Address,
                bits: 4,
                raw: 0b1101,
            }),
            r#"{"kind":"error","reason":"address","bits":4,"raw":13}"#
        );
        for (reason, name) in [
            (ErrorReason::Start, "start"),
            (ErrorReason::Toggle, "toggle"),
            (ErrorReason::Command, "command"),
        ] {
            let text = json(&Rc5Event::Error {
                reason,
                bits: 1,
                raw: 1,
            });
            assert!(text.contains(&format!(r#""reason":"{name}""#)), "{text}");
        }
    }

    #[test]
    fn reads_back_what_it_writes() {
        let event = Rc5Event::Frame {
            toggle: false,
            address: 31,
            command: 12,
            extended: false,
        };
        let back: Rc5Event = serde_json::from_str(&json(&event)).expect("parses");
        assert_eq!(back, event);
    }
}
