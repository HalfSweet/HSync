// SPDX-License-Identifier: Apache-2.0

//! The `mdio.events` contract: what the MDIO decoder emits.

use uiharu_core::SchemaId;

/// The schema of MDIO event streams.
pub const MDIO_EVENTS: SchemaId = SchemaId::new("mdio.events", 1);

pub use uiharu_dsl_protocols::mdio::{Clause, ErrorReason, MdioEvent, Op};

/// Conversion helpers for the canonical validated numeric enum.
pub trait ClauseExt: Sized {
    /// The clause's number.
    fn number(self) -> u8;
    /// Parse exactly clause 22 or clause 45.
    fn from_number(number: u8) -> Result<Self, UnknownClause>;
}
impl ClauseExt for Clause {
    fn number(self) -> u8 {
        self.value() as u8
    }
    fn from_number(number: u8) -> Result<Self, UnknownClause> {
        match number {
            22 => Ok(Self::C22),
            45 => Ok(Self::C45),
            other => Err(UnknownClause(other)),
        }
    }
}

/// A number that names no clause with a management frame format.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("clause {0} defines no management frame; 22 and 45 do")]
pub struct UnknownClause(pub u8);

/// Helpers on the canonical MDIO operation.
pub trait OpExt {
    /// Whether the device drives the data bits, from the second turnaround bit on.
    fn device_drives(self) -> bool;
}
impl OpExt for Op {
    fn device_drives(self) -> bool {
        matches!(self, Op::Read | Op::PostReadIncrement)
    }
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
        assert_eq!(Clause::from_number(46), Err(UnknownClause(46)));
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
