// SPDX-License-Identifier: Apache-2.0

//! The `ws281x.events` contract: the frames of LED data on the data line of a WS281x chain, as
//! the WS281x decoder emits them.
//!
//! A frame is the data a controller sends between two resets: a `frame-begin` carrying the
//! frame's id, one `led` per LED in the order the data travels down the chain, and a
//! `frame-end` with the number of LEDs and the reason. An LED's bytes are reported as they are on
//! the wire: which byte drives which colour depends on the part, and is the reader's business.

use serde::{Deserialize, Serialize};
use uiharu_core::{EventSchema, SchemaId};

/// The schema of WS281x event streams.
pub const WS281X_EVENTS: SchemaId = SchemaId::new("ws281x.events", 1);

/// A frame, numbered from 1 by the decoder.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct FrameId(pub u64);

/// Why a frame ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EndReason {
    /// A low of the reset time followed the frame's last bit: the chain latches the data.
    Reset,
    /// The samples ended or were lost before the reset was seen.
    Aborted,
}

/// What went wrong in a frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ErrorReason {
    /// The reset came after fewer bits than an LED takes.
    PartialLed,
}

/// What travels on a WS281x event stream.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum Ws281xEvent {
    /// A frame began: its first bit rose after a reset.
    FrameBegin {
        /// The frame that begins.
        frame: FrameId,
    },
    /// The bits of one LED.
    Led {
        /// The frame the LED's data belongs to.
        frame: FrameId,
        /// The LED's place in the chain, from 0 for the first LED the data reaches.
        index: u32,
        /// The LED's three or four bytes in wire order, the first bit received being the most
        /// significant bit of the first byte: green, red and blue on WS2812B-class parts, white
        /// fourth on RGBW parts.
        bytes: Vec<u8>,
    },
    /// The frame ended.
    FrameEnd {
        /// The frame that ends.
        frame: FrameId,
        /// The whole LEDs it carried.
        leds: u32,
        /// Why it ended.
        reason: EndReason,
    },
    /// Something in the frame is not as the protocol has it.
    Error {
        /// The frame it happened in.
        frame: FrameId,
        /// What it was.
        reason: ErrorReason,
        /// For `partial-led`, the bits left over, fewer than an LED takes.
        bits: u8,
    },
}

impl Ws281xEvent {
    /// The frame the event belongs to.
    pub fn frame(&self) -> FrameId {
        match self {
            Ws281xEvent::FrameBegin { frame }
            | Ws281xEvent::Led { frame, .. }
            | Ws281xEvent::FrameEnd { frame, .. }
            | Ws281xEvent::Error { frame, .. } => *frame,
        }
    }
}

impl EventSchema for Ws281xEvent {
    const SCHEMA: SchemaId = WS281X_EVENTS;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_as_the_contract_spells_it() {
        let json = |event: &Ws281xEvent| serde_json::to_string(event).expect("serializes");
        assert_eq!(
            json(&Ws281xEvent::FrameBegin { frame: FrameId(1) }),
            r#"{"kind":"frame-begin","frame":1}"#
        );
        assert_eq!(
            json(&Ws281xEvent::Led {
                frame: FrameId(1),
                index: 2,
                bytes: vec![0xff, 0x00, 0x10],
            }),
            r#"{"kind":"led","frame":1,"index":2,"bytes":[255,0,16]}"#
        );
        assert_eq!(
            json(&Ws281xEvent::FrameEnd {
                frame: FrameId(1),
                leds: 3,
                reason: EndReason::Reset,
            }),
            r#"{"kind":"frame-end","frame":1,"leds":3,"reason":"reset"}"#
        );
        assert_eq!(
            json(&Ws281xEvent::FrameEnd {
                frame: FrameId(4),
                leds: 0,
                reason: EndReason::Aborted,
            }),
            r#"{"kind":"frame-end","frame":4,"leds":0,"reason":"aborted"}"#
        );
        assert_eq!(
            json(&Ws281xEvent::Error {
                frame: FrameId(1),
                reason: ErrorReason::PartialLed,
                bits: 5,
            }),
            r#"{"kind":"error","frame":1,"reason":"partial-led","bits":5}"#
        );
        assert_eq!(
            Ws281xEvent::Error {
                frame: FrameId(7),
                reason: ErrorReason::PartialLed,
                bits: 1
            }
            .frame(),
            FrameId(7)
        );
    }
}
