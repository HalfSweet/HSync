// SPDX-License-Identifier: Apache-2.0

//! Wiegand: the decoder from the logic samples of an access-control reader's DATA0 and DATA1
//! lines to `wiegand.events`, and a waveform synthesizer.
//!
//! The interface is the one the SIA AC-01 access-control standard and reader datasheets
//! describe: a bit is a short low pulse on DATA0 (a 0) or DATA1 (a 1), and a frame ends with a
//! silence. The stream is unframed — one event per frame — so readers take the
//! [`WiegandEvent`]s straight from their inbox.

mod decoder;
mod events;
pub mod synth;

pub use decoder::{Wiegand, WiegandConfig, WiegandConfigError};
pub use events::{ErrorReason, FrameId, MAX_BITS, WIEGAND_EVENTS, WiegandEvent};
