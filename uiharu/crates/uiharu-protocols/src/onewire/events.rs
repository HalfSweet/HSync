// SPDX-License-Identifier: Apache-2.0

//! The two 1-Wire contracts: `onewire.link`, the time slots the link layer reads from the
//! line, and `onewire.events`, the transactions the network layer makes of them.

use serde::{Deserialize, Serialize};
use uiharu_core::{EventSchema, SchemaId};

/// The schema of 1-Wire link-layer streams: resets, bits and bad slots.
pub const ONEWIRE_LINK: SchemaId = SchemaId::new("onewire.link", 1);

/// The schema of 1-Wire network-layer streams: transactions, their ROM level and their bytes.
pub const ONEWIRE_EVENTS: SchemaId = SchemaId::new("onewire.events", 1);

/// What travels on a 1-Wire link-layer stream: one event per low pulse the master started.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum LinkEvent {
    /// A reset pulse, and whether a slave answered it.
    Reset {
        /// Whether a presence pulse followed the reset.
        presence: bool,
    },
    /// A time slot. Read and write slots look alike on the line, so this is the bit the line
    /// carried, whichever side drove it.
    Bit {
        /// The bit: `true` for a 1.
        value: bool,
    },
    /// A low pulse that fits no window: longer than a slot without being a reset, a pulse in
    /// a presence window that is no presence pulse, or a slot whose sampling point fell into
    /// the next pulse.
    BadSlot {
        /// How long the line was low, in ticks.
        low_ticks: u64,
    },
}

impl EventSchema for LinkEvent {
    const SCHEMA: SchemaId = ONEWIRE_LINK;
}

/// Canonical generated network types. Transaction IDs are transparent numeric aliases.
pub use uiharu_dsl_protocols::onewire::{EndReason, OneWireEvent};

/// A transaction number, serialized transparently as an unsigned integer.
pub type TxId = u64;

/// Helpers on the canonical generated network event.
pub trait OneWireEventExt {
    /// The transaction the event belongs to.
    fn tx(&self) -> TxId;
}
impl OneWireEventExt for OneWireEvent {
    fn tx(&self) -> TxId {
        match self {
            OneWireEvent::Begin { tx, .. }
            | OneWireEvent::RomCommand { tx, .. }
            | OneWireEvent::Rom { tx, .. }
            | OneWireEvent::Search { tx, .. }
            | OneWireEvent::Byte { tx, .. }
            | OneWireEvent::Bits { tx, .. }
            | OneWireEvent::End { tx, .. } => *tx,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn json<T: Serialize>(value: &T) -> String {
        serde_json::to_string(value).expect("serializes")
    }

    #[test]
    fn link_events_serialize_as_the_contract_spells_them() {
        assert_eq!(
            json(&LinkEvent::Reset { presence: true }),
            r#"{"kind":"reset","presence":true}"#
        );
        assert_eq!(
            json(&LinkEvent::Bit { value: false }),
            r#"{"kind":"bit","value":false}"#
        );
        assert_eq!(
            json(&LinkEvent::BadSlot { low_ticks: 1500 }),
            r#"{"kind":"bad-slot","lowTicks":1500}"#
        );
    }

    #[test]
    fn network_events_serialize_as_the_contract_spells_them() {
        let tx = 2;
        assert_eq!(
            json(&OneWireEvent::Begin {
                tx,
                presence: false
            }),
            r#"{"kind":"begin","tx":2,"presence":false}"#
        );
        assert_eq!(
            json(&OneWireEvent::RomCommand { tx, code: 0x55 }),
            r#"{"kind":"rom-command","tx":2,"code":85}"#
        );
        assert_eq!(
            json(&OneWireEvent::Rom {
                tx,
                id: 0x28,
                family: 0x28,
                crc_ok: false
            }),
            r#"{"kind":"rom","tx":2,"id":"0x28","family":40,"crcOk":false}"#
        );
        assert_eq!(
            json(&OneWireEvent::Search {
                tx,
                id: 1,
                crc_ok: true,
                conflicts: 2,
                unanswered: 0
            }),
            r#"{"kind":"search","tx":2,"id":"0x1","crcOk":true,"conflicts":2,"unanswered":0}"#
        );
        assert_eq!(
            json(&OneWireEvent::Byte { tx, value: 0xbe }),
            r#"{"kind":"byte","tx":2,"value":190}"#
        );
        assert_eq!(
            json(&OneWireEvent::Bits {
                tx,
                value: 1,
                count: 1
            }),
            r#"{"kind":"bits","tx":2,"value":1,"count":1}"#
        );
        assert_eq!(
            json(&OneWireEvent::End {
                tx,
                reason: EndReason::BadSlot
            }),
            r#"{"kind":"end","tx":2,"reason":"bad-slot"}"#
        );
        assert_eq!(
            OneWireEvent::Bits {
                tx,
                value: 0,
                count: 3
            }
            .tx(),
            tx
        );
    }
}
