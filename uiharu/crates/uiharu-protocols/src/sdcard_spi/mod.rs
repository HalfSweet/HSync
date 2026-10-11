// SPDX-License-Identifier: Apache-2.0

//! SD memory cards in SPI mode over `spi.events`: command frames, responses, control tokens and
//! data blocks, decoded as they stream.
//!
//! Written from the SD Association's Physical Layer Simplified Specification, version 9.10,
//! Part 1: chapter 7, SPI Mode — the formats of commands, responses and tokens (§7.3) and the
//! timing of responses and data (§7.5) — with the CRCs of §4.5 and the commands of §4.7.
//!
//! The decoding rules:
//!
//! - an SPI transaction — one chip select period — holds any number of operations, each a
//!   command, the card's response and the data the command moves;
//! - between operations MOSI carries fill (`0xFF`); a byte `01xxxxxx` begins a six-byte command
//!   frame: the index, the 32-bit argument most significant byte first, the CRC7
//!   (x⁷ + x³ + 1) and an end bit, which is not checked; `crc_ok` says whether the CRC7 matches,
//!   which a card in SPI mode checks only for CMD0 and CMD8 unless CMD59 turned checking on;
//! - a transaction whose first byte on MOSI that is not fill does not begin a command frame is
//!   reported as `unknown-command` and skipped; stray bytes between operations are skipped; a
//!   transaction whose MOSI bytes are all fill is skipped whatever MISO carries — busy polling
//!   looks like that, and so does a multiple-block read that a host goes on clocking in a new
//!   chip select period, whose data comes without the command that asked for it;
//! - a command frame cut short by chip select or a gap is no command and is dropped;
//! - the response is the first byte from the card with bit 7 clear; the fill before it is not
//!   reported, and after CMD12 the first byte is a stuff byte, skipped whatever it holds;
//! - its format follows the command — R1b for CMD12, CMD20, CMD28, CMD29 and CMD38, R2 for
//!   CMD13 and ACMD13, R3 for CMD58, R7 for CMD8, R1 for the others — except that a card that
//!   refuses a command (illegal command or CRC error in R1) answers with R1 alone, reported as
//!   `r1`;
//! - R1b's busy bytes (`0x00`) are counted until the card turns ready (any other byte), the
//!   host starts its next command or releases chip select, and the response goes out then;
//! - a command whose R1 has an error flag moves no data; otherwise a read waits for the start
//!   block token (`0xFE`) or a data error token (`0000xxxx`) and takes the block and its CRC16
//!   (x¹⁶ + x¹² + x⁵ + 1), and a write waits for the host's token (`0xFE`, `0xFC` for each block
//!   of CMD25), takes the block and its CRC16 from MOSI, then the card's data response token
//!   (`xxx0sss1`); blocks are the block length long for CMD17, CMD18, CMD24, CMD25, CMD42 and
//!   CMD56, 16 bytes for CMD9, CMD10 and CMD27, 8 for ACMD51, 64 for CMD6 and ACMD13, 4 for
//!   CMD30 and ACMD22, 512 for CMD48 and CMD49;
//! - CMD18 reads blocks until the host starts its next command, normally CMD12, and CMD25
//!   writes blocks until the stop token (`0xFD`) or a command; either ends the operation
//!   `complete`; a single-block operation that a command interrupts ends `incomplete`, and so
//!   does any operation whose chip select is released before its response or data is whole —
//!   R1b's busy period aside;
//! - the busy period after a data response or a stop token belongs to no operation and is
//!   skipped;
//! - data goes out in batches of at most [`DATA_BATCH`] bytes, each as soon as it is full and
//!   the last of a block with the block's last byte, followed by the block's CRC;
//! - the block length is the configured one until a CMD16 whose R1 has no error flag sets
//!   another (1 to [`MAX_BLOCK_LENGTH`] bytes), and CMD0 returns it to 512 bytes; a gap leaves
//!   it as it was, as it leaves the card's;
//! - the command after a CMD55 the card did not refuse is an application command when its index
//!   names one (ACMD6, 13, 18, 22, 23, 25, 26, 38, 41, 42, 43–49, 51), a regular command
//!   otherwise; a gap forgets a CMD55;
//! - a transaction under way when the samples begin or after a gap is skipped, since its words
//!   may not be aligned on bytes; a gap or the end of the samples ends the open operation
//!   `aborted`;
//! - events carry bus data only: the response flags and the registers are not interpreted.
//!
//! The decoder requires the SPI stream to carry 8-bit words, most significant bit first, framed
//! by chip select, with both data lines; the host is expected to send fill while it reads.
//! High-capacity cards (SDHC, SDXC) keep 512-byte blocks for reads and writes whatever CMD16
//! says; hosts do not send them CMD16 for that, and the decoder follows CMD16 regardless.
//! SDIO responses (R4, R5) and MultiMediaCards are out of scope.

mod commands;
mod crc;
mod decoder;
mod events;
pub mod synth;

pub use commands::{DEFAULT_BLOCK_LENGTH, MAX_BLOCK_LENGTH};
pub use decoder::{SdCardSpi, SdCardSpiConfig, SdCardSpiConfigError};
pub use events::{DATA_BATCH, OpId, ResponseFormat, SDCARD_EVENTS, SdEnd, SdEvent, Token};
