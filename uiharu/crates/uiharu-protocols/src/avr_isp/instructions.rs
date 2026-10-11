// SPDX-License-Identifier: Apache-2.0

//! The serial programming instruction set: which bytes name which instruction, and which of
//! its bytes carry what.

use super::events::Command;

/// What an instruction's bytes carry besides its name.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Fields {
    /// Bytes two and three are an address, the second most significant.
    pub(crate) address: bool,
    /// Byte four is data from the programmer.
    pub(crate) data_in: bool,
    /// What MISO carries during byte four is data from the device.
    pub(crate) data_out: bool,
    /// What MISO carries during byte three is the echo of byte two: Programming Enable's
    /// check that the device is in sync.
    pub(crate) echo: bool,
}

impl Fields {
    const NONE: Fields = Fields {
        address: false,
        data_in: false,
        data_out: false,
        echo: false,
    };
    const ADDRESS: Fields = Fields {
        address: true,
        ..Fields::NONE
    };
    const DATA_IN: Fields = Fields {
        data_in: true,
        ..Fields::NONE
    };
    const DATA_OUT: Fields = Fields {
        data_out: true,
        ..Fields::NONE
    };
    const ADDRESS_IN: Fields = Fields {
        address: true,
        data_in: true,
        ..Fields::NONE
    };
    const ADDRESS_OUT: Fields = Fields {
        address: true,
        data_out: true,
        ..Fields::NONE
    };
    const ECHO: Fields = Fields {
        echo: true,
        ..Fields::NONE
    };
}

impl Command {
    /// Every instruction, in the order of the contract.
    pub const ALL: [Command; 23] = [
        Command::ProgrammingEnable,
        Command::ChipErase,
        Command::PollRdyBsy,
        Command::LoadExtendedAddress,
        Command::LoadProgramMemoryPageHigh,
        Command::LoadProgramMemoryPageLow,
        Command::LoadEepromMemoryPage,
        Command::ReadProgramMemoryHigh,
        Command::ReadProgramMemoryLow,
        Command::ReadEepromMemory,
        Command::ReadLockBits,
        Command::ReadSignatureByte,
        Command::ReadFuseBits,
        Command::ReadFuseHighBits,
        Command::ReadExtendedFuseBits,
        Command::ReadCalibrationByte,
        Command::WriteProgramMemoryPage,
        Command::WriteEepromMemory,
        Command::WriteEepromMemoryPage,
        Command::WriteLockBits,
        Command::WriteFuseBits,
        Command::WriteFuseHighBits,
        Command::WriteExtendedFuseBits,
    ];

    /// The instruction whose first two bytes are `first` and `second`, if the set lists it.
    /// The second byte only counts where the set fixes it; where it is an address byte, any
    /// value names the instruction.
    pub const fn from_bytes(first: u8, second: u8) -> Option<Command> {
        Some(match (first, second) {
            (0xac, 0x53) => Command::ProgrammingEnable,
            (0xac, 0x80) => Command::ChipErase,
            (0xac, 0xe0) => Command::WriteLockBits,
            (0xac, 0xa0) => Command::WriteFuseBits,
            (0xac, 0xa8) => Command::WriteFuseHighBits,
            (0xac, 0xa4) => Command::WriteExtendedFuseBits,
            (0xf0, 0x00) => Command::PollRdyBsy,
            (0x4d, 0x00) => Command::LoadExtendedAddress,
            (0x48, 0x00) => Command::LoadProgramMemoryPageHigh,
            (0x40, 0x00) => Command::LoadProgramMemoryPageLow,
            (0xc1, 0x00) => Command::LoadEepromMemoryPage,
            (0x28, _) => Command::ReadProgramMemoryHigh,
            (0x20, _) => Command::ReadProgramMemoryLow,
            (0xa0, _) => Command::ReadEepromMemory,
            (0x58, 0x00) => Command::ReadLockBits,
            (0x58, 0x08) => Command::ReadFuseHighBits,
            (0x30, 0x00) => Command::ReadSignatureByte,
            (0x50, 0x00) => Command::ReadFuseBits,
            (0x50, 0x08) => Command::ReadExtendedFuseBits,
            (0x38, 0x00) => Command::ReadCalibrationByte,
            (0x4c, _) => Command::WriteProgramMemoryPage,
            (0xc0, _) => Command::WriteEepromMemory,
            (0xc2, _) => Command::WriteEepromMemoryPage,
            _ => return None,
        })
    }

