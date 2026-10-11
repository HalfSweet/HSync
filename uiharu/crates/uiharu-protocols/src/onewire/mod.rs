// SPDX-License-Identifier: Apache-2.0

//! 1-Wire: the link layer from logic samples to `onewire.link` (resets, presence pulses and
//! time slots), the network layer from `onewire.link` to `onewire.events` (transactions, ROM
//! commands, ROM codes, searches and transport bytes), the readers decoders built on either
//! use, the ROM commands and CRC-8, and a waveform synthesizer.

mod bus;
mod events;
mod link;
mod network;
pub mod rom;
pub mod synth;

pub use bus::{
    LinkBus, LinkError, LinkNext, Next, OneWireBus, Read, ReadError, Rom, Search, Transaction,
    TxItem,
};
pub use events::{EndReason, LinkEvent, ONEWIRE_EVENTS, ONEWIRE_LINK, OneWireEvent, TxId};
pub use link::{LinkConfig, LinkConfigError, OneWireLink, Speed};
pub use network::OneWireNetwork;
