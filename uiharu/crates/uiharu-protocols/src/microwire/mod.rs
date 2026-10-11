// SPDX-License-Identifier: Apache-2.0

//! Microwire: the decoder from logic samples to `microwire.events`, the reader decoders built
//! on Microwire use, and a waveform synthesizer.
//!
//! Microwire is the synchronous serial interface of the 93Cxx EEPROMs and other devices: chip
//! select, a clock (SK), data into the device (DI) and data out of it (DO). The stream reports
//! every clock pulse as a bit, framed by chip select; decoders of devices read it through
//! [`MicrowireBus`] and [`Transaction`].

mod bus;
mod decoder;
mod events;
pub mod synth;

pub use bus::{Bit, MicrowireBus, Next, Read, ReadError, Transaction};
pub use decoder::{Microwire, MicrowireConfig, MicrowireConfigError};
pub use events::{EndReason, MICROWIRE_EVENTS, MicrowireEvent, TxId};
