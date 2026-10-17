// SPDX-License-Identifier: Apache-2.0

//! The `eeprom24xx.events` contract: the commands a 24xx-series EEPROM was sent, as they
//! streamed over I²C.

use serde::{Deserialize, Serialize};
use uiharu_core::{EventSchema, SchemaId};

use crate::i2c::TxId;

/// The schema of 24xx EEPROM event streams.
pub const EEPROM24XX_EVENTS: SchemaId = SchemaId::new("eeprom24xx.events", 1);

/// The most bytes one data event carries.
pub const DATA_BATCH: usize = 32;

/// An operation on the EEPROM, numbered from 1 by the decoder.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct OpId(pub u64);

/// The commands the decoder tells apart.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Command {
    /// A write of the word address alone, which sets where the next current-address read
    /// starts.
    SetAddress,
    /// A write of one data byte.
    ByteWrite,
    /// A write of several data bytes.
    PageWrite,
    /// A write cut off by a gap or the end of the samples before what followed its control
    /// byte could tell which of the three it was.
    Write,
    /// A read from where the device's address counter stands.
    CurrentAddressRead,
    /// A set-address, a repeated START and a read from the address it set.
    RandomRead,
}

/// How an operation ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EepromEnd {
    /// The command ran to its end.
    Complete,
    /// The controller ended the command before it was whole: a read of no byte, or a write of
    /// data ended by a repeated START, which the device does not program.
    Incomplete,
    /// The samples ended or were lost while the command was under way.
    Aborted,
}

/// What travels on a 24xx EEPROM event stream.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum EepromEvent {
    /// A command was told.
    Begin {
        /// The operation that begins.
        op: OpId,
        /// The I²C transaction it rides on.
        tx: TxId,
        /// The device's 7-bit address.
        device: u8,
        /// The command.
        command: Command,
    },
    /// The word address the command gave.
    Address {
        /// The operation.
        op: OpId,
        /// The word address, as sent.
        word_address: u16,
    },
    /// Bytes written or read, in order.
    Data {
        /// The operation.
        op: OpId,
        /// The position of the first byte, counted from the operation's first data byte.
        offset: u32,
        /// At most [`DATA_BATCH`] bytes.
        bytes: Vec<u8>,
    },
    /// The operation ended.
    End {
        /// The operation.
        op: OpId,
        /// How it ended.
        reason: EepromEnd,
        /// The data bytes it carried.
        length: u32,
    },
    /// A control byte with nothing after it: the controller polling for the end of a write
    /// cycle.
    AckPoll {
        /// The I²C transaction.
        tx: TxId,
        /// The device's 7-bit address.
        device: u8,
        /// Whether the device acknowledged: it is ready. A device busy with a write cycle, or
        /// absent, does not.
        acked: bool,
    },
    /// Traffic to the device that fits no command: a write whose word address was cut short.
    Unknown {
        /// The I²C transaction.
        tx: TxId,
        /// The device's 7-bit address.
        device: u8,
    },
}

impl EventSchema for EepromEvent {
    const SCHEMA: SchemaId = EEPROM24XX_EVENTS;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn json(event: &EepromEvent) -> String {
        serde_json::to_string(event).expect("serializes")
    }

    #[test]
    fn serializes_as_the_contract_spells_it() {
        assert_eq!(
            json(&EepromEvent::Begin {
                op: OpId(1),
                tx: 2,
                device: 0x50,
                command: Command::CurrentAddressRead,
            }),
            r#"{"kind":"begin","op":1,"tx":2,"device":80,"command":"current-address-read"}"#
        );
        assert_eq!(
            json(&EepromEvent::Address {
                op: OpId(1),
                word_address: 0x1234,
            }),
            r#"{"kind":"address","op":1,"wordAddress":4660}"#
        );
        assert_eq!(
            json(&EepromEvent::Data {
                op: OpId(1),
                offset: 32,
                bytes: vec![1, 2],
            }),
            r#"{"kind":"data","op":1,"offset":32,"bytes":[1,2]}"#
        );
        assert_eq!(
            json(&EepromEvent::End {
                op: OpId(1),
                reason: EepromEnd::Incomplete,
                length: 0,
            }),
            r#"{"kind":"end","op":1,"reason":"incomplete","length":0}"#
        );
        assert_eq!(
            json(&EepromEvent::AckPoll {
                tx: 3,
                device: 0x57,
                acked: false,
            }),
            r#"{"kind":"ack-poll","tx":3,"device":87,"acked":false}"#
        );
        assert_eq!(
            json(&EepromEvent::Unknown {
                tx: 4,
                device: 0x51,
            }),
            r#"{"kind":"unknown","tx":4,"device":81}"#
        );
    }

    #[test]
    fn commands_are_named_in_kebab_case() {
        let names: Vec<String> = [
            Command::SetAddress,
            Command::ByteWrite,
            Command::PageWrite,
            Command::Write,
            Command::CurrentAddressRead,
            Command::RandomRead,
        ]
        .iter()
        .map(|command| serde_json::to_string(command).expect("serializes"))
        .collect();
        assert_eq!(
            names,
            [
                r#""set-address""#,
                r#""byte-write""#,
                r#""page-write""#,
                r#""write""#,
                r#""current-address-read""#,
                r#""random-read""#,
            ]
        );
    }
}
