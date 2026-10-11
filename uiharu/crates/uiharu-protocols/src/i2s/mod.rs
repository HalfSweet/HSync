// SPDX-License-Identifier: Apache-2.0

//! I²S: the decoder from logic samples to `i2s.events`, which reads the audio samples of an
//! I²S bus in the I²S, left-justified or right-justified format, and a waveform synthesizer.

mod decoder;
mod events;
pub mod synth;

pub use decoder::{Format, I2s, I2sConfig, I2sConfigError, MAX_WORD_BITS, SampleEdge};
pub use events::{Channel, I2S_EVENTS, I2sEvent};
