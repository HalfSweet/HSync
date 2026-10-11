// SPDX-License-Identifier: Apache-2.0

//! 1-Wire: the link layer from logic samples to `onewire.link` (resets, presence pulses and
//! time slots), the ROM commands and CRC-8 of the standard, and a waveform synthesizer.

mod events;
mod link;
pub mod rom;
pub mod synth;

pub use events::{LinkEvent, ONEWIRE_LINK};
pub use link::{LinkConfig, LinkConfigError, OneWireLink, Speed};
