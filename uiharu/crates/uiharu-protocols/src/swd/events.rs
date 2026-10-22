// SPDX-License-Identifier: Apache-2.0
//! Canonical SWD contract declared in the DSL; native transport helpers.
use uiharu_core::SchemaId;
pub use uiharu_dsl_protocols::swd::{Ack, EndReason, Interface, SwdEvent};
/// Schema of SWD transport streams.
pub const SWD_EVENTS: SchemaId = SchemaId::new("swd.events", 1);
/// Packet identity, serialized as an unsigned integer.
pub type TxId = u64;
/// SWD wire encoding operations for the generated acknowledge type.
pub trait AckExt: Sized {
    /// Decode the least-significant-first three-bit acknowledge.
    fn from_bits(bits: u8) -> Self;
    /// The three wire bits.
    fn bits(self) -> u8;
}
impl AckExt for Ack {
    /// The acknowledge whose three bits, read least significant bit first, make `bits`.
    fn from_bits(bits: u8) -> Ack {
        match bits & 0b111 {
            0b001 => Ack::Ok,
            0b010 => Ack::Wait,
            0b100 => Ack::Fault,
            other => Ack::Other(other),
        }
    }

    /// The acknowledge's three bits, least significant first on the wire.
    fn bits(self) -> u8 {
        match self {
            Ack::Ok => 0b001,
            Ack::Wait => 0b010,
            Ack::Fault => 0b100,
            Ack::Other(bits) => bits & 0b111,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn json(event: &SwdEvent) -> String {
        serde_json::to_string(event).expect("serializes")
    }

    #[test]
    fn serializes_as_the_contract_spells_it() {
        assert_eq!(
            json(&SwdEvent::Begin { tx: 7 }),
            r#"{"kind":"begin","tx":7}"#
        );
        assert_eq!(
            json(&SwdEvent::Request {
                tx: 7,
                apndp: true,
                rnw: false,
                addr: 0xc,
                parity_ok: true
            }),
            r#"{"kind":"request","tx":7,"apndp":true,"rnw":false,"addr":12,"parityOk":true}"#
        );
        assert_eq!(
            json(&SwdEvent::Ack {
                tx: 7,
                value: Ack::Wait
            }),
            r#"{"kind":"ack","tx":7,"value":"wait"}"#
        );
        assert_eq!(
            json(&SwdEvent::Ack {
                tx: 7,
                value: Ack::Other(7)
            }),
            r#"{"kind":"ack","tx":7,"value":{"other":7}}"#
        );
        assert_eq!(
            json(&SwdEvent::Data {
                tx: 7,
                value: 0x0001_0203,
                parity_ok: false,
                from_target: true
            }),
            r#"{"kind":"data","tx":7,"value":66051,"parityOk":false,"fromTarget":true}"#
        );
        assert_eq!(
            json(&SwdEvent::End {
                tx: 7,
                reason: EndReason::ProtocolError
            }),
            r#"{"kind":"end","tx":7,"reason":"protocol-error"}"#
        );
        assert_eq!(json(&SwdEvent::LineReset), r#"{"kind":"line-reset"}"#);
        assert_eq!(
            json(&SwdEvent::Switch {
                to: Interface::Jtag
            }),
            r#"{"kind":"switch","to":"jtag"}"#
        );
    }

    #[test]
    fn acknowledges_read_least_significant_bit_first() {
        assert_eq!(Ack::from_bits(0b001), Ack::Ok);
        assert_eq!(Ack::from_bits(0b010), Ack::Wait);
        assert_eq!(Ack::from_bits(0b100), Ack::Fault);
        assert_eq!(Ack::from_bits(0b111), Ack::Other(0b111));
        for bits in 0..8 {
            assert_eq!(Ack::from_bits(bits).bits(), bits);
        }
    }
}
