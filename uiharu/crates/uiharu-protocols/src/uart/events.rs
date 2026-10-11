// SPDX-License-Identifier: Apache-2.0

//! The `uart.events` contract: what a UART decoder emits for one line, and what decoders built
//! on asynchronous serial read.
//!
//! The stream is unframed: every character is one event, decided once its last stop bit has
//! been sampled, so there is no `begin` or `end` and nothing to abort. A character cut off by a
//! gap or by the end of the samples is not reported at all.

use serde::{Deserialize, Serialize};
use uiharu_core::{EventSchema, SchemaId};

/// The schema of UART event streams.
pub const UART_EVENTS: SchemaId = SchemaId::new("uart.events", 1);

/// What the parity bit of a character says.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ParityCheck {
    /// The parity bit is what the configured parity calls for.
    Ok,
    /// It is not.
    Error,
}

/// What travels on a UART event stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum UartEvent {
    /// A character: a start bit, the data bits, the parity bit if the line has one, and the stop
    /// bits.
    Frame {
        /// The data bits as a number. Least significant bit first, the first data bit is bit 0;
        /// most significant bit first, it is the highest of the configured data bits.
        value: u16,
        /// Whether the parity bit matched; `None` on a line without a parity bit.
        parity: Option<ParityCheck>,
        /// Whether every stop bit read mark (the idle level).
        framing_ok: bool,
    },
    /// The line was held at space (the active level) through a whole character, stop bits
    /// included, and then returned to mark.
    Break {
        /// How long the line was held at space: from the falling edge that looked like a start
        /// bit to the first tick back at mark.
        ticks: u64,
    },
}

impl EventSchema for UartEvent {
    const SCHEMA: SchemaId = UART_EVENTS;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_as_the_contract_spells_it() {
        let json = serde_json::to_string(&UartEvent::Frame {
            value: 0x41,
            parity: Some(ParityCheck::Ok),
            framing_ok: true,
        })
        .expect("serializes");
        assert_eq!(
            json,
            r#"{"kind":"frame","value":65,"parity":"ok","framingOk":true}"#
        );
        let json = serde_json::to_string(&UartEvent::Frame {
            value: 0x1ff,
            parity: None,
            framing_ok: false,
        })
        .expect("serializes");
        assert_eq!(
            json,
            r#"{"kind":"frame","value":511,"parity":null,"framingOk":false}"#
        );
        let json = serde_json::to_string(&UartEvent::Frame {
            value: 0,
            parity: Some(ParityCheck::Error),
            framing_ok: true,
        })
        .expect("serializes");
        assert!(json.contains(r#""parity":"error""#), "{json}");
        let json = serde_json::to_string(&UartEvent::Break { ticks: 1234 }).expect("serializes");
        assert_eq!(json, r#"{"kind":"break","ticks":1234}"#);
    }
}
