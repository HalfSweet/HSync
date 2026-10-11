// SPDX-License-Identifier: Apache-2.0

//! The `spi.events` contract: what an SPI decoder emits and what decoders built on SPI read.

use serde::{Deserialize, Serialize};
use uiharu_core::{EventSchema, SchemaId};

/// The schema of SPI event streams.
pub const SPI_EVENTS: SchemaId = SchemaId::new("spi.events", 1);

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
    CsRelease,
    /// The samples ended or were lost while chip select was still asserted.
    Aborted,
}

/// What travels on an SPI event stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum SpiEvent {
    /// Chip select was asserted.
    Begin {
        /// The transaction that begins.
        tx: TxId,
        /// Chip select was already asserted when the samples began (at the start of the
        /// capture or after a gap): earlier words of this transaction were not seen.
        partial: bool,
    },
    /// A word was shifted on both lines; a line that is not connected reads `None`.
    Word {
        /// The transaction the word belongs to.
        tx: TxId,
        /// The word on MOSI.
        mosi: Option<u8>,
        /// The word on MISO.
        miso: Option<u8>,
    },
    /// The transaction ended.
    End {
        /// The transaction that ends.
        tx: TxId,
        /// Why it ended.
        reason: EndReason,
    },
}

impl SpiEvent {
    /// The transaction the event belongs to.
    pub fn tx(&self) -> TxId {
        match self {
            SpiEvent::Begin { tx, .. } | SpiEvent::Word { tx, .. } | SpiEvent::End { tx, .. } => {
                *tx
            }
        }
    }
}

impl EventSchema for SpiEvent {
    const SCHEMA: SchemaId = SPI_EVENTS;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_as_the_contract_spells_it() {
        let json = serde_json::to_string(&SpiEvent::Begin {
            tx: TxId(3),
            partial: false,
        })
        .expect("serializes");
        assert_eq!(json, r#"{"kind":"begin","tx":3,"partial":false}"#);
        let json = serde_json::to_string(&SpiEvent::Word {
            tx: TxId(3),
            mosi: Some(3),
            miso: None,
        })
        .expect("serializes");
        assert_eq!(json, r#"{"kind":"word","tx":3,"mosi":3,"miso":null}"#);
        let json = serde_json::to_string(&SpiEvent::End {
            tx: TxId(3),
            reason: EndReason::CsRelease,
        })
        .expect("serializes");
        assert_eq!(json, r#"{"kind":"end","tx":3,"reason":"cs-release"}"#);
    }
}
