// SPDX-License-Identifier: Apache-2.0

//! The command set: which opcode is which command, and what each command shifts after its
//! opcode.

use super::events::{Command, IdScheme};

/// Where a command's address width comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Address {
    /// The command takes no address.
    None,
    /// Always three bytes, whatever the address mode.
    Three,
    /// Always four bytes: the dedicated four-byte commands.
    Four,
    /// Three or four bytes, as the flash's address mode is.
    Mode,
}

impl Address {
    /// The width in bytes when the address mode is `mode` bytes.
    pub(crate) const fn bytes(self, mode: u8) -> u8 {
        match self {
            Address::None => 0,
            Address::Three => 3,
            Address::Four => 4,
            Address::Mode => mode,
        }
    }
}

/// What a command transfers after its address and dummy bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Phase {
    /// Nothing: the command is whole.
    None,
    /// Data from the flash on MISO, as many bytes as the host reads.
    DataOut,
    /// Data from the host on MOSI, at least one byte.
    DataIn,
    /// Values of status register `n` on MISO, as many as the host reads.
    ReadStatus(u8),
    /// Values for status registers `first` to `last` on MOSI, at least one.
    WriteStatus {
        /// The register the first byte goes to.
        first: u8,
        /// The register of the last byte the command takes.
        last: u8,
    },
    /// Identification bytes on MISO: `len` of them, or as many as the host reads.
    Id {
        /// Which identification.
        scheme: IdScheme,
        /// Its length, if fixed.
        len: Option<u8>,
    },
    /// RES: nothing, or three dummy bytes and the one-byte electronic signature.
    Signature,
}

/// What a command shifts after its opcode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Layout {
    pub(crate) address: Address,
    pub(crate) dummy: u8,
    pub(crate) phase: Phase,
}

impl Layout {
    const fn bare(phase: Phase) -> Self {
        Self {
            address: Address::None,
            dummy: 0,
            phase,
        }
    }

    const fn addressed(address: Address, dummy: u8, phase: Phase) -> Self {
        Self {
            address,
            dummy,
            phase,
        }
    }
}

impl Command {
    /// Every command, in the order of the contract.
    pub const ALL: [Command; 40] = [
        Command::WriteEnable,
        Command::WriteDisable,
        Command::VolatileSrWriteEnable,
        Command::ReadStatus,
        Command::ReadStatus2,
        Command::ReadStatus3,
        Command::WriteStatus,
        Command::WriteStatus2,
        Command::WriteStatus3,
        Command::Read,
        Command::FastRead,
        Command::Read4b,
        Command::FastRead4b,
        Command::PageProgram,
        Command::PageProgram4b,
        Command::SectorErase,
        Command::SectorErase4b,
        Command::BlockErase32k,
        Command::BlockErase64k,
        Command::BlockErase64k4b,
        Command::ChipErase,
        Command::DeepPowerDown,
        Command::ReleasePowerDown,
        Command::ReadManufacturerDeviceId,
        Command::ReadJedecId,
        Command::ReadSfdp,
        Command::ReadUniqueId,
        Command::Enter4bMode,
        Command::Exit4bMode,
        Command::ResetEnable,
        Command::Reset,
        Command::ProgramEraseSuspend,
        Command::ProgramEraseResume,
        Command::ReadSecurityRegister,
        Command::ProgramSecurityRegister,
        Command::EraseSecurityRegister,
        Command::BlockLock,
        Command::BlockUnlock,
        Command::GlobalBlockLock,
        Command::GlobalBlockUnlock,
    ];

