// SPDX-License-Identifier: Apache-2.0

//! The `wiegand.events` contract: the frames an access-control reader sent over its DATA0 and
//! DATA1 lines, and the frames that could not be read.
//!
//! The stream is unframed: a frame is one event, decided once the silence after its last pulse
//! has passed, so there is no `begin` or `end`. A frame that could not be read to its end is an
//! `error` instead.

use serde::{Deserialize, Serialize};
use uiharu_core::{EventSchema, SchemaId};

/// The schema of Wiegand event streams.
pub const WIEGAND_EVENTS: SchemaId = SchemaId::new("wiegand.events", 1);

/// The most bits a frame carries; a longer one is an error.
pub const MAX_BITS: u16 = 64;

/// A frame, numbered from 1 by the decoder in the order frames are reported.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct FrameId(pub u64);

/// Why a frame could not be read.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ErrorReason {
    /// Both lines were low at once.
    BothLines,
    /// A pulse was shorter than the narrowest the configuration allows.
    ShortPulse,
    /// A pulse was longer than the widest the configuration allows.
    LongPulse,
    /// The frame went on past [`MAX_BITS`] bits.
    TooLong,
    /// A gap or the end of the samples cut the frame off before its silence had passed.
    Aborted,
}

/// What travels on a Wiegand event stream.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum WiegandEvent {
    /// A frame: the bits of the pulses up to a silence.
    Frame {
        /// The frame.
        frame: FrameId,
        /// How many bits it carries, 1 to [`MAX_BITS`].
        bits: u16,
        /// The bits packed most significant bit first: the first bit on the lines is the top
        /// bit of the first byte, and the last byte is padded with zeros; ⌈`bits` / 8⌉ bytes.
        data: Vec<u8>,
        /// For a 26-bit frame, whether both its parity bits hold: the first bit makes the first
        /// 13 bits even, the last bit makes the last 13 odd. `None` for other lengths, whose
        /// formats differ.
        parity_ok: Option<bool>,
    },
    /// A frame that could not be read; the pulses after it up to the next silence are skipped.
    Error {
        /// What went wrong.
        reason: ErrorReason,
    },
}

impl EventSchema for WiegandEvent {
    const SCHEMA: SchemaId = WIEGAND_EVENTS;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn json(event: &WiegandEvent) -> String {
        serde_json::to_string(event).expect("serializes")
    }

    #[test]
    fn serializes_as_the_contract_spells_it() {
        assert_eq!(
            json(&WiegandEvent::Frame {
                frame: FrameId(2),
                bits: 26,
                data: vec![0x3d, 0x98, 0x1c, 0xc0],
                parity_ok: Some(true),
            }),
            r#"{"kind":"frame","frame":2,"bits":26,"data":[61,152,28,192],"parityOk":true}"#
        );
        assert_eq!(
            json(&WiegandEvent::Frame {
                frame: FrameId(3),
                bits: 4,
                data: vec![0xa0],
                parity_ok: None,
            }),
            r#"{"kind":"frame","frame":3,"bits":4,"data":[160],"parityOk":null}"#
        );
        for (reason, name) in [
            (ErrorReason::BothLines, "both-lines"),
            (ErrorReason::ShortPulse, "short-pulse"),
            (ErrorReason::LongPulse, "long-pulse"),
            (ErrorReason::TooLong, "too-long"),
            (ErrorReason::Aborted, "aborted"),
        ] {
            assert_eq!(
                json(&WiegandEvent::Error { reason }),
                format!(r#"{{"kind":"error","reason":"{name}"}}"#)
            );
        }
    }

    #[test]
    fn reads_back_what_it_writes() {
        let event = WiegandEvent::Frame {
            frame: FrameId(1),
            bits: 34,
            data: vec![1, 2, 3, 4, 0x80],
            parity_ok: None,
        };
        let back: WiegandEvent = serde_json::from_str(&json(&event)).expect("parses");
        assert_eq!(back, event);
    }
}
