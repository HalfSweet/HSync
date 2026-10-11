// SPDX-License-Identifier: Apache-2.0

//! USB Power Delivery on the CC line: the decoder from logic samples of the line's BMC signal
//! to `usb_pd.events`, and a waveform synthesizer.

mod crc;
mod decoder;
mod events;
mod symbols;
pub mod synth;

pub use decoder::{MAX_TOLERANCE, MIN_TICKS_PER_BIT, UsbPd, UsbPdConfig, UsbPdConfigError};
pub use events::{
    ControlMessage, DATA_BATCH, DataMessage, DataRole, EndReason, ErrorReason, ExtendedMessage,
    ExtendedMessageHeader, MessageHeader, OrderedSet, PacketId, PowerObject, PowerRole,
    RequestObject, USB_PD_EVENTS, UsbPdEvent,
};
pub use symbols::{CABLE_RESET, HARD_RESET, KCode};
