// SPDX-License-Identifier: Apache-2.0

//! JTAG (IEEE 1149.1): the decoder from logic samples to `jtag.events` — the TAP controller's
//! states, its resets and the scans through its registers — the reader decoders built on JTAG
//! use, and a waveform synthesizer.

mod bus;
mod decoder;
mod events;
pub mod synth;

pub use bus::{Bit, JtagBus, Line, Next, Read, ReadError, Scan};
pub use decoder::{InitialState, Jtag, JtagConfig, JtagConfigError, RESET_CLOCKS};
pub use events::{
    EndReason, JTAG_EVENTS, JtagEvent, Register, ResetBy, SHIFT_BATCH_BITS, ScanId, TapState,
};
