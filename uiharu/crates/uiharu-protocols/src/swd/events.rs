// SPDX-License-Identifier: Apache-2.0

//! The `swd.events` contract: what an SWD decoder emits and what decoders built on SWD read.

use serde::{Deserialize, Serialize};
use uiharu_core::{EventSchema, SchemaId};

/// The schema of SWD event streams.
pub const SWD_EVENTS: SchemaId = SchemaId::new("swd.events", 1);

/// A packet, numbered from 1 by the decoder.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct TxId(pub u64);

/// The acknowledge a target answers a request with.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Ack {
    /// OK, `0b001`: the transfer goes ahead.
    Ok,
    /// WAIT, `0b010`: the target cannot take the transfer now; the host retries.
    Wait,
    /// FAULT, `0b100`: the target refuses the transfer until an error flag is cleared.
    Fault,
    /// Any other value, as read least significant bit first: no target drove the line, or the
    /// bits were not where the decoder looked for them.
    Other(u8),
}

impl Ack {
    /// The acknowledge whose three bits, read least significant bit first, make `bits`.
    pub const fn from_bits(bits: u8) -> Ack {
        match bits & 0b111 {
            0b001 => Ack::Ok,
            0b010 => Ack::Wait,
            0b100 => Ack::Fault,
            other => Ack::Other(other),
        }
    }

    /// The acknowledge's three bits, least significant first on the wire.
    pub const fn bits(self) -> u8 {
        match self {
            Ack::Ok => 0b001,
            Ack::Wait => 0b010,
            Ack::Fault => 0b100,
            Ack::Other(bits) => bits & 0b111,
        }
    }
}

/// Why a packet ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EndReason {
    /// The target acknowledged OK and the data phase was whole.
    Complete,
    /// The target acknowledged WAIT; no data followed.
    Wait,
    /// The target acknowledged FAULT; no data followed.
    Fault,
    /// The packet broke the protocol where it ended: a request with the wrong parity, a Stop
    /// bit that is not 0 or a Park bit that is not 1, an acknowledge other than OK, WAIT and
    /// FAULT, or data with the wrong parity.
    ProtocolError,
    /// The samples ended or were lost while the packet was under way.
    Aborted,
}

/// The interface a select sequence or an activation code switches a debug port to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Interface {
    /// Serial Wire Debug: selected by `0xE79E` after a line reset, or by the Arm CoreSight
    /// SW-DP activation code after a Selection Alert.
    Swd,
    /// JTAG: selected by `0xE73C` after a line reset, or by the Arm CoreSight JTAG-DP or the
    /// JTAG-Serial activation code after a Selection Alert.
    Jtag,
    /// The dormant state, selected by `0xE3BC` after a line reset; a Selection Alert and an
    /// activation code leave it.
    Dormant,
}

/// What travels on an SWD event stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum SwdEvent {
    /// A packet begins with its Start bit.
    Begin {
        /// The packet that begins.
        tx: TxId,
    },
    /// The packet request the host sent.
    Request {
        /// The packet.
        tx: TxId,
        /// APnDP: the request is for an access port, not the debug port.
        apndp: bool,
        /// RnW: the request reads, rather than writes.
        rnw: bool,
        /// A\[3:2\] as a byte address: `0x0`, `0x4`, `0x8` or `0xC`.
        addr: u8,
        /// Whether the parity bit makes the count of ones in APnDP, RnW, A\[2\], A\[3\] and
        /// parity even.
        parity_ok: bool,
    },
    /// The acknowledge the target sent.
    Ack {
        /// The packet.
        tx: TxId,
        /// The acknowledge.
        value: Ack,
    },
    /// The data phase: 32 bits, least significant first, and their parity bit.
    Data {
        /// The packet.
        tx: TxId,
        /// The data word.
        value: u32,
        /// Whether the parity bit makes the count of ones in the word and parity even.
        parity_ok: bool,
        /// Whether the target sent the word (a read) rather than the host (a write).
        from_target: bool,
    },
    /// The packet ended.
    End {
        /// The packet that ends.
        tx: TxId,
        /// Why it ended.
        reason: EndReason,
    },
    /// SWDIO was held high for at least 50 clocks: a line reset.
    LineReset,
    /// A select sequence followed a line reset, or a Selection Alert and an activation code
    /// woke the port from the dormant state.
    Switch {
        /// The interface selected.
        to: Interface,
    },
}

impl EventSchema for SwdEvent {
    const SCHEMA: SchemaId = SWD_EVENTS;
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
            json(&SwdEvent::Begin { tx: TxId(7) }),
            r#"{"kind":"begin","tx":7}"#
        );
        assert_eq!(
            json(&SwdEvent::Request {
                tx: TxId(7),
                apndp: true,
                rnw: false,
                addr: 0xc,
                parity_ok: true
            }),
            r#"{"kind":"request","tx":7,"apndp":true,"rnw":false,"addr":12,"parityOk":true}"#
        );
        assert_eq!(
            json(&SwdEvent::Ack {
                tx: TxId(7),
                value: Ack::Wait
            }),
            r#"{"kind":"ack","tx":7,"value":"wait"}"#
        );
        assert_eq!(
            json(&SwdEvent::Ack {
                tx: TxId(7),
                value: Ack::Other(7)
            }),
            r#"{"kind":"ack","tx":7,"value":{"other":7}}"#
        );
        assert_eq!(
            json(&SwdEvent::Data {
                tx: TxId(7),
                value: 0x0001_0203,
                parity_ok: false,
                from_target: true
            }),
            r#"{"kind":"data","tx":7,"value":66051,"parityOk":false,"fromTarget":true}"#
        );
        assert_eq!(
            json(&SwdEvent::End {
                tx: TxId(7),
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
