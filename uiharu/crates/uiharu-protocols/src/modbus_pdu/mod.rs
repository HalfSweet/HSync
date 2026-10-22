// SPDX-License-Identifier: Apache-2.0

//! Modbus PDU semantics generated from `dsl/decoders/modbus-pdu.ts`; the input is
//! the RTU contract declared in `dsl/contracts/modbus.ts`. No decoder logic is hand-written.

pub use uiharu_dsl_protocols::modbus_pdu::{
    Direction, FieldName, ModbusPdu, ModbusPduConfig, ModbusPduEvent,
};
