// SPDX-License-Identifier: Apache-2.0

//! Arm Serial Wire Debug: the decoder from logic samples to `swd.events` — packets with their
//! request, acknowledge and data, line resets and the select sequences of a Serial Wire/JTAG
//! debug port — the reader decoders built on SWD use, and a waveform synthesizer.

mod bus;
mod decoder;
mod events;
pub mod synth;

pub use bus::{Data, Field, Next, Packet, Read, ReadError, Request, SwdBus};
pub use decoder::{
    ACTIVATE_JTAG_DP, ACTIVATE_SWD, JTAG_SERIAL_BITS, JTAG_TO_SWD, LINE_RESET_CLOCKS,
    SELECTION_ALERT, SWD_TO_DS, SWD_TO_JTAG, Swd, SwdConfig, SwdConfigError,
};
pub use events::{Ack, EndReason, Interface, SWD_EVENTS, SwdEvent, TxId};
