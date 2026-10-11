// SPDX-License-Identifier: Apache-2.0

//! DMX512: the packets of a DMX512-A line, decoded from `uart.events` into `dmx512.events`, and
//! a packet builder for the UART synthesizer.
//!
//! A DMX512 line is an asynchronous serial line at 250 kbit/s whose packets are framed by
//! breaks, so the decoder, [`Dmx512`], is stacked on the UART decoder configured as
//! [`Dmx512::uart_config`] says. Start codes and slot values are reported as they are, not
//! interpreted; the rules are listed in the decoder module's documentation and on the
//! protocol's page, `docs/protocols/dmx512.md`.

mod decoder;
mod events;
pub mod synth;

pub use decoder::{BAUD, Dmx512};
pub use events::{DMX512_EVENTS, Dmx512Event, EndReason, PacketId, SLOT_BATCH};
