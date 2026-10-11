// SPDX-License-Identifier: Apache-2.0

//! WS281x: the decoder from the logic samples of the data line of a chain of WS2812B-class
//! LEDs (WS2812B, SK6812 and their RGBW kin) to `ws281x.events`, and a waveform synthesizer.
//!
//! The line carries frames of LED data: each bit a high pulse whose length tells a 0 from a 1,
//! the bits grouped into LEDs of 24 or 32 bits, and a long low, the reset, that latches the
//! frame. The decoder, [`Ws281x`], reports each LED's bytes as they are on the wire; its rules
//! are listed in its module's documentation and on the protocol's page,
//! `docs/protocols/ws281x.md`.

mod decoder;
mod events;
pub mod synth;

pub use decoder::{Ws281x, Ws281xConfig, Ws281xConfigError};
pub use events::{EndReason, ErrorReason, FrameId, WS281X_EVENTS, Ws281xEvent};
