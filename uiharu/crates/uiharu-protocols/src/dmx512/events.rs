// SPDX-License-Identifier: Apache-2.0
//! The exact DMX transport contract is generated from `dsl/contracts/dmx512.ts`.
use uiharu_core::SchemaId;
pub use uiharu_dsl_protocols::dmx512::{Dmx512Event, EndReason};
/// The DMX event schema.
pub const DMX512_EVENTS: SchemaId = SchemaId::new("dmx512.events", 1);
/// The maximum slot batch size.
pub const SLOT_BATCH: usize = 32;
/// A packet number, starting at one.
pub type PacketId = u64;
/// Helpers retained for the generated wire contract.
pub trait Dmx512EventExt {
    /// The packet the event belongs to.
    fn packet(&self) -> PacketId;
}
impl Dmx512EventExt for Dmx512Event {
    /// The packet the event belongs to.
    fn packet(&self) -> PacketId {
        match self {
            Dmx512Event::PacketBegin { packet, .. }
            | Dmx512Event::StartCode { packet, .. }
            | Dmx512Event::Slots { packet, .. }
            | Dmx512Event::PacketEnd { packet, .. }
            | Dmx512Event::Error { packet, .. } => *packet,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_as_the_contract_spells_it() {
        let json = |event: &Dmx512Event| serde_json::to_string(event).expect("serializes");
        assert_eq!(
            json(&Dmx512Event::PacketBegin {
                packet: 1,
                break_ticks: 176,
            }),
            r#"{"kind":"packet-begin","packet":1,"breakTicks":176}"#
        );
        assert_eq!(
            json(&Dmx512Event::StartCode { packet: 1, code: 0 }),
            r#"{"kind":"start-code","packet":1,"code":0}"#
        );
        assert_eq!(
            json(&Dmx512Event::Slots {
                packet: 1,
                offset: 32,
                values: vec![255, 0, 7],
            }),
            r#"{"kind":"slots","packet":1,"offset":32,"values":[255,0,7]}"#
        );
        assert_eq!(
            json(&Dmx512Event::PacketEnd {
                packet: 1,
                slots: 512,
                reason: EndReason::Break,
            }),
            r#"{"kind":"packet-end","packet":1,"slots":512,"reason":"break"}"#
        );
        assert_eq!(
            json(&Dmx512Event::PacketEnd {
                packet: 2,
                slots: 3,
                reason: EndReason::Aborted,
            }),
            r#"{"kind":"packet-end","packet":2,"slots":3,"reason":"aborted"}"#
        );
        assert_eq!(
            json(&Dmx512Event::Error { packet: 2, slot: 0 }),
            r#"{"kind":"error","packet":2,"slot":0}"#
        );
        assert_eq!(Dmx512Event::Error { packet: 5, slot: 9 }.packet(), 5);
    }
}
