// SPDX-License-Identifier: Apache-2.0

//! 24xx-series I²C EEPROMs: the decoder from `i2c.events` to `eeprom24xx.events`, and the I²C
//! traffic of their commands for the waveform synthesizer.

mod decoder;
mod events;
pub mod synth;

pub use decoder::{Eeprom24xx, Eeprom24xxConfig, Eeprom24xxConfigError};
pub use events::{Command, DATA_BATCH, EEPROM24XX_EVENTS, EepromEnd, EepromEvent, OpId};
