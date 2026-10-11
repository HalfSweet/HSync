// SPDX-License-Identifier: Apache-2.0

//! Philips RC-6 infrared remote control, mode 0: the decoder from the demodulated output of an
//! infrared receiver to `ir_rc6.events`, and a waveform synthesizer.
//!
//! An RC-6 remote control sends a frame — a leader, a start bit, three mode bits, a trailer bit
//! of double length that carries the toggle, then in mode 0 an 8-bit address and an 8-bit
//! command — every time a key is pressed or held; the toggle changes at every new press. The
//! receiver module demodulates the 36 kHz carrier: its output is at mark while the carrier is
//! received. The stream is unframed: a frame, or a frame that went wrong, is one event each.

mod decoder;
mod events;
pub mod synth;

pub use decoder::{MAX_TOLERANCE, Polarity, Rc6, Rc6Config, Rc6ConfigError};
pub use events::{ErrorReason, IR_RC6_EVENTS, Rc6Event};
