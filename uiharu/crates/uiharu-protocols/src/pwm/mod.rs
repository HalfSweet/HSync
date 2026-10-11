// SPDX-License-Identifier: Apache-2.0

//! PWM: the decoder from logic samples to `pwm.events`, which measures the cycles of a
//! pulse-width modulated line, and a waveform synthesizer.

mod decoder;
mod events;
pub mod synth;

pub use decoder::{Polarity, Pwm, PwmConfig, PwmConfigError};
pub use events::{PWM_EVENTS, PwmEvent};
