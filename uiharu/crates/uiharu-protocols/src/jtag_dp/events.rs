// SPDX-License-Identifier: Apache-2.0

//! JTAG-DP's unchanged JSON contract, declared in the decoder DSL for semantic consumers.

use uiharu_core::SchemaId;
pub use uiharu_dsl_protocols::jtag_dp::{Ack, JtagDpEvent, ScanStatus};
/// JTAG-DP output schema.
pub const JTAG_DP_EVENTS: SchemaId = SchemaId::new("jtag-dp.events", 1);

/// Interprets JTAG's ACK bits; this encoding differs from SWD's ACK.
pub(super) fn ack_from_bits(bits: u8) -> Ack {
    match bits {
        1 => Ack::Wait,
        2 => Ack::OkFault,
        _ => Ack::Invalid,
    }
}
