// SPDX-License-Identifier: Apache-2.0
//! Canonical AVR ISP contract declared in the DSL.
use uiharu_core::SchemaId;
pub use uiharu_dsl_protocols::avr_isp::{Command, ErrorReason, IspEvent};
/// AVR ISP event schema.
pub const AVR_ISP_EVENTS: SchemaId = SchemaId::new("avrisp.events", 1);
/// Bytes in one complete instruction.
pub const INSTRUCTION_BYTES: usize = 4;
/// Recognized instruction identity.
pub type OpId = u64;
#[cfg(test)]
mod tests {
    use super::*;

    fn json(event: &IspEvent) -> String {
        serde_json::to_string(event).expect("serializes")
    }

    #[test]
    fn serializes_as_the_contract_spells_it() {
        assert_eq!(
            json(&IspEvent::Instruction {
                op: 2,
                tx: 1,
                command: Command::ReadProgramMemoryHigh,
                address: Some(0x0123),
                data_in: None,
                data_out: Some(0x94),
                in_sync: None,
                bytes: [0x28, 0x01, 0x23, 0x00],
            }),
            r#"{"kind":"instruction","op":2,"tx":1,"command":"read-program-memory-high","address":291,"dataIn":null,"dataOut":148,"inSync":null,"bytes":[40,1,35,0]}"#
        );
        assert_eq!(
            json(&IspEvent::Unknown {
                tx: 0,
                bytes: [0xff, 0, 0, 0],
            }),
            r#"{"kind":"unknown","tx":0,"bytes":[255,0,0,0]}"#
        );
        assert_eq!(
            json(&IspEvent::Error {
                tx: 3,
                reason: ErrorReason::Incomplete,
                bytes: vec![0xac, 0x53],
            }),
            r#"{"kind":"error","tx":3,"reason":"incomplete","bytes":[172,83]}"#
        );
    }

    #[test]
    fn commands_are_named_as_the_datasheets_name_them() {
        let name = |command: Command| serde_json::to_value(command).expect("serializes");
        assert_eq!(name(Command::ProgrammingEnable), "programming-enable");
        assert_eq!(name(Command::PollRdyBsy), "poll-rdy-bsy");
        assert_eq!(name(Command::LoadExtendedAddress), "load-extended-address");
        assert_eq!(
            name(Command::LoadProgramMemoryPageHigh),
            "load-program-memory-page-high"
        );
        assert_eq!(
            name(Command::LoadEepromMemoryPage),
            "load-eeprom-memory-page"
        );
        assert_eq!(name(Command::ReadFuseHighBits), "read-fuse-high-bits");
        assert_eq!(
            name(Command::ReadExtendedFuseBits),
            "read-extended-fuse-bits"
        );
        assert_eq!(
            name(Command::WriteEepromMemoryPage),
            "write-eeprom-memory-page"
        );
        assert_eq!(
            serde_json::to_value(ErrorReason::Aborted).expect("serializes"),
            "aborted"
        );
    }
}
