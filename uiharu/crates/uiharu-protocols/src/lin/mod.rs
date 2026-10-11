// SPDX-License-Identifier: Apache-2.0

//! LIN: the frames of a Local Interconnect Network bus, decoded from `uart.events` into
//! `lin.events`, with the parity of the protected identifier, the two checksum models and a
//! frame builder for the UART synthesizer.
//!
//! A LIN frame is a header the master sends — a break field, the sync byte and the protected
//! identifier — and a response of data bytes and a checksum. Its bytes are UART characters of
//! eight data bits without parity, so the decoder reads the stream of the
//! [UART decoder](crate::uart), breaks included: see [`Lin`] for the rules.

mod decoder;
mod events;
mod fields;
pub mod synth;

pub use decoder::{Lin, LinConfig, LinConfigError};
pub use events::{DATA_BYTES, EndReason, ErrorReason, FrameId, LIN_EVENTS, LinEvent};
pub use fields::{
    SYNC, classic_checksum, data_length_by_id, enhanced_checksum, frame_id, pid_ok, protected_id,
};
