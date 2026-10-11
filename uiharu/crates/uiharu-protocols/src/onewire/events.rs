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
#[serde(tag = "kind", rename_all = "kebab-case")]
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
    /// A low pulse that fits no window: longer than a slot and shorter than a reset, a pulse
    /// in a presence window that is no presence pulse, or a slot whose sampling point fell
    /// into the next pulse.
    BadSlot {
        /// How long the line was low, in ticks.
        low_ticks: u64,
    },
}

impl EventSchema for LinkEvent {
    const SCHEMA: SchemaId = ONEWIRE_LINK;
}

/// A transaction: from one reset to the next. Numbered from 1 by the network decoder.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct TxId(pub u64);

/// Why a transaction ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EndReason {
    /// The next reset: the normal end of a transaction.
    Reset,
    /// A bad slot: the bits after it can no longer be told apart.
    BadSlot,
    /// The samples ended or were lost while the transaction was under way.
    Aborted,
}

/// What travels on a 1-Wire network-layer stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum OneWireEvent {
    /// A reset began a transaction.
    Begin {
        /// The transaction that begins.
        tx: TxId,
        /// Whether a slave answered the reset with a presence pulse.
        presence: bool,
    },
    /// The first byte of the transaction: the ROM command.
    RomCommand {
        /// The transaction.
        tx: TxId,
        /// The command's code, such as `0x55` for Match ROM.
        code: u8,
    },
    /// The 64-bit ROM code that Read ROM read or Match ROM sent.
    Rom {
        /// The transaction.
        tx: TxId,
        /// The ROM code, its first bit on the line as bit 0: the family code is the low byte,
        /// the CRC the high byte.
        id: u64,
        /// The family code, the first byte of the ROM code.
        family: u8,
        /// Whether the last byte is the CRC-8 of the first seven.
        crc_ok: bool,
    },
    /// A Search ROM or Alarm Search ran its 64 triplets.
    Search {
        /// The transaction.
        tx: TxId,
        /// The direction bits the master wrote, the first as bit 0: the ROM code it selected.
        id: u64,
        /// Whether the last byte of `id` is the CRC-8 of the first seven.
        crc_ok: bool,
        /// Triplets where devices disagreed (bit and complement both 0) and the master chose.
        conflicts: u8,
        /// Triplets where no device answered (bit and complement both 1).
        unanswered: u8,
    },
    /// A byte after the ROM level, least significant bit first on the line.
    Byte {
        /// The transaction.
        tx: TxId,
        /// The byte.
        value: u8,
    },
    /// The bits after the last whole byte when the transaction ended: fewer than eight.
    Bits {
        /// The transaction.
        tx: TxId,
        /// The bits, the first as bit 0.
        value: u8,
        /// How many there are, 1 to 7.
        count: u8,
    },
    /// The transaction ended.
    End {
        /// The transaction.
        tx: TxId,
        /// Why it ended.
        reason: EndReason,
    },
}

impl OneWireEvent {
    /// The transaction the event belongs to.
    pub fn tx(&self) -> TxId {
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

impl EventSchema for OneWireEvent {
    const SCHEMA: SchemaId = ONEWIRE_EVENTS;
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
            r#"{"kind":"bad-slot","low_ticks":1500}"#
        );
    }

    #[test]
    fn network_events_serialize_as_the_contract_spells_them() {
        let tx = TxId(2);
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
            r#"{"kind":"rom","tx":2,"id":40,"family":40,"crc_ok":false}"#
        );
        assert_eq!(
            json(&OneWireEvent::Search {
                tx,
                id: 1,
                crc_ok: true,
                conflicts: 2,
                unanswered: 0
            }),
            r#"{"kind":"search","tx":2,"id":1,"crc_ok":true,"conflicts":2,"unanswered":0}"#
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
