// SPDX-License-Identifier: Apache-2.0

//! The `i2s.events` contract: the audio samples on an I²S bus, one word per channel and WS
//! period.

use serde::{Deserialize, Serialize};
use uiharu_core::{EventSchema, SchemaId};

/// The schema of I²S event streams.
pub const I2S_EVENTS: SchemaId = SchemaId::new("i2s.events", 1);

/// An audio channel, as the word select line tells it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Channel {
    /// The left channel, the first of a frame: WS low, unless WS is inverted.
    Left,
    /// The right channel: WS high, unless WS is inverted.
    Right,
}

/// What travels on an I²S event stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum I2sEvent {
    /// One word: an audio sample of one channel.
    Sample {
        /// The WS period the word belongs to, counted from 1: a left half-period and the
        /// right one after it make a frame.
        frame: u64,
        /// The channel.
        channel: Channel,
        /// The bits received, the first one most significant: an unsigned integer of `bits`
        /// bits. Audio samples are two's complement; extending the sign is the reader's
        /// business.
        value: u32,
        /// The bits in `value`, 1 to 32.
        bits: u8,
        /// Whether the word has its full width: `false` when its half-period ended before the
        /// configured number of bits arrived.
        complete: bool,
    },
}

impl EventSchema for I2sEvent {
    const SCHEMA: SchemaId = I2S_EVENTS;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_as_the_contract_spells_it() {
        let json = serde_json::to_string(&I2sEvent::Sample {
            frame: 7,
            channel: Channel::Left,
            value: 0x00ff_8001,
            bits: 24,
            complete: true,
        })
        .expect("serializes");
        assert_eq!(
            json,
            r#"{"kind":"sample","frame":7,"channel":"left","value":16744449,"bits":24,"complete":true}"#
        );
        let json = serde_json::to_string(&I2sEvent::Sample {
            frame: 8,
            channel: Channel::Right,
            value: 0x1234,
            bits: 16,
            complete: false,
        })
        .expect("serializes");
        assert_eq!(
            json,
            r#"{"kind":"sample","frame":8,"channel":"right","value":4660,"bits":16,"complete":false}"#
        );
    }
}
