// SPDX-License-Identifier: Apache-2.0

//! I²C: the decoder from logic samples to `i2c.events`, the reader decoders built on I²C use,
//! and a waveform synthesizer.

mod bus;
mod decoder;
mod events;
pub mod synth;

pub use bus::{Address, I2cBus, Next, Read, ReadError, Transaction, TxEvent};
pub use decoder::{I2c, I2cConfig, I2cConfigError};
pub use events::{EndReason, I2C_EVENTS, I2cEvent, TxId};
