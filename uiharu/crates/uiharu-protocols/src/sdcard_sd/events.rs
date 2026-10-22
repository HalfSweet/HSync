// SPDX-License-Identifier: Apache-2.0

use serde::{Deserialize, Serialize};
use uiharu_core::{EventSchema, SchemaId};

/// Native SD event schema.
pub const SDCARD_SD_EVENTS: SchemaId = SchemaId::new("sdcard-sd.events", 1);

/// How a command token or read block ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EndReason {
    /// All bits including a high end bit arrived (CRC result is reported separately).
    Complete,
    /// The end bit was low.
    InvalidEnd,
    /// A frame of unknown alignment was drained to eight high clock samples.
    Resynchronized,
    /// Missing samples or end of input interrupted the frame.
    Aborted,
}

/// Streaming transport events. Token and block identifiers share one sequence.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum SdcardSdEvent {
    /// A CMD token's start bit (or a low CMD at capture start).
    Begin { token: u64, partial: bool },
    /// The token's first eight bits. `index` preserves reserved response bits too.
    Header {
        token: u64,
        host: bool,
        index: u8,
        bits: u16,
    },
    /// Up to 32 argument/register bits, MSB first, before the trailing CRC7.
    Payload {
        token: u64,
        offset: u16,
        value: u32,
        bits: u8,
    },
    /// Received CRC7 and its check; R3 has no CRC, so `ok` is null.
    Crc {
        token: u64,
        value: u8,
        expected: Option<u8>,
        ok: Option<bool>,
    },
    /// The CMD token closed.
    End { token: u64, reason: EndReason },
    /// The read block's start bits, on all connected DAT lanes.
    DataBegin { block: u64, partial: bool },
    /// One byte, reported without waiting for the rest of the block.
    Data { block: u64, offset: u32, value: u8 },
    /// One lane's CRC16, after all sixteen CRC clocks.
    DataCrc {
        block: u64,
        lane: u8,
        value: u16,
        expected: u16,
        ok: bool,
    },
    /// The read block closed.
    DataEnd { block: u64, reason: EndReason },
}
impl EventSchema for SdcardSdEvent {
    const SCHEMA: SchemaId = SDCARD_SD_EVENTS;
}
