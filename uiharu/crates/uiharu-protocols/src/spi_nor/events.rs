// SPDX-License-Identifier: Apache-2.0

//! The `nor.events` contract: what the SPI NOR decoder emits.

use serde::{Deserialize, Serialize};
use uiharu_core::{EventSchema, SchemaId};

use crate::spi::TxId;

/// The schema of SPI NOR event streams. Version 2 carries the single-line command set: address
/// widths, dummy bytes, data in both directions, status registers and identification.
pub const NOR_EVENTS: SchemaId = SchemaId::new("nor.events", 2);

/// The most bytes one `data` or `id` event carries.
pub const DATA_BATCH: usize = 32;

/// An operation on the flash, numbered from 1 by the decoder.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct OpId(pub u64);

/// The commands the decoder knows, named after what they do; the datasheet mnemonic and the
/// opcode are given with each.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Command {
    /// Write Enable (WREN), `0x06`.
    WriteEnable,
    /// Write Disable (WRDI), `0x04`.
    WriteDisable,
    /// Write Enable for Volatile Status Register, `0x50`.
    VolatileSrWriteEnable,
    /// Read Status Register 1 (RDSR), `0x05`.
    ReadStatus,
    /// Read Status Register 2, `0x35`.
    #[serde(rename = "read-status-2")]
    ReadStatus2,
    /// Read Status Register 3, `0x15`.
    #[serde(rename = "read-status-3")]
    ReadStatus3,
    /// Write Status Register (WRSR), `0x01`: register 1, and register 2 when a second byte
    /// follows.
    WriteStatus,
    /// Write Status Register 2, `0x31`.
    #[serde(rename = "write-status-2")]
    WriteStatus2,
    /// Write Status Register 3, `0x11`.
    #[serde(rename = "write-status-3")]
    WriteStatus3,
    /// Read Data (READ), `0x03`.
    Read,
    /// Fast Read (FAST_READ), `0x0B`: one dummy byte before the data.
    FastRead,
    /// Read Data with a four-byte address (READ4B), `0x13`.
    #[serde(rename = "read-4b")]
    Read4b,
    /// Fast Read with a four-byte address (FAST_READ4B), `0x0C`.
    #[serde(rename = "fast-read-4b")]
    FastRead4b,
    /// Page Program (PP), `0x02`.
    PageProgram,
    /// Page Program with a four-byte address (PP4B), `0x12`.
    #[serde(rename = "page-program-4b")]
    PageProgram4b,
    /// Sector Erase, 4 KiB (SE), `0x20`.
    SectorErase,
    /// Sector Erase with a four-byte address (SE4B), `0x21`.
    #[serde(rename = "sector-erase-4b")]
    SectorErase4b,
    /// Block Erase, 32 KiB (BE32K), `0x52`.
    #[serde(rename = "block-erase-32k")]
    BlockErase32k,
    /// Block Erase, 64 KiB (BE64K), `0xD8`.
    #[serde(rename = "block-erase-64k")]
    BlockErase64k,
    /// Block Erase, 64 KiB, with a four-byte address (BE64K4B), `0xDC`.
    #[serde(rename = "block-erase-64k-4b")]
    BlockErase64k4b,
    /// Chip Erase (CE), `0xC7` or `0x60`.
    ChipErase,
    /// Deep Power-Down (DP), `0xB9`.
    DeepPowerDown,
    /// Release from Deep Power-Down, and Read Electronic Signature (RES) when bytes follow,
    /// `0xAB`.
    ReleasePowerDown,
    /// Read Electronic Manufacturer and Device ID (REMS), `0x90`.
    ReadManufacturerDeviceId,
    /// Read JEDEC ID (RDID), `0x9F`.
    ReadJedecId,
    /// Read Serial Flash Discoverable Parameters (RDSFDP), `0x5A`.
    ReadSfdp,
    /// Read Unique ID (RUID), `0x4B`.
    ReadUniqueId,
    /// Enter Four-Byte Address Mode (EN4B), `0xB7`.
    #[serde(rename = "enter-4b-mode")]
    Enter4bMode,
    /// Exit Four-Byte Address Mode (EX4B), `0xE9`.
    #[serde(rename = "exit-4b-mode")]
    Exit4bMode,
    /// Reset Enable (RSTEN), `0x66`.
    ResetEnable,
    /// Reset Device (RST), `0x99`.
    Reset,
    /// Program/Erase Suspend, `0x75`.
    ProgramEraseSuspend,
    /// Program/Erase Resume, `0x7A`.
    ProgramEraseResume,
    /// Read Security Register, `0x48`.
    ReadSecurityRegister,
    /// Program Security Register, `0x42`.
    ProgramSecurityRegister,
    /// Erase Security Register, `0x44`.
    EraseSecurityRegister,
    /// Individual Block Lock, `0x36`.
    BlockLock,
    /// Individual Block Unlock, `0x39`.
    BlockUnlock,
    /// Global Block Lock, `0x7E`.
    GlobalBlockLock,
    /// Global Block Unlock, `0x98`.
    GlobalBlockUnlock,
}

/// Which way data moves.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Direction {
    /// From the host into the flash, on MOSI: the bytes a program writes.
    In,
    /// From the flash out to the host, on MISO: the bytes a read returns.
    Out,
}

