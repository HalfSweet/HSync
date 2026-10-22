// SPDX-License-Identifier: Apache-2.0

//! Shared SWIM observations. The exact event payload is generated from the TS physical producer.

use uiharu_core::{EventSchema, SchemaId};
pub use uiharu_dsl_protocols::swim_stm8::{Direction, ErrorReason, Field, SwimEvent};

/// The SWIM event contract, preserving its established schema identity.
pub const SWIM_EVENTS: SchemaId = <SwimEvent as EventSchema>::SCHEMA;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_round_trip_with_the_documented_field_names() {
        let events = [
            SwimEvent::Command {
                value: 1,
                parity_ok: true,
                ack: false,
            },
            SwimEvent::Byte {
                direction: Direction::TargetToHost,
                field: Field::Data,
                value: 0xa5,
                parity_ok: false,
                ack: true,
                address: Some(0x123456),
            },
            SwimEvent::Reset {},
            SwimEvent::Entry {},
            SwimEvent::Error {
                reason: ErrorReason::Aborted,
                partial: true,
            },
        ];
        for event in events {
            let json = serde_json::to_string(&event).expect("serializes");
            assert_eq!(
                serde_json::from_str::<SwimEvent>(&json).expect("parses"),
                event
            );
        }
        assert_eq!(
            serde_json::to_value(events[0]).expect("serializes"),
            serde_json::json!({"kind":"command","value":1,"parityOk":true,"ack":false})
        );
    }
}
