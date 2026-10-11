// SPDX-License-Identifier: Apache-2.0

//! SPI: the decoder from logic samples to `spi.events`, the reader decoders built on SPI use,
//! and a waveform synthesizer.

mod bus;
mod decoder;
mod events;
pub mod synth;

pub use bus::{Line, Next, Read, ReadError, SpiBus, Transaction, Word};
pub use decoder::{BitOrder, Spi, SpiConfig, SpiConfigError};
pub use events::{EndReason, SPI_EVENTS, SpiEvent, TxId};
