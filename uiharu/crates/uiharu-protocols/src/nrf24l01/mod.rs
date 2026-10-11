// SPDX-License-Identifier: Apache-2.0

//! The nRF24L01+ 2.4 GHz transceiver over `spi.events`: its SPI commands, registers and
//! payloads, decoded as they stream.
//!
//! Written from the Nordic Semiconductor nRF24L01+ Product Specification v1.0: chapter 8, the
//! SPI interface and its command set, and chapter 9, the register map; with ACTIVATE from the
//! nRF24L01 Product Specification v2.0, the part before the plus. The rules:
//!
//! - every SPI transaction — one chip select period — carries one command: its first word is
//!   the command byte on MOSI, with the STATUS register on MISO, and the words after it are
//!   the command's data, on MOSI for the commands that write and on MISO for those that read;
//! - the register commands carry the register's address in their five low bits, W_ACK_PAYLOAD
//!   its pipe, 0 to 5, in its three low bits. A byte the command set does not define is
//!   reported as `unknown`, with STATUS, and the rest of its transaction is skipped. `0x50` is
//!   ACTIVATE, of the nRF24L01 before the plus, only when `0x73` follows; otherwise it is
//!   unknown, as the next byte or the release decides;
//! - registers are named after the register map; the addresses it leaves out (`0x18` to
//!   `0x1B`, `0x1E`, `0x1F`) have no name;
//! - a register command takes 1 to as many bytes as its register is wide: five for RX_ADDR_P0,
//!   RX_ADDR_P1 and TX_ADDR, of which as many as SETUP_AW sets are used, five for an address
//!   the map leaves out, one for every other register. A payload command takes 1 to
//!   [`MAX_PAYLOAD`] bytes, R_RX_PL_WID one, ACTIVATE its key; FLUSH_TX, FLUSH_RX, REUSE_TX_PL
//!   and NOP none;
//! - the command ends `complete` when chip select is released after its fewest bytes,
//!   `incomplete` before them — a register or payload command without a data byte — and
//!   `too-long` at the first byte past the most it takes, after which the rest of the
//!   transaction is skipped;
//! - data goes out in one batch of at most [`DATA_BATCH`] bytes, as soon as the command has the
//!   most bytes it takes, or when its end decides it; the bytes are in bus order, so a
//!   multi-byte register comes least significant byte first;
//! - a transaction already under way when the samples begin or after a gap is skipped: its
//!   first word is not the command byte, and its words may not be aligned on bytes;
//! - a gap or the end of the samples during a command ends it `aborted`, after the data read
//!   so far; the gap or the end is then passed on;
//! - events carry bus data only: STATUS, register values and payloads are not interpreted.
//!
//! The decoder requires the SPI stream to carry 8-bit words, most significant bit first,
//! framed by chip select, with both data lines; the chip uses SPI mode 0. Limits: the decoder
//! does not follow the chip's configuration — SETUP_AW, the payload widths, dynamic payloads —
//! so it takes up to five address bytes and up to 32 payload bytes whatever the chip is set
//! to, and does not judge whether the chip accepted a command (W_REGISTER outside power down
//! and standby, payload commands with a full FIFO).

mod commands;
mod decoder;
mod events;
pub mod synth;

pub use commands::{ACTIVATE, ACTIVATE_KEY, MAX_PAYLOAD, MAX_REGISTER_BYTES};
pub use decoder::Nrf24l01;
pub use events::{
    Command, DATA_BATCH, Direction, EndReason, NRF24L01_EVENTS, Nrf24Event, OpId, Register,
};
