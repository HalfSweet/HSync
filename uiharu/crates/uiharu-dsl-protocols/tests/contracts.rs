// SPDX-License-Identifier: Apache-2.0

//! The generated contracts serialize as their pages spell them: a `kind` tag in kebab-case,
//! fields in camelCase, enums by their names, a line that is not connected as `null`.

use uiharu_dsl_protocols::pwm::PwmEvent;
use uiharu_dsl_protocols::spi::{EndReason, SpiEvent};
use uiharu_dsl_protocols::spi_nor::{Command, Direction, IdScheme, NorEnd, NorEvent};
use uiharu_dsl_protocols::uart::{ParityCheck, UartEvent};

fn json<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_string(value).expect("serializes")
}

#[test]
fn spi_events_serialize_as_the_contract_spells_them() {
    assert_eq!(
        json(&SpiEvent::Begin {
            tx: 3,
            partial: false
        }),
        r#"{"kind":"begin","tx":3,"partial":false}"#
    );
    assert_eq!(
        json(&SpiEvent::Word {
            tx: 3,
            mosi: Some(3),
            miso: None
        }),
        r#"{"kind":"word","tx":3,"mosi":3,"miso":null}"#
    );
    assert_eq!(
        json(&SpiEvent::End {
            tx: 3,
            reason: EndReason::CsRelease
        }),
        r#"{"kind":"end","tx":3,"reason":"cs-release"}"#
    );
    let back: SpiEvent =
        serde_json::from_str(r#"{"kind":"end","tx":3,"reason":"aborted"}"#).expect("parses");
    assert_eq!(
        back,
        SpiEvent::End {
            tx: 3,
            reason: EndReason::Aborted
        }
    );
}

#[test]
fn pwm_and_uart_events_serialize_as_the_contracts_spell_them() {
    assert_eq!(
        json(&PwmEvent::Cycle {
            period_ticks: 1000,
            active_ticks: 250
        }),
        r#"{"kind":"cycle","periodTicks":1000,"activeTicks":250}"#
    );
    assert_eq!(
        json(&UartEvent::Frame {
            value: 0x41,
            parity: Some(ParityCheck::Ok),
            framing_ok: true
        }),
        r#"{"kind":"frame","value":65,"parity":"ok","framingOk":true}"#
    );
    assert_eq!(
        json(&UartEvent::Frame {
            value: 0x1ff,
            parity: None,
            framing_ok: false
        }),
        r#"{"kind":"frame","value":511,"parity":null,"framingOk":false}"#
    );
    assert_eq!(
        json(&UartEvent::Break { ticks: 120 }),
        r#"{"kind":"break","ticks":120}"#
    );
}

#[test]
fn nor_events_serialize_as_the_contract_spells_them() {
    assert_eq!(
        json(&NorEvent::Begin {
            op: 1,
            tx: 2,
            command: Command::FastRead4b
        }),
        r#"{"kind":"begin","op":1,"tx":2,"command":"fast-read-4b"}"#
    );
    assert_eq!(
        json(&NorEvent::Address {
            op: 1,
            address: 0x123456,
            bytes: 3
        }),
        r#"{"kind":"address","op":1,"address":1193046,"bytes":3}"#
    );
    assert_eq!(
        json(&NorEvent::Data {
            op: 1,
            direction: Direction::Out,
            offset: 32,
            bytes: vec![1, 2]
        }),
        r#"{"kind":"data","op":1,"direction":"out","offset":32,"bytes":[1,2]}"#
    );
    assert_eq!(
        json(&NorEvent::Status {
            op: 1,
            register: 2,
            value: 0x40,
            written: true
        }),
        r#"{"kind":"status","op":1,"register":2,"value":64,"written":true}"#
    );
    assert_eq!(
        json(&NorEvent::Id {
            op: 1,
            scheme: IdScheme::ManufacturerDevice,
            bytes: vec![0xef, 0x17]
        }),
        r#"{"kind":"id","op":1,"scheme":"manufacturer-device","bytes":[239,23]}"#
    );
    assert_eq!(
        json(&NorEvent::End {
            op: 1,
            reason: NorEnd::Incomplete
        }),
        r#"{"kind":"end","op":1,"reason":"incomplete"}"#
    );
    assert_eq!(
        json(&NorEvent::UnknownCommand {
            tx: 2,
            opcode: 0x3b
        }),
        r#"{"kind":"unknown-command","tx":2,"opcode":59}"#
    );
    assert_eq!(Command::ReadStatus2.name(), "read-status-2");
    assert_eq!(Command::Enter4bMode.name(), "enter-4b-mode");
}
