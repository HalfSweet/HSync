// SPDX-License-Identifier: Apache-2.0

//! USB 2.0 low and full speed: the decoder from logic samples to `usb.packets`, the reader
//! decoders built on USB packets use, the decoder from packets to `usb.transactions`, and a
//! waveform synthesizer.

mod bus;
mod decoder;
mod events;
pub mod synth;
mod transactions;

pub use bus::{BusEvent, Field, Next, Packet, Read, ReadError, UsbBus};
pub use decoder::{LineState, Speed, UsbConfig, UsbConfigError, UsbPackets};
pub use events::{DATA_BATCH, EndReason, PacketId, Pid, USB_PACKETS, UsbEvent};
pub use transactions::{
    Token, TransactionEvent, USB_TRANSACTIONS, UsbTransactions, XactEnd, XactId,
};
