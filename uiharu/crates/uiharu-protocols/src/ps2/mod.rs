// SPDX-License-Identifier: Apache-2.0

//! PS/2: the decoder from the logic samples of a keyboard or mouse port's clock and data lines
//! to `ps2.events`, and a waveform synthesizer.
//!
//! The interface is IBM's PS/2 keyboard and auxiliary device interface: eleven-bit frames
//! clocked by the device in both directions, the host requesting to send by inhibiting the
//! bus. The stream is unframed — one event per frame — so readers take the [`Ps2Event`]s
//! straight from their inbox.

mod decoder;
mod events;
pub mod synth;

pub use decoder::{
    FIRST_CLOCK_US, IDLE_US, INHIBIT_US, MIN_RATE_HZ, PAUSE_US, Ps2, Ps2Config, Ps2ConfigError,
};
pub use events::{Direction, ErrorReason, PS2_EVENTS, Ps2Event};
