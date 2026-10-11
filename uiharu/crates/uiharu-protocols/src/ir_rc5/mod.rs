// SPDX-License-Identifier: Apache-2.0

//! Philips RC-5 infrared remote control, and its extension RC-5X: the decoder from the
//! demodulated output of an infrared receiver to `rc5.events`, and a waveform synthesizer.
//!
//! An RC-5 remote control sends a frame of 14 bi-phase coded bits — two start bits, a toggle
//! bit, a 5-bit address and a 6-bit command — when a key is pressed, and again every 113.8 ms
//! while it stays pressed; the toggle bit changes at every new press. RC-5X turns the second
//! start bit into a seventh command bit. The receiver module demodulates the 36 kHz carrier:
//! its output is at mark while the carrier is received. The stream is unframed: a frame, or a
//! frame that went wrong, is one event each.

mod decoder;
mod events;
pub mod synth;

pub use decoder::{MAX_TOLERANCE, Polarity, Rc5, Rc5Config, Rc5ConfigError};
pub use events::{ErrorReason, IR_RC5_EVENTS, Rc5Event};
