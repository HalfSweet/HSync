// SPDX-License-Identifier: Apache-2.0

//! Edge timing: the decoder from logic samples to `timing.events`, which reports the edges of
//! one line with the ticks since the previous one, and a waveform synthesizer.

mod decoder;
mod events;
pub mod synth;

pub use decoder::{Edges, Timing, TimingConfig, TimingConfigError};
pub use events::{Slope, TIMING_EVENTS, TimingEvent};
