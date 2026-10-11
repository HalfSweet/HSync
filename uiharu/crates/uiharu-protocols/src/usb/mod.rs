// SPDX-License-Identifier: Apache-2.0

//! USB 2.0 low and full speed: the decoder from logic samples to `usb.packets` and a waveform
//! synthesizer.

mod decoder;
mod events;
pub mod synth;

pub use decoder::{LineState, Speed, UsbConfig, UsbConfigError, UsbPackets};
pub use events::{DATA_BATCH, EndReason, PacketId, Pid, USB_PACKETS, UsbEvent};
