// SPDX-License-Identifier: Apache-2.0

//! SPI NOR flash commands over `spi.events`: the single-line command set of serial NOR flash,
//! decoded as it streams.
//!
//! Written from the public specifications: the instruction sets in the datasheets of the
//! Winbond W25Q64JV, the Macronix MX25L6433F and the Micron N25Q and MT25Q families (with the
//! four-byte address mode of their larger members), and JEDEC JESD216 for the Serial Flash
//! Discoverable Parameters read.
//!
//! The decoding rules:
//!
//! - every SPI transaction — one chip select period — carries one command, whose first byte on
//!   MOSI is the opcode; an opcode the decoder does not know is reported as `unknown-command`
//!   and the rest of its transaction is skipped;
//! - dual and quad I/O commands (`0x3B`, `0x6B`, `0xBB`, `0xEB`, `0x32`, `0x38`, `0x92`,
//!   `0x94` …) and QPI move their address or data over two or four lines, which a single-line
//!   bus does not show: they are unknown commands;
//! - after the opcode come, as the command calls for, the address on MOSI, most significant
//!   byte first; dummy bytes, whose values mean nothing; and the command's data: data out on
//!   MISO for reads, data in on MOSI for programs, status register values, identification
//!   bytes;
//! - the four-byte commands (`0x13`, `0x0C`, `0x12`, `0x21`, `0xDC`) take four address bytes,
//!   SFDP (`0x5A`, as JESD216 defines it) and REMS (`0x90`) three; every other addressed
//!   command takes the configured width: three, four, or with [`AddressBytes::Auto`] three
//!   until Enter 4-Byte Address Mode (`0xB7`) completes, four until Exit 4-Byte Address Mode
//!   (`0xE9`) or a software reset — Reset Enable (`0x66`) directly followed by Reset (`0x99`)
//!   — completes, and three again after a gap, which may have hidden either;
//! - what the host sends must be whole: the opcode, the address, the dummy bytes, at least one
//!   byte for a program or a status register write, and RES's three dummy bytes when anything
//!   follows its opcode; chip select released earlier ends the operation `incomplete`;
//! - what the flash sends lasts as long as the host clocks it: a read, a status read or an
//!   identification read may stop after any byte, or before the first, and is `complete`;
//! - data and JEDEC ID bytes go out in batches of at most [`DATA_BATCH`] bytes, each as soon
//!   as it is full and the last when the operation ends, so that a long read shows up while it
//!   is under way; status values go out one by one; the fixed-length ids — two bytes of REMS,
//!   eight of RUID, one of RES — go out once whole, or as far as they were read when the
//!   operation ends;
//! - bytes after a command that takes no more — past an erase's address, a fixed-length id or
//!   a status register write's last register — are skipped, and the operation ends `complete`
//!   when chip select is released, its end covering them;
//! - a transaction already under way when the samples begin or after a gap is skipped: its
//!   first word is not the opcode;
//! - a gap or the end of the samples during a command ends it `aborted`, after the data read
//!   so far;
//! - events carry bus data only: status register bits, manufacturer codes and SFDP tables are
//!   not interpreted.
//!
//! Where vendors give an opcode different meanings, the decoder takes the Winbond W25Q one.
//! The decoder requires the SPI stream to carry 8-bit words, most significant bit first,
//! framed by chip select, with both data lines.

mod commands;
mod decoder;
mod events;
pub mod synth;

pub use decoder::{AddressBytes, SpiNor, SpiNorConfig};
pub use events::{Command, DATA_BATCH, Direction, IdScheme, NOR_EVENTS, NorEnd, NorEvent, OpId};
