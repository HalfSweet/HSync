// SPDX-License-Identifier: Apache-2.0

//! The `am230x.events` contract: the readings an AM2302/DHT22 or DHT11 humidity and
//! temperature sensor sent on its single data line, as raw registers.
//!
//! The stream is unframed: a reading is one event, decided at the sensor's final release, so
//! there is no `begin` or `end`. An exchange that went wrong is an `error` instead; one that a
//! gap or the end of the samples cut off is not reported.

use serde::{Deserialize, Serialize};
use uiharu_core::{EventSchema, SchemaId};

/// The schema of AM230x event streams.
pub const AM230X_EVENTS: SchemaId = SchemaId::new("am230x.events", 1);

/// Why an exchange could not be read.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ErrorReason {
    /// A low pulse on an idle line that is no start signal: shorter than the model's start
    /// signal, such as a sensor's response or bits whose start signal was not seen.
    Start,
    /// After a start signal, the host's release or the sensor's response low or high fell
    /// outside its window.
    Response,
    /// A bit's low or high, or the sensor's final low, fell outside its window.
    Bit,
    /// The line went idle before the fortieth bit: a bit's high outlasted the window of a 1.
    Count,
}

/// What travels on an AM230x event stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum Am230xEvent {
    /// A reading: the five bytes the sensor sent, most significant bit first.
    Reading {
        /// The humidity's high and low byte, the temperature's high and low byte, and the
        /// checksum.
        bytes: [u8; 5],
        /// Whether the checksum is the sum of the four data bytes, modulo 256.
        checksum_ok: bool,
        /// The humidity register, the first two bytes.
        humidity_raw: u16,
        /// The temperature register, the next two bytes.
        temperature_raw: u16,
    },
    /// An exchange that could not be read; the pulses after it up to the next start signal are
    /// skipped.
    Error {
        /// What went wrong.
        reason: ErrorReason,
    },
}

impl EventSchema for Am230xEvent {
    const SCHEMA: SchemaId = AM230X_EVENTS;
}

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
