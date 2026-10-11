// SPDX-License-Identifier: Apache-2.0

//! 93Cxx Microwire EEPROMs over `microwire.events`: the instructions of the 93C46, 93C56,
//! 93C66 and 93C86 families, decoded as they stream.
//!
//! Written from the public datasheets: Microchip's 93AA46A/B/C, 93LC46A/B/C and 93C46A/B/C
//! (DS21749), Atmel's AT93C46 and ST's M93C46, whose instruction sets agree.
//!
//! The decoding rules:
//!
//! - every Microwire transaction — one chip select period — carries at most one instruction.
//!   Clock pulses with DI low before the start bit, a 1 on DI, are no instruction and are
//!   skipped, as the device skips them; a transaction without a start bit carries none;
//! - after the start bit come two opcode bits and `address_bits` address bits, most
//!   significant first: READ `10`, WRITE `01` and ERASE `11` take an address; with opcode `00`
//!   the address's two top bits select EWEN `11`, EWDS `00`, ERAL `10` or WRAL `01`, and its
//!   other bits mean nothing. The instruction goes out once its address is whole, spanning the
//!   transaction from chip select through its last bit;
//! - READ: the device drives a dummy 0 on DO with the address's last bit, then a word of
//!   `organisation` width on DO, most significant bit first, and the next word after it for as
//!   long as chip select stays asserted: a sequential read. The words go out in batches of at
//!   most [`DATA_BATCH`], each as soon as it is full, the last when the read ends, so that a
//!   long read shows up while it is under way. Without a DO line no words are reported. A read
//!   is `complete` when chip select is released after one or more whole words, `incomplete`
//!   when it is released before its first word is whole or inside a word, whose bits make no
//!   word; the dummy bit is not checked;
//! - WRITE and WRAL take one word on DI after the address, which goes out once whole; ERASE,
//!   ERAL, EWEN and EWDS take nothing more. Clock pulses after what the instruction needs are
//!   skipped, and the operation ends `complete` when chip select is released, its end covering
//!   them; released before the instruction is whole, the device executes nothing and the
//!   operation ends `incomplete`. An instruction whose address the release cut short is
//!   reported with no address; one cut before it was known — in its opcode, or in the two
//!   address bits that select an instruction of opcode `00` — is not reported at all;
//! - after a complete WRITE, ERASE, ERAL or WRAL the device runs a programming cycle and shows
//!   its state on DO whenever chip select is asserted again before a start bit: low while busy,
//!   high once ready. The decoder reads that state from the clock pulses before a start bit and
//!   from DO's level at the end of a transaction without one, and reports `status` each time
//!   it differs from the last, until the device is ready or a start bit clears it;
//! - a transaction already under way when the samples begin or after a gap is skipped: its
//!   first bits are not the instruction's;
//! - a gap or the end of the samples during an instruction ends it `aborted`, after the words
//!   read so far; a gap also forgets a programming cycle under way, whose end it may have
//!   hidden;
//! - events carry bus data only: whether erasing and writing are enabled, the address counter
//!   of a sequential read and the array's size are not modelled.
//!
//! The decoder requires the Microwire stream to be framed by chip select and to carry DI; DO
//! is optional.

mod decoder;
mod events;
pub mod synth;

pub use decoder::{Eeprom93xx, Eeprom93xxConfig, Eeprom93xxConfigError, Organisation};
pub use events::{
    Command, DATA_BATCH, Direction, EEPROM93XX_EVENTS, Eeprom93xxEvent, EndReason, OpId,
};
