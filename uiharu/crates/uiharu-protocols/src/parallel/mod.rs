// SPDX-License-Identifier: Apache-2.0

//! Parallel bus: the decoder from logic samples to `parallel.events`, which reads up to 64
//! data lines as one word, and a waveform synthesizer.

mod decoder;
mod events;
pub mod synth;

pub use decoder::{Clock, ClockEdge, Parallel, ParallelConfig, ParallelConfigError};
pub use events::{PARALLEL_EVENTS, ParallelEvent};
