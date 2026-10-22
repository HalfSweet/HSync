// SPDX-License-Identifier: Apache-2.0

//! The RTU transport contract is declared in `dsl/contracts/modbus.ts` so application
//! protocol decoders can consume the exact same event types through the TypeScript DSL.

use uiharu_core::SchemaId;
pub use uiharu_dsl_protocols::modbus::{EndReason, ModbusEvent};

/// The schema of Modbus event streams; JSON is unchanged by the generated declaration.
pub const MODBUS_EVENTS: SchemaId = SchemaId::new("modbus.events", 1);
/// The most bytes one data event carries.
pub const DATA_BATCH: usize = 32;
/// A frame, numbered from one by the RTU transport.
pub type FrameId = u64;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_as_the_contract_spells_it() {
        let json = |event: &ModbusEvent| serde_json::to_string(event).expect("serializes");
        assert_eq!(
            json(&ModbusEvent::Begin {
                frame: 2,
                address: 17,
                function: Some(0x83),
                exception: true,
                partial: false,
            }),
            r#"{"kind":"begin","frame":2,"address":17,"function":131,"exception":true,"partial":false}"#
        );
        assert_eq!(
            json(&ModbusEvent::Begin {
                frame: 3,
                address: 5,
                function: None,
                exception: false,
                partial: true,
            }),
            r#"{"kind":"begin","frame":3,"address":5,"function":null,"exception":false,"partial":true}"#
        );
        assert_eq!(
            json(&ModbusEvent::Data {
                frame: 2,
                offset: 32,
                bytes: vec![1, 2],
            }),
            r#"{"kind":"data","frame":2,"offset":32,"bytes":[1,2]}"#
        );
        assert_eq!(
            json(&ModbusEvent::Crc {
                frame: 2,
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
                json(&ModbusEvent::End { frame: 2, reason }),
                format!(r#"{{"kind":"end","frame":2,"reason":"{name}"}}"#)
            );
        }
    }
}
