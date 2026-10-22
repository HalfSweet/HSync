// SPDX-License-Identifier: Apache-2.0

//! The `am230x.events` contract: the readings an AM2302/DHT22 or DHT11 humidity and
//! temperature sensor sent on its single data line, as raw registers.
//!
//! The stream is unframed: a reading is one event, decided at the sensor's final release, so
//! there is no `begin` or `end`. An exchange that went wrong is an `error` instead; one that a
//! gap or the end of the samples cut off is not reported.

use uiharu_core::SchemaId;

/// The schema of AM230x event streams.
pub const AM230X_EVENTS: SchemaId = SchemaId::new("am230x.events", 1);

pub use uiharu_dsl_protocols::am230x::{Am230xEvent, ErrorReason};

#[cfg(test)]
mod tests {
    use super::*;

    fn json(event: &Am230xEvent) -> String {
        serde_json::to_string(event).expect("serializes")
    }

    #[test]
    fn serializes_as_the_contract_spells_it() {
        assert_eq!(
            json(&Am230xEvent::Reading {
                bytes: [0x02, 0x8c, 0x01, 0x5f, 0xee],
                checksum_ok: true,
                humidity_raw: 652,
                temperature_raw: 351,
            }),
            r#"{"kind":"reading","bytes":[2,140,1,95,238],"checksumOk":true,"humidityRaw":652,"temperatureRaw":351}"#
        );
        for (reason, name) in [
            (ErrorReason::Start, "start"),
            (ErrorReason::Response, "response"),
            (ErrorReason::Bit, "bit"),
            (ErrorReason::Count, "count"),
        ] {
            assert_eq!(
                json(&Am230xEvent::Error { reason }),
                format!(r#"{{"kind":"error","reason":"{name}"}}"#)
            );
        }
    }

    #[test]
    fn reads_back_what_it_writes() {
        let event = Am230xEvent::Reading {
            bytes: [1, 2, 3, 4, 10],
            checksum_ok: true,
            humidity_raw: 0x0102,
            temperature_raw: 0x0304,
        };
        let back: Am230xEvent = serde_json::from_str(&json(&event)).expect("parses");
        assert_eq!(back, event);
    }
}
