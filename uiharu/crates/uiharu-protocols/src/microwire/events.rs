// SPDX-License-Identifier: Apache-2.0

//! The `microwire.events` contract: what a Microwire decoder emits and what decoders built on
//! Microwire read.
//!
//! A transaction is a chip select period. Within it every clock pulse is one `bit`, with the
//! level the controller drove on DI and the one the device drove on DO; the stream knows
//! nothing of the instructions those bits make.

use serde::{Deserialize, Serialize};
use uiharu_core::{EventSchema, SchemaId};

/// The schema of Microwire event streams.
pub const MICROWIRE_EVENTS: SchemaId = SchemaId::new("microwire.events", 1);

/// A transaction: one chip select period. Numbered from 1 by the decoder; 0 stands for the
/// unframed stream of a bus without chip select.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct TxId(pub u64);

/// Why a transaction ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EndReason {
    /// Chip select was released: the transaction is whole.
    Release,
    /// The samples ended or were lost while chip select was still asserted.
    Aborted,
}

/// What travels on a Microwire event stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum MicrowireEvent {
    /// Chip select was asserted.
    Begin {
        /// The transaction that begins.
        tx: TxId,
        /// Chip select was already asserted when the samples began (at the start of the
        /// capture or after a gap): earlier bits of this transaction were not seen.
        partial: bool,
    },
    /// A clock pulse: the bit the controller drove on DI, read on the rising edge, and the one
    /// the device drove on DO, read on the falling edge after it.
    Bit {
        /// The transaction the bit belongs to.
        tx: TxId,
        /// DI at the rising edge.
        di: bool,
        /// DO at the falling edge; `None` when DO is not connected.
        r#do: Option<bool>,
    },
    /// The transaction ended.
    End {
        /// The transaction that ends.
        tx: TxId,
        /// How many bits it carried.
        bits: u32,
        /// Why it ended.
        reason: EndReason,
        /// DO's level on the last sample of the transaction, before chip select was released
        /// or the samples stopped: a device's ready or busy state, for one; `None` when DO is
        /// not connected.
        r#do: Option<bool>,
    },
}

impl MicrowireEvent {
    /// The transaction the event belongs to.
    pub fn tx(&self) -> TxId {
        match self {
            MicrowireEvent::Begin { tx, .. }
            | MicrowireEvent::Bit { tx, .. }
            | MicrowireEvent::End { tx, .. } => *tx,
        }
    }
}

impl EventSchema for MicrowireEvent {
    const SCHEMA: SchemaId = MICROWIRE_EVENTS;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn json(event: &MicrowireEvent) -> String {
        serde_json::to_string(event).expect("serializes")
    }

    #[test]
    fn serializes_as_the_contract_spells_it() {
        let tx = TxId(4);
        assert_eq!(
            json(&MicrowireEvent::Begin { tx, partial: true }),
            r#"{"kind":"begin","tx":4,"partial":true}"#
        );
        assert_eq!(
            json(&MicrowireEvent::Bit {
                tx,
                di: true,
                r#do: None
            }),
            r#"{"kind":"bit","tx":4,"di":true,"do":null}"#
        );
        assert_eq!(
            json(&MicrowireEvent::Bit {
                tx,
                di: false,
                r#do: Some(true)
            }),
            r#"{"kind":"bit","tx":4,"di":false,"do":true}"#
        );
        assert_eq!(
            json(&MicrowireEvent::End {
                tx,
                bits: 25,
                reason: EndReason::Release,
                r#do: Some(false),
            }),
            r#"{"kind":"end","tx":4,"bits":25,"reason":"release","do":false}"#
        );
        assert_eq!(
            json(&MicrowireEvent::End {
                tx,
                bits: 3,
                reason: EndReason::Aborted,
                r#do: None,
            }),
            r#"{"kind":"end","tx":4,"bits":3,"reason":"aborted","do":null}"#
        );
        let back: MicrowireEvent =
            serde_json::from_str(r#"{"kind":"bit","tx":4,"di":true,"do":false}"#).expect("parses");
        assert_eq!(
            back,
            MicrowireEvent::Bit {
                tx,
                di: true,
                r#do: Some(false)
            }
        );
        assert_eq!(back.tx(), tx);
    }
}
