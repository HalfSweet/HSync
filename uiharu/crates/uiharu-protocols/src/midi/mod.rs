// SPDX-License-Identifier: Apache-2.0

//! MIDI: the messages of a MIDI 1.0 link, decoded from `uart.events` into `midi.events`, and
//! message builders for the UART synthesizer.
//!
//! MIDI sends its bytes as UART characters at 31.25 kbaud, a start bit, eight data bits and a
//! stop bit, so the decoder reads the stream of the [UART decoder](crate::uart): see [`Midi`]
//! for the rules.

mod decoder;
mod events;
pub mod synth;

pub use decoder::Midi;
pub use events::{
    ChannelStatus, CommonStatus, EndReason, ErrorReason, MIDI_EVENTS, MessageId, MidiEvent,
    RealtimeStatus, SYSEX_BATCH,
};

/// Helpers retained by the native physical MIDI producer.
pub use events::{ChannelStatusExt, CommonStatusExt, MidiEventExt, RealtimeStatusExt};
