// SPDX-License-Identifier: Apache-2.0

//! The `avrisp.events` contract: what the AVR serial programming decoder emits.

use serde::{Deserialize, Serialize};
use uiharu_core::{EventSchema, SchemaId};

use crate::spi::TxId;

/// The schema of AVR serial programming event streams.
pub const AVR_ISP_EVENTS: SchemaId = SchemaId::new("avrisp.events", 1);

/// The bytes of an instruction.
pub const INSTRUCTION_BYTES: usize = 4;

/// An instruction the decoder recognised, numbered from 1.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct OpId(pub u64);

/// The instructions of the serial programming instruction set, named as the datasheets name
/// them; the first two bytes are given with each, `aa` standing for an address byte.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Command {
    /// Programming Enable, `AC 53`: the device echoes `53` while the third byte is sent when
    /// it is in sync.
    ProgrammingEnable,
    /// Chip Erase, `AC 80`: program memory and EEPROM.
    ChipErase,
    /// Poll RDY/BSY, `F0 00`: bit 0 of the answer is 1 while the device is busy.
    PollRdyBsy,
    /// Load Extended Address byte, `4D 00`: the third byte is the extended address.
    LoadExtendedAddress,
    /// Load Program Memory Page, high byte, `48 00`.
    LoadProgramMemoryPageHigh,
    /// Load Program Memory Page, low byte, `40 00`.
    LoadProgramMemoryPageLow,
    /// Load EEPROM Memory Page, `C1 00`.
    LoadEepromMemoryPage,
    /// Read Program Memory, high byte, `28 aa`.
    ReadProgramMemoryHigh,
    /// Read Program Memory, low byte, `20 aa`.
    ReadProgramMemoryLow,
    /// Read EEPROM Memory, `A0 aa`.
    ReadEepromMemory,
    /// Read Lock bits, `58 00`.
    ReadLockBits,
    /// Read Signature Byte, `30 00`: the third byte selects the signature byte, 0 to 2.
    ReadSignatureByte,
    /// Read Fuse bits, `50 00`.
    ReadFuseBits,
    /// Read Fuse High bits, `58 08`.
    ReadFuseHighBits,
    /// Read Extended Fuse Bits, `50 08`.
    ReadExtendedFuseBits,
    /// Read Calibration Byte, `38 00`.
    ReadCalibrationByte,
    /// Write Program Memory Page, `4C aa`.
    WriteProgramMemoryPage,
    /// Write EEPROM Memory, `C0 aa`: one byte.
    WriteEepromMemory,
    /// Write EEPROM Memory Page, `C2 aa`.
    WriteEepromMemoryPage,
    /// Write Lock bits, `AC E0`.
    WriteLockBits,
    /// Write Fuse bits, `AC A0`.
    WriteFuseBits,
    /// Write Fuse High bits, `AC A8`.
    WriteFuseHighBits,
    /// Write Extended Fuse Bits, `AC A4`.
    WriteExtendedFuseBits,
}

/// Why an instruction was not whole.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ErrorReason {
    /// Chip select (the target's RESET) was released before the fourth byte.
    Incomplete,
    /// The samples ended or were lost before the fourth byte.
    Aborted,
}

/// What travels on an AVR serial programming event stream.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum IspEvent {
    /// A whole instruction the decoder recognised.
    Instruction {
        /// The instruction's number.
        op: OpId,
        /// The SPI transaction it rides on: 0 on a bus without chip select.
        tx: TxId,
        /// Which instruction.
        command: Command,
        /// The address bytes two and three carry, the second most significant, for the
        /// instructions that take one; for Load Extended Address, the extended address byte.
        address: Option<u16>,
        /// The byte the programmer sends in the fourth byte, for the instructions that write
        /// or load one.
        data_in: Option<u8>,
        /// The byte the device answers on MISO during the fourth byte, for the reads and
        /// Poll RDY/BSY.
        data_out: Option<u8>,
        /// For Programming Enable, whether the device echoed `53` during the third byte.
        in_sync: Option<bool>,
        /// The four bytes on MOSI.
        bytes: [u8; INSTRUCTION_BYTES],
    },
    /// Four bytes whose first, or first two, name no instruction of the set.
    Unknown {
        /// The SPI transaction they ride on.
        tx: TxId,
        /// The four bytes on MOSI.
        bytes: [u8; INSTRUCTION_BYTES],
    },
    /// An instruction cut short.
    Error {
        /// The SPI transaction it rode on.
        tx: TxId,
        /// How it was cut.
        reason: ErrorReason,
        /// The bytes on MOSI before the cut, one to three.
        bytes: Vec<u8>,
    },
}

impl EventSchema for IspEvent {
    const SCHEMA: SchemaId = AVR_ISP_EVENTS;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn json(event: &IspEvent) -> String {
        serde_json::to_string(event).expect("serializes")
    }

    #[test]
    fn serializes_as_the_contract_spells_it() {
        assert_eq!(
            json(&IspEvent::Instruction {
                op: OpId(2),
                tx: 1,
                command: Command::ReadProgramMemoryHigh,
                address: Some(0x0123),
                data_in: None,
                data_out: Some(0x94),
                in_sync: None,
                bytes: [0x28, 0x01, 0x23, 0x00],
            }),
            r#"{"kind":"instruction","op":2,"tx":1,"command":"read-program-memory-high","address":291,"dataIn":null,"dataOut":148,"inSync":null,"bytes":[40,1,35,0]}"#
        );
        assert_eq!(
            json(&IspEvent::Unknown {
                tx: 0,
                bytes: [0xff, 0, 0, 0],
            }),
            r#"{"kind":"unknown","tx":0,"bytes":[255,0,0,0]}"#
        );
        assert_eq!(
            json(&IspEvent::Error {
                tx: 3,
                reason: ErrorReason::Incomplete,
                bytes: vec![0xac, 0x53],
            }),
            r#"{"kind":"error","tx":3,"reason":"incomplete","bytes":[172,83]}"#
        );
    }

    #[test]
    fn commands_are_named_as_the_datasheets_name_them() {
        let name = |command: Command| serde_json::to_value(command).expect("serializes");
        assert_eq!(name(Command::ProgrammingEnable), "programming-enable");
        assert_eq!(name(Command::PollRdyBsy), "poll-rdy-bsy");
        assert_eq!(name(Command::LoadExtendedAddress), "load-extended-address");
        assert_eq!(
            name(Command::LoadProgramMemoryPageHigh),
            "load-program-memory-page-high"
        );
        assert_eq!(
            name(Command::LoadEepromMemoryPage),
            "load-eeprom-memory-page"
        );
        assert_eq!(name(Command::ReadFuseHighBits), "read-fuse-high-bits");
        assert_eq!(
            name(Command::ReadExtendedFuseBits),
            "read-extended-fuse-bits"
        );
        assert_eq!(
            name(Command::WriteEepromMemoryPage),
            "write-eeprom-memory-page"
        );
        assert_eq!(
            serde_json::to_value(ErrorReason::Aborted).expect("serializes"),
            "aborted"
        );
    }
}