    /// The command's opcode; Chip Erase also answers to `0x60`.
    pub const fn opcode(self) -> u8 {
        match self {
            Command::WriteEnable => 0x06,
            Command::WriteDisable => 0x04,
            Command::VolatileSrWriteEnable => 0x50,
            Command::ReadStatus => 0x05,
            Command::ReadStatus2 => 0x35,
            Command::ReadStatus3 => 0x15,
            Command::WriteStatus => 0x01,
            Command::WriteStatus2 => 0x31,
            Command::WriteStatus3 => 0x11,
            Command::Read => 0x03,
            Command::FastRead => 0x0b,
            Command::Read4b => 0x13,
            Command::FastRead4b => 0x0c,
            Command::PageProgram => 0x02,
            Command::PageProgram4b => 0x12,
            Command::SectorErase => 0x20,
            Command::SectorErase4b => 0x21,
            Command::BlockErase32k => 0x52,
            Command::BlockErase64k => 0xd8,
            Command::BlockErase64k4b => 0xdc,
            Command::ChipErase => 0xc7,
            Command::DeepPowerDown => 0xb9,
            Command::ReleasePowerDown => 0xab,
            Command::ReadManufacturerDeviceId => 0x90,
            Command::ReadJedecId => 0x9f,
            Command::ReadSfdp => 0x5a,
            Command::ReadUniqueId => 0x4b,
            Command::Enter4bMode => 0xb7,
            Command::Exit4bMode => 0xe9,
            Command::ResetEnable => 0x66,
            Command::Reset => 0x99,
            Command::ProgramEraseSuspend => 0x75,
            Command::ProgramEraseResume => 0x7a,
            Command::ReadSecurityRegister => 0x48,
            Command::ProgramSecurityRegister => 0x42,
            Command::EraseSecurityRegister => 0x44,
            Command::BlockLock => 0x36,
            Command::BlockUnlock => 0x39,
            Command::GlobalBlockLock => 0x7e,
            Command::GlobalBlockUnlock => 0x98,
        }
    }

    /// The command `opcode` stands for, if the decoder knows it.
    pub const fn from_opcode(opcode: u8) -> Option<Command> {
        Some(match opcode {
            0x06 => Command::WriteEnable,
            0x04 => Command::WriteDisable,
            0x50 => Command::VolatileSrWriteEnable,
            0x05 => Command::ReadStatus,
            0x35 => Command::ReadStatus2,
            0x15 => Command::ReadStatus3,
            0x01 => Command::WriteStatus,
            0x31 => Command::WriteStatus2,
            0x11 => Command::WriteStatus3,
            0x03 => Command::Read,
            0x0b => Command::FastRead,
            0x13 => Command::Read4b,
            0x0c => Command::FastRead4b,
            0x02 => Command::PageProgram,
            0x12 => Command::PageProgram4b,
            0x20 => Command::SectorErase,
            0x21 => Command::SectorErase4b,
            0x52 => Command::BlockErase32k,
            0xd8 => Command::BlockErase64k,
            0xdc => Command::BlockErase64k4b,
            0xc7 | 0x60 => Command::ChipErase,
            0xb9 => Command::DeepPowerDown,
            0xab => Command::ReleasePowerDown,
            0x90 => Command::ReadManufacturerDeviceId,
            0x9f => Command::ReadJedecId,
            0x5a => Command::ReadSfdp,
            0x4b => Command::ReadUniqueId,
            0xb7 => Command::Enter4bMode,
            0xe9 => Command::Exit4bMode,
            0x66 => Command::ResetEnable,
            0x99 => Command::Reset,
            0x75 => Command::ProgramEraseSuspend,
            0x7a => Command::ProgramEraseResume,
            0x48 => Command::ReadSecurityRegister,
            0x42 => Command::ProgramSecurityRegister,
            0x44 => Command::EraseSecurityRegister,
            0x36 => Command::BlockLock,
            0x39 => Command::BlockUnlock,
            0x7e => Command::GlobalBlockLock,
            0x98 => Command::GlobalBlockUnlock,
            _ => return None,
        })
    }

