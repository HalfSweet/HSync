// SPDX-License-Identifier: Apache-2.0

//! Native SD command transport and optional SDR read data, implemented from the SD
//! Association's Physical Layer Simplified Specification, sections 3.6, 4.5 and 4.9.
//! See `docs/protocols/sdcard-sd.md` for framing, provenance and supported limits.

mod decoder;
mod events;
pub mod synth;

pub use decoder::{SdcardSd, SdcardSdConfig, SdcardSdConfigError};
pub use events::{EndReason, SDCARD_SD_EVENTS, SdcardSdEvent};

/// Advances the SD CRC7 (x^7 + x^3 + 1), MSB first, initial remainder zero.
pub fn crc7_bit(crc: u8, bit: bool) -> u8 {
    let feedback = (crc & 0x40 != 0) ^ bit;
    ((crc << 1) & 0x7f) ^ if feedback { 0x09 } else { 0 }
}

/// Advances one data lane's CRC16 (x^16 + x^12 + x^5 + 1), MSB first.
pub fn crc16_bit(crc: u16, bit: bool) -> u16 {
    let feedback = (crc & 0x8000 != 0) ^ bit;
    (crc << 1) ^ if feedback { 0x1021 } else { 0 }
}
