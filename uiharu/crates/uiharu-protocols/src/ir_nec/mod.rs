// SPDX-License-Identifier: Apache-2.0

//! NEC infrared remote control: the decoder from the demodulated output of an infrared receiver
//! to `nec.events`, and a waveform synthesizer.
//!
//! An NEC remote control sends a frame — a leader, an address, a command and their inverses —
//! when a key is pressed, and a repeat code every 108 ms while the key stays pressed. The
//! receiver module demodulates the 38 kHz carrier: its output is at mark while the carrier is
//! received. The stream is unframed: a frame, a repeat code or a frame that went wrong is one
//! event each.

mod decoder;
mod events;
pub mod synth;

pub use decoder::{MAX_TOLERANCE, Nec, NecConfig, NecConfigError, Polarity};
pub use events::{ErrorReason, IR_NEC_EVENTS, NecEvent};