/// Which identification an `id` event carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum IdScheme {
    /// The JEDEC ID of RDID: manufacturer, memory type and capacity, and whatever bytes the
    /// flash sends after them.
    Jedec,
    /// The two bytes of REMS, in the order the flash sent them: manufacturer and device ID,
    /// swapped when the address was odd.
    ManufacturerDevice,
    /// The unique ID of RUID: eight bytes.
    Unique,
    /// The one-byte electronic signature of RES.
    ElectronicSignature,
}

/// How an operation ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum NorEnd {
    /// The command ran to its end: chip select was released after it was whole.
    Complete,
    /// Chip select was released before the command was whole.
    Incomplete,
    /// The samples ended or were lost while the command was under way.
    Aborted,
}

/// What travels on an SPI NOR event stream.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum NorEvent {
    /// A command was recognised.
    Begin {
        /// The operation that begins.
        op: OpId,
        /// The SPI transaction it rides on.
        tx: TxId,
        /// The command.
        command: Command,
    },
    /// The command's address.
    Address {
        /// The operation.
        op: OpId,
        /// The address, most significant byte first on the bus.
        address: u32,
        /// How many bytes it took: 3 or 4.
        bytes: u8,
    },
    /// Dummy bytes were clocked; their values mean nothing.
    Dummy {
        /// The operation.
        op: OpId,
        /// How many.
        bytes: u8,
    },
    /// Bytes read from or written to the flash, in order.
    Data {
        /// The operation.
        op: OpId,
        /// Which way they moved.
        direction: Direction,
        /// The position of the first byte, counted from the operation's first data byte.
        offset: u32,
        /// At most [`DATA_BATCH`] bytes.
        bytes: Vec<u8>,
    },
    /// A status register value was read or written.
    Status {
        /// The operation.
        op: OpId,
        /// The register: 1, 2 or 3.
        register: u8,
        /// The value on the bus.
        value: u8,
        /// Whether the host wrote it (on MOSI) rather than read it (on MISO).
        written: bool,
    },
    /// Identification bytes the flash sent.
    Id {
        /// The operation.
        op: OpId,
        /// Which identification.
        scheme: IdScheme,
        /// The bytes as read, at most [`DATA_BATCH`]; a longer JEDEC ID read continues in the
        /// next `id` event.
        bytes: Vec<u8>,
    },
    /// The operation ended.
    End {
        /// The operation.
        op: OpId,
        /// How it ended.
        reason: NorEnd,
    },
    /// A transaction started with an opcode the decoder does not know; its words were
    /// skipped.
    UnknownCommand {
        /// The SPI transaction.
        tx: TxId,
        /// Its first word on MOSI.
        opcode: u8,
    },
}

impl EventSchema for NorEvent {
    const SCHEMA: SchemaId = NOR_EVENTS;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn json(event: &NorEvent) -> String {
        serde_json::to_string(event).expect("serializes")
    }

    #[test]
    fn serializes_as_the_contract_spells_it() {
        let op = OpId(1);
        assert_eq!(
            json(&NorEvent::Begin {
                op,
                tx: TxId(4),
                command: Command::FastRead4b,
            }),
            r#"{"kind":"begin","op":1,"tx":4,"command":"fast-read-4b"}"#
        );
        assert_eq!(
            json(&NorEvent::Address {
                op,
                address: 0x0123_4567,
                bytes: 4,
            }),
            r#"{"kind":"address","op":1,"address":19088743,"bytes":4}"#
        );
        assert_eq!(
            json(&NorEvent::Dummy { op, bytes: 1 }),
            r#"{"kind":"dummy","op":1,"bytes":1}"#
        );
        assert_eq!(
            json(&NorEvent::Data {
                op,
                direction: Direction::In,
                offset: 32,
                bytes: vec![1, 2],
            }),
            r#"{"kind":"data","op":1,"direction":"in","offset":32,"bytes":[1,2]}"#
        );
        assert_eq!(
            json(&NorEvent::Status {
                op,
                register: 2,
                value: 0x02,
                written: true,
            }),
            r#"{"kind":"status","op":1,"register":2,"value":2,"written":true}"#
        );
        assert_eq!(
            json(&NorEvent::Id {
                op,
                scheme: IdScheme::ManufacturerDevice,
                bytes: vec![0xef, 0x16],
            }),
            r#"{"kind":"id","op":1,"scheme":"manufacturer-device","bytes":[239,22]}"#
        );
        assert_eq!(
            json(&NorEvent::End {
                op,
                reason: NorEnd::Incomplete,
            }),
            r#"{"kind":"end","op":1,"reason":"incomplete"}"#
        );
        assert_eq!(
            json(&NorEvent::UnknownCommand {
                tx: TxId(4),
                opcode: 0xeb,
            }),
            r#"{"kind":"unknown-command","tx":4,"opcode":235}"#
        );
    }

    #[test]
    fn names_with_numbers_keep_their_hyphens() {
        let name = |command: Command| serde_json::to_value(command).expect("serializes");
        assert_eq!(name(Command::ReadStatus2), "read-status-2");
        assert_eq!(name(Command::WriteStatus3), "write-status-3");
        assert_eq!(name(Command::Read4b), "read-4b");
        assert_eq!(name(Command::BlockErase32k), "block-erase-32k");
        assert_eq!(name(Command::BlockErase64k4b), "block-erase-64k-4b");
        assert_eq!(name(Command::Enter4bMode), "enter-4b-mode");
        assert_eq!(
            name(Command::VolatileSrWriteEnable),
            "volatile-sr-write-enable"
        );
        assert_eq!(
            name(Command::ReadManufacturerDeviceId),
            "read-manufacturer-device-id"
        );
    }
}
