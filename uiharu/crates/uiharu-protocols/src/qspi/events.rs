// SPDX-License-Identifier: Apache-2.0

use serde::{Deserialize, Serialize};
use uiharu_core::dsl::{Frame, Framed};
use uiharu_core::{EventSchema, SchemaId};

/// Fixed-lane SDR transport schema.
pub const QSPI_EVENTS: SchemaId = SchemaId::new("qspi.events", 1);
/// Why CS framing ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EndReason {
    /// CS deasserted.
    Release,
    /// Samples ended or a gap interrupted CS.
    Aborted,
}
/// A complete word in lane order, without interpreting direction or protocol.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Word {
    /// Assembled word, at most 32 bits.
    pub value: u32,
}
/// Transport events. Transaction zero is the unframed stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum QspiEvent {
    /// CS asserted; partial means its assertion was not observed.
    Begin { tx: u64, partial: bool },
    /// A complete configured word.
    Word { tx: u64, value: u32 },
    /// CS released or samples lost; incomplete bits never become a word.
    End {
        tx: u64,
        reason: EndReason,
        dropped_bits: u8,
    },
}
impl EventSchema for QspiEvent {
    const SCHEMA: SchemaId = QSPI_EVENTS;
}
impl Framed for QspiEvent {
    type Id = u64;
    type Item = Word;
    fn frame(&self) -> Frame<u64, Word> {
        match *self {
            Self::Begin { tx, partial } => Frame::Begin { id: tx, partial },
            Self::Word { tx, value } => Frame::Item {
                id: tx,
                item: Word { value },
            },
            Self::End { tx, reason, .. } => Frame::End {
                id: tx,
                normal: reason == EndReason::Release,
            },
        }
    }
    fn unframed() -> Option<u64> {
        Some(0)
    }
}
/// Cursor for consumers of this generic transport.
pub type QspiBus = uiharu_core::dsl::FramedBus<QspiEvent>;
