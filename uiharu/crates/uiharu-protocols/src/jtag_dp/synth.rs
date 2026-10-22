// SPDX-License-Identifier: Apache-2.0

//! Helpers producing LSB-first JTAG scans; no complete capture is buffered here.
use crate::jtag::{Register, synth::JtagWaveform};
/// Select one four-bit JTAG-DP instruction.
pub fn instruction(wave: &mut JtagWaveform, value: u8) {
    wave.scan(Register::Ir, &[value & 15], &[1], 4);
}
/// Shift one 35-bit data register value on each line.
pub fn data_register(wave: &mut JtagWaveform, tdi: u64, tdo: u64) {
    wave.scan(Register::Dr, &tdi.to_le_bytes(), &tdo.to_le_bytes(), 35);
}
/// Pack one ADIv5 access's shifted-in request; address is A[3:2] in byte positions.
pub fn request(read: bool, addr: u8, data: u32) -> u64 {
    (u64::from(data) << 3) | (u64::from((addr >> 2) & 3) << 1) | u64::from(read)
}
/// Pack captured raw response bits. JTAG ACK 1 is WAIT and 2 is OK/FAULT.
pub fn response(ack: u8, data: u32) -> u64 {
    (u64::from(data) << 3) | u64::from(ack & 7)
}
