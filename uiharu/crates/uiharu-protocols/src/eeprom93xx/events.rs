// SPDX-License-Identifier: Apache-2.0

//! The `eeprom93xx.events` contract: the instructions a 93Cxx Microwire EEPROM was sent and the
//! words they carried, as they streamed.

use serde::{Deserialize, Serialize};
use uiharu_core::{EventSchema, SchemaId};

use crate::microwire::TxId;

/// The schema of 93Cxx EEPROM event streams.
pub const EEPROM93XX_EVENTS: SchemaId = SchemaId::new("eeprom93xx.events", 1);

/// The most words one `data` event carries.
pub const DATA_BATCH: usize = 32;

/// An operation on the EEPROM — one instruction — numbered from 1 by the decoder.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct OpId(pub u64);

/// The instructions, by their datasheet mnemonics.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Command {
    /// READ, opcode `10`: the words from an address on, on DO.
    Read,
    /// WRITE, opcode `01`: one word written to an address.
    Write,
    /// ERASE, opcode `11`: the word at an address set to all ones.
    Erase,
    /// EWEN, opcode `00` with the address's top bits `11`: erasing and writing enabled.
    Ewen,
    /// EWDS, opcode `00` with the address's top bits `00`: erasing and writing disabled.
    Ewds,
    /// ERAL, opcode `00` with the address's top bits `10`: the whole array erased.
    Eral,
    /// WRAL, opcode `00` with the address's top bits `01`: one word written everywhere.
    Wral,
}

impl Command {
    /// Whether the instruction's address bits are an address; for the others they select the
    /// instruction or mean nothing.
    pub const fn addressed(self) -> bool {
        matches!(self, Command::Read | Command::Write | Command::Erase)
    }

    /// Whether the instruction starts a programming cycle, whose busy and ready state the
    /// device then shows on DO.
    pub const fn programs(self) -> bool {
        matches!(
            self,
            Command::Write | Command::Erase | Command::Eral | Command::Wral
        )
    }
}

/// Which way words move.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Direction {
    /// From the controller into the EEPROM, on DI: the word a write carries.
    In,
    /// From the EEPROM out to the controller, on DO: the words a read returns.
    Out,
}

/// How an operation ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EndReason {
    /// The instruction ran to its end: chip select was released after every bit it needs —
    /// for a read, after one or more whole words.
    Complete,
    /// Chip select was released before the instruction was whole, which the device does not
    /// execute; a read released inside a word, or before its first.
    Incomplete,
    /// The samples ended or were lost while the instruction was under way.
    Aborted,
}

/// What travels on a 93Cxx EEPROM event stream.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum Eeprom93xxEvent {
    /// An instruction: its start bit, opcode and address.
    Instruction {
        /// The operation it begins.
        op: OpId,
        /// The Microwire transaction it rides on.
        tx: TxId,
        /// The instruction.
        command: Command,
        /// The address, for READ, WRITE and ERASE; `None` for the others, and for an
        /// instruction whose address chip select cut short.
        address: Option<u16>,
    },
    /// Words written or read, in order.
    Data {
        /// The operation.
        op: OpId,
        /// Which way they went.
        direction: Direction,
        /// The position of the first word, counted from the operation's first.
        offset: u32,
        /// At most [`DATA_BATCH`] words, most significant bit first on the bus.
        values: Vec<u16>,
    },
    /// DO showed whether the device is busy with the programming cycle of `op` or ready again.
    Status {
        /// The programming operation the device is busy with or has finished.
        op: OpId,
        /// The Microwire transaction in which DO showed it.
        tx: TxId,
        /// Whether DO was high: the device is ready.
        ready: bool,
    },
    /// The operation ended.
    End {
        /// The operation.
        op: OpId,
        /// How it ended.
        reason: EndReason,
    },
}

impl EventSchema for Eeprom93xxEvent {
    const SCHEMA: SchemaId = EEPROM93XX_EVENTS;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn json(event: &Eeprom93xxEvent) -> String {
        serde_json::to_string(event).expect("serializes")
    }

    #[test]
    fn serializes_as_the_contract_spells_it() {
        let op = OpId(2);
        assert_eq!(
            json(&Eeprom93xxEvent::Instruction {
                op,
                tx: TxId(5),
                command: Command::Read,
                address: Some(0x2a),
            }),
            r#"{"kind":"instruction","op":2,"tx":5,"command":"read","address":42}"#
        );
        assert_eq!(
            json(&Eeprom93xxEvent::Instruction {
                op,
                tx: TxId(5),
                command: Command::Ewen,
                address: None,
            }),
            r#"{"kind":"instruction","op":2,"tx":5,"command":"ewen","address":null}"#
        );
        assert_eq!(
            json(&Eeprom93xxEvent::Data {
                op,
                direction: Direction::Out,
                offset: 3,
                values: vec![0xbeef, 1],
            }),
            r#"{"kind":"data","op":2,"direction":"out","offset":3,"values":[48879,1]}"#
        );
        assert_eq!(
            json(&Eeprom93xxEvent::Status {
                op,
                tx: TxId(6),
                ready: true
            }),
            r#"{"kind":"status","op":2,"tx":6,"ready":true}"#
        );
        assert_eq!(
            json(&Eeprom93xxEvent::End {
                op,
                reason: EndReason::Incomplete
            }),
            r#"{"kind":"end","op":2,"reason":"incomplete"}"#
        );
        for (command, name) in [
            (Command::Write, "write"),
            (Command::Erase, "erase"),
            (Command::Ewds, "ewds"),
            (Command::Eral, "eral"),
            (Command::Wral, "wral"),
        ] {
            assert_eq!(
                serde_json::to_string(&command).expect("serializes"),
                format!(r#""{name}""#)
            );
        }
    }

    #[test]
    fn commands_know_their_address_and_programming() {
        assert!(Command::Read.addressed() && !Command::Read.programs());
        assert!(Command::Erase.addressed() && Command::Erase.programs());
        assert!(!Command::Wral.addressed() && Command::Wral.programs());
        assert!(!Command::Ewen.addressed() && !Command::Ewen.programs());
    }
}
