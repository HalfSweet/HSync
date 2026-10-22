// SPDX-License-Identifier: Apache-2.0

//! The `ps2.events` contract: the bytes a PS/2 keyboard or mouse and its host exchanged, frame
//! by frame, in both directions.
//!
//! The stream is unframed: a frame is one event, decided at its last clock edge, so there is no
//! `begin` or `end`. A frame that could not be read to its end is an `error` instead.

pub use uiharu_dsl_protocols::ps2::{Direction, ErrorReason, Ps2Event};

/// The schema of PS/2 event streams.
pub const PS2_EVENTS: uiharu_core::SchemaId = <Ps2Event as uiharu_core::EventSchema>::SCHEMA;

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
