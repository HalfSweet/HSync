// SPDX-License-Identifier: Apache-2.0

//! The `apa102.events` contract: the frames of LED data a controller clocks into a chain of
//! APA102 or SK9822 LEDs, as the APA102 decoder emits them.
//!
//! A frame is framed like any bus transfer: a `frame-begin` carrying the frame's id, one `led`
//! per LED frame in the order the data travels down the chain, and a `frame-end` with the number
//! of LEDs, the bits that followed them and the reason. An LED's fields are those of its LED
//! frame, named and ordered as the datasheets name them; what the chain makes of them is the
//! reader's business.

use serde::{Deserialize, Serialize};
use uiharu_core::{EventSchema, SchemaId};

/// The schema of APA102 event streams.
pub const APA102_EVENTS: SchemaId = SchemaId::new("apa102.events", 1);

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
    /// The next frame's start frame came.
    NextFrame,
    /// Chip select was released: the transaction that carried the frame is over.
    Release,
    /// The samples ended or were lost, or the transaction was cut off, before the frame ended.
    Aborted,
}

/// What went wrong in a frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ErrorReason {
    /// The first byte after the start frame lacks the `111` bits that begin an LED frame.
    Header,
}

/// What travels on an APA102 event stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum Apa102Event {
    /// A start frame began a frame.
    FrameBegin {
        /// The frame that begins.
        frame: FrameId,
    },
    /// One LED frame.
    Led {
        /// The frame it belongs to.
        frame: FrameId,
        /// The LED's place in the chain, from 0 for the first LED the data reaches.
        index: u32,
        /// The five bits of global brightness, 0 to 31.
        brightness: u8,
        /// The blue byte.
        blue: u8,
        /// The green byte.
        green: u8,
        /// The red byte.
        red: u8,
    },
    /// The frame ended.
    FrameEnd {
        /// The frame that ends.
        frame: FrameId,
        /// The LED frames it carried.
        leds: u32,
        /// The bits after its last LED frame, up to the next start frame or the frame's end:
        /// the end frame, and the reset frame of an SK9822 chain.
        end_bits: u32,
        /// Why it ended.
        reason: EndReason,
    },
    /// Something in the frame is not as the protocol has it.
    Error {
        /// The frame it happened in.
        frame: FrameId,
        /// What it was.
        reason: ErrorReason,
    },
}

impl Apa102Event {
    /// The frame the event belongs to.
    pub fn frame(&self) -> FrameId {
        match self {
            Apa102Event::FrameBegin { frame }
            | Apa102Event::Led { frame, .. }
            | Apa102Event::FrameEnd { frame, .. }
            | Apa102Event::Error { frame, .. } => *frame,
        }
    }
}

impl EventSchema for Apa102Event {
    const SCHEMA: SchemaId = APA102_EVENTS;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_as_the_contract_spells_it() {
        let json = |event: &Apa102Event| serde_json::to_string(event).expect("serializes");
        assert_eq!(
            json(&Apa102Event::FrameBegin { frame: FrameId(1) }),
            r#"{"kind":"frame-begin","frame":1}"#
        );
        assert_eq!(
            json(&Apa102Event::Led {
                frame: FrameId(1),
                index: 3,
                brightness: 31,
                blue: 0x10,
                green: 0x20,
                red: 0xff,
            }),
            r#"{"kind":"led","frame":1,"index":3,"brightness":31,"blue":16,"green":32,"red":255}"#
        );
        let reasons = [
            (EndReason::NextFrame, "next-frame"),
            (EndReason::Release, "release"),
            (EndReason::Aborted, "aborted"),
        ];
        for (reason, name) in reasons {
            assert_eq!(
                json(&Apa102Event::FrameEnd {
                    frame: FrameId(2),
                    leds: 4,
                    end_bits: 8,
                    reason,
                }),
                format!(
                    r#"{{"kind":"frame-end","frame":2,"leds":4,"endBits":8,"reason":"{name}"}}"#
                )
            );
        }
        assert_eq!(
            json(&Apa102Event::Error {
                frame: FrameId(2),
                reason: ErrorReason::Header,
            }),
            r#"{"kind":"error","frame":2,"reason":"header"}"#
        );
        assert_eq!(
            Apa102Event::FrameEnd {
                frame: FrameId(6),
                leds: 0,
                end_bits: 0,
                reason: EndReason::Aborted
            }
            .frame(),
            FrameId(6)
        );
    }
}
