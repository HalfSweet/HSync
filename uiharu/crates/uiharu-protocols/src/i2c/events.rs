// SPDX-License-Identifier: Apache-2.0

//! The `i2c.events` contract: what an I²C decoder emits and what decoders built on I²C read.
//!
//! A transaction runs from a START to a STOP. Within a transaction whose START was seen, the
//! first event and the first event after every `restart` is an `address`, and addresses come
//! nowhere else; the other bytes are `data`. Every byte carries the acknowledge bit that
//! followed it. A transaction found under way (`partial`) carries no bytes.

use serde::{Deserialize, Serialize};
use uiharu_core::{EventSchema, SchemaId};

/// The schema of I²C event streams.
pub const I2C_EVENTS: SchemaId = SchemaId::new("i2c.events", 1);

/// A transaction: from a START to a STOP. Numbered from 1 by the decoder.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct TxId(pub u64);

/// Why a transaction ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EndReason {
    /// A STOP condition: the transaction is whole.
    Stop,
    /// A START condition ended a partial transaction; the transaction it begins follows at the
    /// same tick. Only partial transactions end this way: in any other, a repeated START is a
    /// `restart` event and the transaction goes on.
    Restart,
    /// The samples ended or were lost while the transaction was under way.
    Aborted,
}

/// What travels on an I²C event stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum I2cEvent {
    /// A START condition, or a transaction found under way.
    Begin {
        /// The transaction that begins.
        tx: TxId,
        /// The bus was already busy when the samples began (at the start of the capture or
        /// after a gap), or was clocked with no START seen: the transaction's beginning was
        /// missed and its bytes cannot be aligned, so it carries none.
        partial: bool,
    },
    /// The address after a START or repeated START, and the controller's direction.
    Address {
        /// The transaction.
        tx: TxId,
        /// The 7-bit or 10-bit address.
        address: u16,
        /// Whether this is a 10-bit address: `1111 0xx` with its second byte, or a read that
        /// refers back to one.
        ten_bit: bool,
        /// The R/W̄ bit: whether the controller reads from the target.
        read: bool,
        /// Whether the address was acknowledged; for a 10-bit write address, both its bytes.
        ack: bool,
    },
    /// A data byte.
    Data {
        /// The transaction.
        tx: TxId,
        /// The byte, most significant bit first on the bus.
        byte: u8,
        /// Whether the receiver acknowledged it: SDA low on the ninth clock.
        ack: bool,
    },
    /// A repeated START: the transaction goes on with another address.
    Restart {
        /// The transaction.
        tx: TxId,
        /// Bits of an unfinished byte that the repeated START cut short; they make no byte.
        dropped_bits: u8,
    },
    /// The transaction ended.
    End {
        /// The transaction.
        tx: TxId,
        /// Why it ended.
        reason: EndReason,
        /// Bits of an unfinished byte that the end cut short; they make no byte.
        dropped_bits: u8,
    },
}

impl I2cEvent {
    /// The transaction the event belongs to.
    pub fn tx(&self) -> TxId {
        match self {
            I2cEvent::Begin { tx, .. }
            | I2cEvent::Address { tx, .. }
            | I2cEvent::Data { tx, .. }
            | I2cEvent::Restart { tx, .. }
            | I2cEvent::End { tx, .. } => *tx,
        }
    }
}

impl EventSchema for I2cEvent {
    const SCHEMA: SchemaId = I2C_EVENTS;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn json(event: I2cEvent) -> String {
        serde_json::to_string(&event).expect("serializes")
    }

    #[test]
    fn serializes_as_the_contract_spells_it() {
        assert_eq!(
            json(I2cEvent::Begin {
                tx: TxId(3),
                partial: true,
            }),
            r#"{"kind":"begin","tx":3,"partial":true}"#
        );
        assert_eq!(
            json(I2cEvent::Address {
                tx: TxId(3),
                address: 0x2a5,
                ten_bit: true,
                read: false,
                ack: true,
            }),
            r#"{"kind":"address","tx":3,"address":677,"ten_bit":true,"read":false,"ack":true}"#
        );
        assert_eq!(
            json(I2cEvent::Data {
                tx: TxId(3),
                byte: 0x12,
                ack: false,
            }),
            r#"{"kind":"data","tx":3,"byte":18,"ack":false}"#
        );
        assert_eq!(
            json(I2cEvent::Restart {
                tx: TxId(3),
                dropped_bits: 0,
            }),
            r#"{"kind":"restart","tx":3,"dropped_bits":0}"#
        );
        assert_eq!(
            json(I2cEvent::End {
                tx: TxId(3),
                reason: EndReason::Stop,
                dropped_bits: 2,
            }),
            r#"{"kind":"end","tx":3,"reason":"stop","dropped_bits":2}"#
        );
        assert_eq!(
            json(I2cEvent::End {
                tx: TxId(4),
                reason: EndReason::Restart,
                dropped_bits: 0,
            }),
            r#"{"kind":"end","tx":4,"reason":"restart","dropped_bits":0}"#
        );
        assert_eq!(
            json(I2cEvent::End {
                tx: TxId(5),
                reason: EndReason::Aborted,
                dropped_bits: 0,
            }),
            r#"{"kind":"end","tx":5,"reason":"aborted","dropped_bits":0}"#
        );
    }

    #[test]
    fn every_event_names_its_transaction() {
        let tx = TxId(9);
        for event in [
            I2cEvent::Begin { tx, partial: false },
            I2cEvent::Address {
                tx,
                address: 0x50,
                ten_bit: false,
                read: true,
                ack: true,
            },
            I2cEvent::Data {
                tx,
                byte: 0,
                ack: true,
            },
            I2cEvent::Restart {
                tx,
                dropped_bits: 1,
            },
            I2cEvent::End {
                tx,
                reason: EndReason::Stop,
                dropped_bits: 0,
            },
        ] {
            assert_eq!(event.tx(), tx);
        }
    }
}
