// SPDX-License-Identifier: Apache-2.0

//! The `mdio.events` contract: what the MDIO decoder emits.

use serde::{Deserialize, Serialize};
use uiharu_core::{EventSchema, SchemaId};

/// The schema of MDIO event streams.
pub const MDIO_EVENTS: SchemaId = SchemaId::new("mdio.events", 1);

/// The clause of IEEE 802.3 whose frame format a frame follows, written as its number.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(into = "u8", try_from = "u8")]
pub enum Clause {
    /// Clause 22, the MII management frame: ST `01`, a PHY and a register address.
    C22,
    /// Clause 45, the frame of MDIO manageable devices: ST `00`, a port and a device
    /// address, the register reached through an address frame before.
    C45,
}

impl Clause {
    /// The clause's number.
    pub const fn number(self) -> u8 {
        match self {
            Clause::C22 => 22,
            Clause::C45 => 45,
        }
    }
}

impl From<Clause> for u8 {
    fn from(clause: Clause) -> u8 {
        clause.number()
    }
}

/// A number that names no clause with a management frame format.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("clause {0} defines no management frame; 22 and 45 do")]
pub struct UnknownClause(pub u8);

impl TryFrom<u8> for Clause {
    type Error = UnknownClause;

    fn try_from(number: u8) -> Result<Self, Self::Error> {
        match number {
            22 => Ok(Clause::C22),
            45 => Ok(Clause::C45),
            other => Err(UnknownClause(other)),
        }
    }
}

/// What a frame does, from its OP field.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Op {
    /// The device sends the register's value: OP `10` in clause 22, `11` in clause 45.
    Read,
    /// The station writes the register: OP `01` in both clauses.
    Write,
    /// The station sets the register address that the next frames to the device use:
    /// clause 45, OP `00`.
    Address,
    /// The device sends the register's value, then advances its register address: clause 45,
    /// OP `10`.
    PostReadIncrement,
}

impl Op {
    /// Whether the device drives the data bits, from the second turnaround bit on.
    pub const fn device_drives(self) -> bool {
        matches!(self, Op::Read | Op::PostReadIncrement)
    }
}

/// What went wrong.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ErrorReason {
    /// A clause 22 start (ST `01`) with an OP that clause 22 does not define, `00` or `11`:
    /// no frame. The decoder skips bits until it is in sync again.
    Start,
    /// The turnaround broke its rule: on a write or an address frame the station sends `10`,
    /// on a read the device drives the second bit `0`. The frame goes on, and is reported
    /// with the bits as seen.
    Turnaround,
    /// A gap or the end of the samples cut the frame under way.
    Aborted,
}

/// What travels on an MDIO event stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum MdioEvent {
    /// A whole management frame.
    Frame {
        /// The frame format: 22 or 45.
        clause: Clause,
        /// The operation.
        op: Op,
        /// PHYAD in clause 22, PRTAD in clause 45: the port address, 0 to 31.
        phy: u8,
        /// REGAD in clause 22, DEVAD in clause 45: the register or the device, 0 to 31.
        reg: u8,
        /// The two turnaround bits as seen, the first in bit 1.
        turnaround: u8,
        /// The sixteen data bits, the first in bit 15: register data, or the register
        /// address of a clause 45 address frame.
        data: u16,
        /// The ones clocked in a row before ST, counted up to the 32 of a preamble: 0 when
        /// the station suppressed it.
        preamble_bits: u8,
    },
    /// Something on MDIO that is not a whole, well-formed frame.
    Error {
        /// What it was.
        reason: ErrorReason,
    },
}

impl EventSchema for MdioEvent {
    const SCHEMA: SchemaId = MDIO_EVENTS;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn json(event: &MdioEvent) -> String {
        serde_json::to_string(event).expect("serializes")
    }

    #[test]
    fn serializes_as_the_contract_spells_it() {
        assert_eq!(
            json(&MdioEvent::Frame {
                clause: Clause::C45,
                op: Op::PostReadIncrement,
                phy: 3,
                reg: 1,
                turnaround: 0b10,
                data: 0x1234,
                preamble_bits: 32,
            }),
            r#"{"kind":"frame","clause":45,"op":"post-read-increment","phy":3,"reg":1,"turnaround":2,"data":4660,"preambleBits":32}"#
        );
        assert_eq!(
            json(&MdioEvent::Error {
                reason: ErrorReason::Turnaround,
            }),
            r#"{"kind":"error","reason":"turnaround"}"#
        );
        let name = |reason: ErrorReason| serde_json::to_value(reason).expect("serializes");
        assert_eq!(name(ErrorReason::Start), "start");
        assert_eq!(name(ErrorReason::Aborted), "aborted");
        let op = |op: Op| serde_json::to_value(op).expect("serializes");
        assert_eq!(op(Op::Read), "read");
        assert_eq!(op(Op::Write), "write");
        assert_eq!(op(Op::Address), "address");
    }

    #[test]
    fn clauses_are_numbers() {
        assert_eq!(
            serde_json::to_value(Clause::C22).expect("serializes"),
            serde_json::json!(22)
        );
        let clause: Clause = serde_json::from_str("45").expect("parses");
        assert_eq!(clause, Clause::C45);
        assert!(serde_json::from_str::<Clause>("23").is_err());
        assert_eq!(Clause::try_from(46), Err(UnknownClause(46)));
        let event: MdioEvent = serde_json::from_str(
            r#"{"kind":"frame","clause":22,"op":"read","phy":1,"reg":2,"turnaround":2,"data":7,"preambleBits":0}"#,
        )
        .expect("parses");
        assert_eq!(
            event,
            MdioEvent::Frame {
                clause: Clause::C22,
                op: Op::Read,
                phy: 1,
                reg: 2,
                turnaround: 2,
                data: 7,
                preamble_bits: 0,
            }
        );
    }

    #[test]
    fn reads_are_driven_by_the_device() {
        assert!(Op::Read.device_drives());
        assert!(Op::PostReadIncrement.device_drives());
        assert!(!Op::Write.device_drives());
        assert!(!Op::Address.device_drives());
    }
}
