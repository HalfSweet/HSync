// SPDX-License-Identifier: Apache-2.0

//! APA102: the frames a controller clocks into a chain of APA102 or SK9822 LEDs, decoded from
//! `spi.events` into `apa102.events`, and a frame builder for the SPI synthesizer.
//!
//! The parts take a clock and a data line, SPI without chip select, so the decoder,
//! [`Apa102`], is stacked on the SPI decoder, with or without a chip select line. Its rules are
//! listed in the decoder module's documentation and on the protocol's page,
//! `docs/protocols/apa102.md`.

mod decoder;
mod events;
pub mod synth;

pub use decoder::Apa102;
pub use events::{APA102_EVENTS, Apa102Event, EndReason, ErrorReason, FrameId};