    /// What the command shifts after its opcode.
    pub(crate) const fn layout(self) -> Layout {
        match self {
            Command::WriteEnable
            | Command::WriteDisable
            | Command::VolatileSrWriteEnable
            | Command::ChipErase
            | Command::DeepPowerDown
            | Command::Enter4bMode
            | Command::Exit4bMode
            | Command::ResetEnable
            | Command::Reset
            | Command::ProgramEraseSuspend
            | Command::ProgramEraseResume
            | Command::GlobalBlockLock
            | Command::GlobalBlockUnlock => Layout::bare(Phase::None),
            Command::ReadStatus => Layout::bare(Phase::ReadStatus(1)),
            Command::ReadStatus2 => Layout::bare(Phase::ReadStatus(2)),
            Command::ReadStatus3 => Layout::bare(Phase::ReadStatus(3)),
            Command::WriteStatus => Layout::bare(Phase::WriteStatus { first: 1, last: 2 }),
            Command::WriteStatus2 => Layout::bare(Phase::WriteStatus { first: 2, last: 2 }),
            Command::WriteStatus3 => Layout::bare(Phase::WriteStatus { first: 3, last: 3 }),
            Command::Read => Layout::addressed(Address::Mode, 0, Phase::DataOut),
            Command::FastRead | Command::ReadSecurityRegister => {
                Layout::addressed(Address::Mode, 1, Phase::DataOut)
            }
            Command::Read4b => Layout::addressed(Address::Four, 0, Phase::DataOut),
            Command::FastRead4b => Layout::addressed(Address::Four, 1, Phase::DataOut),
            Command::PageProgram | Command::ProgramSecurityRegister => {
                Layout::addressed(Address::Mode, 0, Phase::DataIn)
            }
            Command::PageProgram4b => Layout::addressed(Address::Four, 0, Phase::DataIn),
            Command::SectorErase
            | Command::BlockErase32k
            | Command::BlockErase64k
            | Command::EraseSecurityRegister
            | Command::BlockLock
            | Command::BlockUnlock => Layout::addressed(Address::Mode, 0, Phase::None),
            Command::SectorErase4b | Command::BlockErase64k4b => {
                Layout::addressed(Address::Four, 0, Phase::None)
            }
            Command::ReleasePowerDown => Layout::bare(Phase::Signature),
            Command::ReadManufacturerDeviceId => Layout::addressed(
                Address::Three,
                0,
                Phase::Id {
                    scheme: IdScheme::ManufacturerDevice,
                    len: Some(2),
                },
            ),
            Command::ReadJedecId => Layout::bare(Phase::Id {
                scheme: IdScheme::Jedec,
                len: None,
            }),
            // JESD216 defines the SFDP read with a three-byte address in every address mode.
            Command::ReadSfdp => Layout::addressed(Address::Three, 1, Phase::DataOut),
            Command::ReadUniqueId => Layout::addressed(
                Address::None,
                4,
                Phase::Id {
                    scheme: IdScheme::Unique,
                    len: Some(8),
                },
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opcodes_map_both_ways() {
        for command in Command::ALL {
            assert_eq!(Command::from_opcode(command.opcode()), Some(command));
        }
        assert_eq!(Command::from_opcode(0x60), Some(Command::ChipErase));
        let known = (0..=255u8)
            .filter(|opcode| Command::from_opcode(*opcode).is_some())
            .count();
        assert_eq!(known, 41, "40 commands, chip erase under two opcodes");
    }

    #[test]
    fn dual_and_quad_commands_are_not_known() {
        for opcode in [0x3b, 0x6b, 0xbb, 0xeb, 0x32, 0x38, 0x92, 0x94, 0x5b] {
            assert_eq!(Command::from_opcode(opcode), None, "{opcode:#04x}");
        }
    }

    #[test]
    fn four_byte_commands_ignore_the_mode() {
        for command in [
            Command::Read4b,
            Command::FastRead4b,
            Command::PageProgram4b,
            Command::SectorErase4b,
            Command::BlockErase64k4b,
        ] {
            assert_eq!(command.layout().address.bytes(3), 4, "{command:?}");
        }
        assert_eq!(Command::ReadSfdp.layout().address.bytes(4), 3);
        assert_eq!(
            Command::ReadManufacturerDeviceId.layout().address.bytes(4),
            3
        );
        assert_eq!(Command::Read.layout().address.bytes(4), 4);
        assert_eq!(Command::ReadJedecId.layout().address.bytes(4), 0);
    }
}
