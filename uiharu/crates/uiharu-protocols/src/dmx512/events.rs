// SPDX-License-Identifier: Apache-2.0

//! The `dmx512.events` contract: the packets of a DMX512 line, as the DMX512 decoder emits them.
//!
//! A packet is framed like any bus transfer: a `packet-begin` carrying the packet's id and the
//! length of the break that began it, the `start-code` and the data `slots` carrying that id,
//! and a `packet-end` with the number of data slots and the reason. Slots are numbered as
//! ANSI E1.11 numbers them: slot 0 is the start code, slots 1 to 512 carry the data. Values are
//! reported as the line carried them; what they mean is up to the start code and the reader.

use serde::{Deserialize, Serialize};
use uiharu_core::{EventSchema, SchemaId};

/// The schema of DMX512 event streams.
pub const DMX512_EVENTS: SchemaId = SchemaId::new("dmx512.events", 1);

/// The most values one `slots` event carries.
pub const SLOT_BATCH: usize = 32;

/// A packet, numbered from 1 by the decoder.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct PacketId(pub u64);

/// Why a packet ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EndReason {
    /// The next break began: the packet is whole.
    Break,
    /// The samples ended or were lost before the next break.
    Aborted,
}

/// What travels on a DMX512 event stream.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum Dmx512Event {
    /// A break began a packet.
    PacketBegin {
        /// The packet that begins.
        packet: PacketId,
        /// How long the line was held at space, in ticks: E1.11 asks transmitters for at least
        /// 92 µs and receivers to accept 88 µs.
        break_ticks: u64,
    },
    /// The packet's first slot after the break: what its data is. 0 for dimmer data.
    StartCode {
        /// The packet it belongs to.
        packet: PacketId,
        /// The start code.
        code: u8,
    },
    /// Data slots, in order.
    Slots {
        /// The packet they belong to.
        packet: PacketId,
        /// How many data slots of the packet came before `values[0]`: it is slot `offset + 1`.
        offset: u32,
        /// At most [`SLOT_BATCH`] values.
        values: Vec<u8>,
    },
    /// The packet ended.
    PacketEnd {
        /// The packet that ends.
        packet: PacketId,
        /// The data slots it carried, the start code not counted; E1.11 allows up to 512.
        slots: u32,
        /// Why it ended.
        reason: EndReason,
    },
    /// A slot's character had a framing error: a stop bit read space. Its value is reported
    /// all the same, as the line carried it.
    Error {
        /// The packet it belongs to.
        packet: PacketId,
        /// The slot, numbered as E1.11 numbers them: 0 for the start code.
        slot: u32,
    },
}

impl Dmx512Event {
    /// The packet the event belongs to.
    pub fn packet(&self) -> PacketId {
        match self {
            Dmx512Event::PacketBegin { packet, .. }
            | Dmx512Event::StartCode { packet, .. }
            | Dmx512Event::Slots { packet, .. }
            | Dmx512Event::PacketEnd { packet, .. }
            | Dmx512Event::Error { packet, .. } => *packet,
        }
    }
}

impl EventSchema for Dmx512Event {
    const SCHEMA: SchemaId = DMX512_EVENTS;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_as_the_contract_spells_it() {
        let json = |event: &Dmx512Event| serde_json::to_string(event).expect("serializes");
        assert_eq!(
            json(&Dmx512Event::PacketBegin {
                packet: PacketId(1),
                break_ticks: 176,
            }),
            r#"{"kind":"packet-begin","packet":1,"breakTicks":176}"#
        );
        assert_eq!(
            json(&Dmx512Event::StartCode {
                packet: PacketId(1),
                code: 0,
            }),
            r#"{"kind":"start-code","packet":1,"code":0}"#
        );
        assert_eq!(
            json(&Dmx512Event::Slots {
                packet: PacketId(1),
                offset: 32,
                values: vec![255, 0, 7],
            }),
            r#"{"kind":"slots","packet":1,"offset":32,"values":[255,0,7]}"#
        );
        assert_eq!(
            json(&Dmx512Event::PacketEnd {
                packet: PacketId(1),
                slots: 512,
                reason: EndReason::Break,
            }),
            r#"{"kind":"packet-end","packet":1,"slots":512,"reason":"break"}"#
        );
        assert_eq!(
            json(&Dmx512Event::PacketEnd {
                packet: PacketId(2),
                slots: 3,
                reason: EndReason::Aborted,
            }),
            r#"{"kind":"packet-end","packet":2,"slots":3,"reason":"aborted"}"#
        );
        assert_eq!(
            json(&Dmx512Event::Error {
                packet: PacketId(2),
                slot: 0,
            }),
            r#"{"kind":"error","packet":2,"slot":0}"#
        );
        assert_eq!(
            Dmx512Event::Error {
                packet: PacketId(5),
                slot: 9
            }
            .packet(),
            PacketId(5)
        );
    }
}
