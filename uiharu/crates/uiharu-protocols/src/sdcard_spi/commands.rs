// SPDX-License-Identifier: Apache-2.0

//! What each command brings about in SPI mode — its response format and its data transfer —
//! after the command lists of the specification (§7.3.1.3, with §4.7 for the application
//! commands).

use super::events::ResponseFormat;

/// A card's block length after power-up or a reset.
pub const DEFAULT_BLOCK_LENGTH: u16 = 512;

/// The longest block the decoder follows: a CSD may declare up to 2048-byte blocks.
pub const MAX_BLOCK_LENGTH: u16 = 2048;

/// R1 flags under which the card refused the command and answers with R1 alone: illegal
/// command, CRC error.
pub(crate) const R1_REFUSED: u8 = 0b0000_1100;

/// R1 flags under which no data follows: illegal command, CRC error, erase sequence error,
/// address error, parameter error.
pub(crate) const R1_ERRORS: u8 = 0b0111_1100;

/// How long a data block is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Length {
    /// The card's block length: 512 bytes, or what CMD16 set.
    Block,
    /// A register or a status of fixed length.
    Fixed(u16),
}

/// The data a command moves after its response.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Transfer {
    /// None.
    None,
    /// One block from the card.
    Read(Length),
    /// Blocks from the card until the host stops them.
    ReadMultiple,
    /// One block from the host.
    Write(Length),
    /// Blocks from the host until it sends the stop token.
    WriteMultiple,
}

/// What a command brings about after its frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Shape {
    pub(crate) format: ResponseFormat,
    pub(crate) transfer: Transfer,
}

/// Whether `index`, after a CMD55, names an application command.
pub(crate) const fn is_application(index: u8) -> bool {
    matches!(
        index,
        6 | 13 | 18 | 22 | 23 | 25 | 26 | 38 | 41 | 42 | 43..=49 | 51
    )
}

/// What command `index` brings about, `app` telling whether it is an application command;
/// bit 0 of `argument` gives the direction of CMD56.
pub(crate) const fn shape(index: u8, app: bool, argument: u32) -> Shape {
    use Length::{Block, Fixed};
    use ResponseFormat::{R1, R1b, R2, R3, R7};
    let (format, transfer) = if app {
        match index {
            13 => (R2, Transfer::Read(Fixed(64))),
            22 => (R1, Transfer::Read(Fixed(4))),
            51 => (R1, Transfer::Read(Fixed(8))),
            _ => (R1, Transfer::None),
        }
    } else {
        match index {
            6 => (R1, Transfer::Read(Fixed(64))),
            8 => (R7, Transfer::None),
            9 | 10 => (R1, Transfer::Read(Fixed(16))),
            12 | 20 | 28 | 29 | 38 => (R1b, Transfer::None),
            13 => (R2, Transfer::None),
            17 => (R1, Transfer::Read(Block)),
            18 => (R1, Transfer::ReadMultiple),
            24 | 42 => (R1, Transfer::Write(Block)),
            25 => (R1, Transfer::WriteMultiple),
            27 => (R1, Transfer::Write(Fixed(16))),
            30 => (R1, Transfer::Read(Fixed(4))),
            48 => (R1, Transfer::Read(Fixed(512))),
            49 => (R1, Transfer::Write(Fixed(512))),
            56 if argument & 1 != 0 => (R1, Transfer::Read(Block)),
            56 => (R1, Transfer::Write(Block)),
            58 => (R3, Transfer::None),
            _ => (R1, Transfer::None),
        }
    };
    Shape { format, transfer }
}

/// The number of bytes the response takes after R1, unless the card refused the command.
pub(crate) const fn tail(format: ResponseFormat) -> usize {
    match format {
        ResponseFormat::R1 | ResponseFormat::R1b => 0,
        ResponseFormat::R2 => 1,
        ResponseFormat::R3 | ResponseFormat::R7 => 4,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_reads_have_their_lengths() {
        let read = |index, app| shape(index, app, 0).transfer;
        assert_eq!(read(9, false), Transfer::Read(Length::Fixed(16)));
        assert_eq!(read(10, false), Transfer::Read(Length::Fixed(16)));
        assert_eq!(read(51, true), Transfer::Read(Length::Fixed(8)));
        assert_eq!(read(13, true), Transfer::Read(Length::Fixed(64)));
        assert_eq!(read(30, false), Transfer::Read(Length::Fixed(4)));
        assert_eq!(read(17, false), Transfer::Read(Length::Block));
        assert_eq!(shape(56, false, 1).transfer, Transfer::Read(Length::Block));
        assert_eq!(shape(56, false, 0).transfer, Transfer::Write(Length::Block));
    }

    #[test]
    fn responses_have_their_formats() {
        let format = |index, app| shape(index, app, 0).format;
        assert_eq!(format(0, false), ResponseFormat::R1);
        assert_eq!(format(8, false), ResponseFormat::R7);
        assert_eq!(format(12, false), ResponseFormat::R1b);
        assert_eq!(format(13, false), ResponseFormat::R2);
        assert_eq!(format(13, true), ResponseFormat::R2);
        assert_eq!(format(58, false), ResponseFormat::R3);
        assert_eq!(format(41, true), ResponseFormat::R1);
        assert_eq!(tail(ResponseFormat::R7), 4);
        assert!(is_application(41));
        assert!(!is_application(17));
    }
}
