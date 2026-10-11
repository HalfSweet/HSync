// SPDX-License-Identifier: Apache-2.0

//! CAN: the decoder from logic samples to `can.events`, for classic and FD frames, and a
//! waveform synthesizer.

mod crc;
mod decoder;
mod events;
pub mod synth;
mod timing;

pub use decoder::{Can, CanConfig, CanConfigError, MIN_TICKS_PER_BIT};
pub use events::{CAN_EVENTS, CanEvent, DATA_BATCH, EndReason, FrameId};
