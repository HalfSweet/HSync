// SPDX-License-Identifier: Apache-2.0

//! AVR in-system programming over `spi.events`: the four-byte instructions of the serial
//! programming interface, decoded as they stream.
//!
//! Written from the serial programming sections of Microchip (formerly Atmel) AVR datasheets —
//! the "Serial Downloading" description and the "Serial Programming Instruction Set" table of
//! the ATmega48A/PA/88A/PA/168A/PA/328/P datasheet, with the Load Extended Address byte of the
//! larger devices such as the ATmega640/1280/2560. The rules:
//!
//! - an instruction is four bytes on MOSI, four SPI words in a row; the device answers on MISO
//!   a byte late, so the decoder reads MISO in two places only: during the third byte of
//!   Programming Enable, where a device in sync echoes the second byte, `53`, and during the
//!   fourth byte of the reads and Poll RDY/BSY, which carries their answer;
//! - chip select, when the bus has one, is the target's RESET, held low through a programming
//!   session: the words of a transaction seen to begin are grouped in fours from its first
//!   word;
//! - anywhere else the grouping is not known — in a transaction already under way at the first
//!   sample or after a gap, whose words may not even be aligned on bytes, and on a bus without
//!   chip select, at the first sample and after every gap. There words are skipped until a
//!   Programming Enable answered in sync — `AC 53` on MOSI, and the echo `53` on MISO during
//!   the next byte — and grouped from its first byte on: that is how a programmer itself gets
//!   in step with the device, and the echo keeps data that happens to hold `AC 53` from
//!   passing for it;
//! - the first byte names the instruction, and the second where the set fixes it (`AC` is
//!   followed by `53`, `80`, `E0`, `A0`, `A8` or `A4`; `F0`, `4D`, `48`, `40`, `C1`, `30`
//!   and `38` by `00`; `58` and `50` by `00` or `08`); four bytes the set does not name are
//!   reported as `unknown`, and the grouping goes on;
//! - an instruction goes out once its fourth byte is in, as one event spanning its four
//!   words, with the fields its bytes carry: the address of bytes two and three, the second
//!   most significant (for Load Extended Address, the extended address byte); the data byte
//!   the programmer sends fourth; the device's answer during the fourth byte; and
//!   Programming Enable's echo. The assertion of chip select belongs to no instruction;
//! - an instruction cut short is an `error` with the bytes sent so far: `incomplete` when chip
//!   select was released, covering the words through the release; `aborted` when a gap, the
//!   end of the samples or an aborted transaction cut it, placed at the cut. Words that only
//!   may begin Programming Enable, while the grouping is not known, are no instruction yet and
//!   are dropped silently;
//! - progress is passed on, held back to the first word held;
//! - events carry bus data only: fuse and lock bits, signature bytes and memory contents are
//!   not interpreted, and the decoder does not judge whether the device accepted an
//!   instruction.
//!
//! Limits: the decoder does not follow the session's state — whether programming was enabled,
//! which extended address or page is loaded — and checks no echo but Programming Enable's.
//! Without chip select, a capture that starts after the session's Programming Enable decodes
//! nothing until the next one. The instruction sets of the older AT90S devices, and the TPI,
//! PDI and UPDI interfaces, are out of scope.

mod decoder;
mod events;
mod instructions;
pub mod synth;

pub use decoder::{AvrIsp, SYNC_BYTES};
pub use events::{AVR_ISP_EVENTS, Command, ErrorReason, INSTRUCTION_BYTES, IspEvent, OpId};
