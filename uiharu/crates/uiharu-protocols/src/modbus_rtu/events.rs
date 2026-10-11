// SPDX-License-Identifier: Apache-2.0

//! The `modbus.events` contract: the frames of a Modbus serial line, as a Modbus decoder emits
//! them and decoders of the application protocol read them.
//!
//! A frame is framed like any bus transfer: a `begin` with the frame's id, its `data` and `crc`
//! carrying that id, and an `end` with the reason. The PDU is not interpreted: the function code
//! and the data are the bytes on the line.

use serde::{Deserialize, Serialize};
use uiharu_core::{EventSchema, SchemaId};

/// The schema of Modbus event streams.
pub const MODBUS_EVENTS: SchemaId = SchemaId::new("modbus.events", 1);

/// The most bytes one `data` event carries.
pub const DATA_BATCH: usize = 32;

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
    /// The silence that ends a frame followed it, and its CRC matched.
    Complete,
    /// The silence followed the frame, but its CRC did not match the bytes before it.
    BadCrc,
    /// The frame had fewer than four bytes: no room for an address, a function code and a CRC.
    Short,
    /// A character came after an inter-character silence that a frame must not have, or a break
    /// interrupted the frame: a receiver declares it incomplete and discards it.
    Broken,
    /// The samples ended or were lost before the silence that ends the frame was seen.
    Aborted,
}

/// What travels on a Modbus event stream.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum ModbusEvent {
    /// A frame began: its first two bytes.
    Begin {
        /// The frame that begins.
        frame: FrameId,
        /// The first byte: the server address, 0 for a broadcast.
        address: u8,
        /// The second byte, the function code; `None` for a frame of a single byte.
        function: Option<u8>,
        /// Whether the function code has its top bit set: an exception response, whose one data
        /// byte is the exception code.
        exception: bool,
        /// Whether earlier bytes of the frame may have gone unseen: its first byte came less than
        /// the inter-frame silence after the samples began or resumed after a gap.
        partial: bool,
    },
    /// Bytes of the frame between the function code and the CRC, in order.
    Data {
        /// The frame they belong to.
        frame: FrameId,
        /// The position of the first byte, counted from the first byte after the function code.
        offset: u32,
        /// At most [`DATA_BATCH`] bytes.
        bytes: Vec<u8>,
    },
    /// The frame's CRC: its last two bytes.
    Crc {
        /// The frame it closes.
        frame: FrameId,
        /// The CRC as sent, low byte first on the line.
        value: u16,
        /// Whether it is the CRC of the frame's other bytes.
        ok: bool,
    },
    /// The frame ended.
    End {
        /// The frame that ends.
        frame: FrameId,
        /// Why it ended.
        reason: EndReason,
    },
}

impl ModbusEvent {
    /// The frame the event belongs to.
    pub fn frame(&self) -> FrameId {
        match self {
            ModbusEvent::Begin { frame, .. }
            | ModbusEvent::Data { frame, .. }
            | ModbusEvent::Crc { frame, .. }
            | ModbusEvent::End { frame, .. } => *frame,
        }
    }
}

impl EventSchema for ModbusEvent {
    const SCHEMA: SchemaId = MODBUS_EVENTS;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_as_the_contract_spells_it() {
        let json = |event: &ModbusEvent| serde_json::to_string(event).expect("serializes");
        assert_eq!(
            json(&ModbusEvent::Begin {
                frame: FrameId(2),
                address: 17,
                function: Some(0x83),
                exception: true,
                partial: false,
            }),
            r#"{"kind":"begin","frame":2,"address":17,"function":131,"exception":true,"partial":false}"#
        );
        assert_eq!(
            json(&ModbusEvent::Begin {
                frame: FrameId(3),
                address: 5,
                function: None,
                exception: false,
                partial: true,
            }),
            r#"{"kind":"begin","frame":3,"address":5,"function":null,"exception":false,"partial":true}"#
        );
        assert_eq!(
            json(&ModbusEvent::Data {
                frame: FrameId(2),
                offset: 32,
                bytes: vec![1, 2],
            }),
            r#"{"kind":"data","frame":2,"offset":32,"bytes":[1,2]}"#
        );
        assert_eq!(
            json(&ModbusEvent::Crc {
                frame: FrameId(2),
                value: 0x8776,
                ok: true,
            }),
            r#"{"kind":"crc","frame":2,"value":34678,"ok":true}"#
        );
        let reasons = [
            (EndReason::Complete, "complete"),
            (EndReason::BadCrc, "bad-crc"),
            (EndReason::Short, "short"),
            (EndReason::Broken, "broken"),
            (EndReason::Aborted, "aborted"),
        ];
        for (reason, name) in reasons {
            assert_eq!(
                json(&ModbusEvent::End {
                    frame: FrameId(2),
                    reason,
                }),
                format!(r#"{{"kind":"end","frame":2,"reason":"{name}"}}"#)
            );
        }
        assert_eq!(
            ModbusEvent::End {
                frame: FrameId(9),
                reason: EndReason::Short
            }
            .frame(),
            FrameId(9)
        );
    }
}