    /// What the instruction's bytes carry.
    pub(crate) const fn fields(self) -> Fields {
        match self {
            Command::ProgrammingEnable => Fields::ECHO,
            Command::ChipErase => Fields::NONE,
            Command::PollRdyBsy
            | Command::ReadLockBits
            | Command::ReadFuseBits
            | Command::ReadFuseHighBits
            | Command::ReadExtendedFuseBits
            | Command::ReadCalibrationByte => Fields::DATA_OUT,
            Command::LoadExtendedAddress
            | Command::WriteProgramMemoryPage
            | Command::WriteEepromMemoryPage => Fields::ADDRESS,
            Command::LoadProgramMemoryPageHigh
            | Command::LoadProgramMemoryPageLow
            | Command::LoadEepromMemoryPage
            | Command::WriteEepromMemory => Fields::ADDRESS_IN,
            Command::ReadProgramMemoryHigh
            | Command::ReadProgramMemoryLow
            | Command::ReadEepromMemory
            | Command::ReadSignatureByte => Fields::ADDRESS_OUT,
            Command::WriteLockBits
            | Command::WriteFuseBits
            | Command::WriteFuseHighBits
            | Command::WriteExtendedFuseBits => Fields::DATA_IN,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_instruction_is_named_by_its_first_bytes() {
        let examples = [
            ([0xac, 0x53], Command::ProgrammingEnable),
            ([0xac, 0x80], Command::ChipErase),
            ([0xf0, 0x00], Command::PollRdyBsy),
            ([0x4d, 0x00], Command::LoadExtendedAddress),
            ([0x48, 0x00], Command::LoadProgramMemoryPageHigh),
            ([0x40, 0x00], Command::LoadProgramMemoryPageLow),
            ([0xc1, 0x00], Command::LoadEepromMemoryPage),
            ([0x28, 0x01], Command::ReadProgramMemoryHigh),
            ([0x20, 0x3f], Command::ReadProgramMemoryLow),
            ([0xa0, 0x03], Command::ReadEepromMemory),
            ([0x58, 0x00], Command::ReadLockBits),
            ([0x30, 0x00], Command::ReadSignatureByte),
            ([0x50, 0x00], Command::ReadFuseBits),
            ([0x58, 0x08], Command::ReadFuseHighBits),
            ([0x50, 0x08], Command::ReadExtendedFuseBits),
            ([0x38, 0x00], Command::ReadCalibrationByte),
            ([0x4c, 0x00], Command::WriteProgramMemoryPage),
            ([0xc0, 0x01], Command::WriteEepromMemory),
            ([0xc2, 0x00], Command::WriteEepromMemoryPage),
            ([0xac, 0xe0], Command::WriteLockBits),
            ([0xac, 0xa0], Command::WriteFuseBits),
            ([0xac, 0xa8], Command::WriteFuseHighBits),
            ([0xac, 0xa4], Command::WriteExtendedFuseBits),
        ];
        assert_eq!(examples.len(), Command::ALL.len());
        for ([first, second], command) in examples {
            assert_eq!(Command::from_bytes(first, second), Some(command));
        }
        for command in Command::ALL {
            assert!(examples.iter().any(|(_, named)| *named == command));
        }
    }

    #[test]
    fn a_second_byte_the_set_fixes_must_match() {
        for (first, second) in [
            (0xac, 0x00),
            (0xac, 0x54),
            (0xf0, 0x01),
            (0x4d, 0x01),
            (0x48, 0x80),
            (0x58, 0x01),
            (0x50, 0x09),
            (0x30, 0x08),
            (0x38, 0x01),
            (0xff, 0xff),
            (0x00, 0x00),
        ] {
            assert_eq!(
                Command::from_bytes(first, second),
                None,
                "{first:02x} {second:02x}"
            );
        }
        // Address bytes in the second place may be anything.
        for second in [0x00, 0x7f, 0xff] {
            assert_eq!(
                Command::from_bytes(0xa0, second),
                Some(Command::ReadEepromMemory)
            );
            assert_eq!(
                Command::from_bytes(0x4c, second),
                Some(Command::WriteProgramMemoryPage)
            );
        }
    }

    #[test]
    fn fields_follow_the_instruction() {
        let fields = |command: Command| command.fields();
        assert!(fields(Command::ProgrammingEnable).echo);
        assert_eq!(fields(Command::ChipErase), Fields::default());
        let read = fields(Command::ReadProgramMemoryLow);
        assert!(read.address && read.data_out && !read.data_in);
        let load = fields(Command::LoadProgramMemoryPageHigh);
        assert!(load.address && load.data_in && !load.data_out);
        let fuse = fields(Command::WriteFuseBits);
        assert!(fuse.data_in && !fuse.address);
        assert!(fields(Command::PollRdyBsy).data_out);
        assert!(fields(Command::ReadSignatureByte).address);
        let page = fields(Command::WriteEepromMemoryPage);
        assert!(page.address && !page.data_in);
        for command in Command::ALL {
            let fields = command.fields();
            assert!(
                !(fields.data_in && fields.data_out),
                "{command:?} moves data both ways"
            );
            assert_eq!(fields.echo, command == Command::ProgrammingEnable);
        }
    }
}
