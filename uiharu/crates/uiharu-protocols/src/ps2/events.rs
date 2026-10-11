// SPDX-License-Identifier: Apache-2.0

//! The `ps2.events` contract: the bytes a PS/2 keyboard or mouse and its host exchanged, frame
//! by frame, in both directions.
//!
//! The stream is unframed: a frame is one event, decided at its last clock edge, so there is no
//! `begin` or `end`. A frame that could not be read to its end is an `error` instead.

use serde::{Deserialize, Serialize};
use uiharu_core::{EventSchema, SchemaId};

/// The schema of PS/2 event streams.
pub const PS2_EVENTS: SchemaId = SchemaId::new("ps2.events", 1);

/// Which side sent a frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Direction {
    /// The keyboard or mouse sent it, clocking it out itself.
    DeviceToHost,
    /// The host sent it after a request to send, the device clocking it in.
    HostToDevice,
}

/// Why a frame could not be read.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ErrorReason {
    /// The device clocked a frame whose start bit was 1.
    Start,
    /// The frame stopped before its last clock: the host took the clock, the clock paused, or
    /// a gap or the end of the samples cut it off.
    Aborted,
}

/// What travels on a PS/2 event stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum Ps2Event {
    /// A frame: a start bit, eight data bits least significant first, an odd parity bit and a
    /// stop bit, and for a frame the host sent the device's acknowledge.
    Frame {
        /// Who sent it.
        direction: Direction,
        /// The data bits, the first one on the line as bit 0.
        value: u8,
        /// Whether the parity bit makes the number of 1s among the data and parity bits odd.
        parity_ok: bool,
        /// Whether the stop bit was 1.
        stop_ok: bool,
        /// For a frame the host sent, whether the device pulled the data line low on the clock
        /// after the stop bit; `None` for a frame the device sent, which has no acknowledge.
        ack: Option<bool>,
    },
    /// A frame that could not be read.
    Error {
        /// What went wrong.
        reason: ErrorReason,
    },
}

impl EventSchema for Ps2Event {
    const SCHEMA: SchemaId = PS2_EVENTS;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn json(event: &Ps2Event) -> String {
        serde_json::to_string(event).expect("serializes")
    }

    #[test]
    fn serializes_as_the_contract_spells_it() {
        assert_eq!(
            json(&Ps2Event::Frame {
                direction: Direction::DeviceToHost,
                value: 0x1c,
                parity_ok: true,
                stop_ok: true,
                ack: None,
            }),
            r#"{"kind":"frame","direction":"device-to-host","value":28,"parityOk":true,"stopOk":true,"ack":null}"#
        );
        assert_eq!(
            json(&Ps2Event::Frame {
                direction: Direction::HostToDevice,
                value: 0xed,
                parity_ok: false,
                stop_ok: true,
                ack: Some(true),
            }),
            r#"{"kind":"frame","direction":"host-to-device","value":237,"parityOk":false,"stopOk":true,"ack":true}"#
        );
        assert_eq!(
            json(&Ps2Event::Error {
                reason: ErrorReason::Start
            }),
            r#"{"kind":"error","reason":"start"}"#
        );
        assert_eq!(
            json(&Ps2Event::Error {
                reason: ErrorReason::Aborted
            }),
            r#"{"kind":"error","reason":"aborted"}"#
        );
    }
}
