// SPDX-License-Identifier: Apache-2.0

//! Arm Serial Wire Debug: the decoder from logic samples to `swd.events` — packets with their
//! request, acknowledge and data, line resets and the select sequences of a Serial Wire/JTAG
//! debug port — and a waveform synthesizer.

mod decoder;
mod events;
pub mod synth;

pub use decoder::{JTAG_TO_SWD, LINE_RESET_CLOCKS, SWD_TO_JTAG, Swd, SwdConfig, SwdConfigError};
pub use events::{Ack, EndReason, Interface, SWD_EVENTS, SwdEvent, TxId};
