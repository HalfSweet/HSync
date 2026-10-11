// SPDX-License-Identifier: Apache-2.0

//! Sony SIRC infrared remote control: the decoder from the demodulated output of an infrared
//! receiver to `ir_sirc.events`, and a waveform synthesizer.
//!
//! A SIRC remote control sends a frame — a start mark, then 12, 15 or 20 pulse-width coded bits
//! carrying a command, an address and in 20-bit frames an extended field — every 45 ms while a
//! key is held. Nothing in a frame says how long it is: it ends with the silence after its last
//! bit. The receiver module demodulates the 40 kHz carrier: its output is at mark while the
//! carrier is received. The stream is unframed: a frame, or a frame that went wrong, is one
//! event each.

mod decoder;
mod events;
pub mod synth;

pub use decoder::{MAX_TOLERANCE, Polarity, Sirc, SircConfig, SircConfigError};
pub use events::{ErrorReason, IR_SIRC_EVENTS, SircEvent};
