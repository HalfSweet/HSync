// SPDX-License-Identifier: Apache-2.0

//! AM230x: the decoder from the logic samples of an AM2302/DHT22 or DHT11 sensor's data line to
//! `am230x.events`, and a waveform synthesizer.
//!
//! The sensors answer a host's start signal on one open-drain line with 40 bits told apart by
//! the length of their highs: humidity, temperature and a checksum. The stream is unframed —
//! one event per reading — so readers take the [`Am230xEvent`]s straight from their inbox.

mod decoder;
mod events;
pub mod synth;

pub use decoder::{Am230x, Am230xConfig, Am230xConfigError, MAX_TOLERANCE, MIN_RATE_HZ, Model};
pub use events::{AM230X_EVENTS, Am230xEvent, ErrorReason};
