// SPDX-License-Identifier: Apache-2.0

//! Word lists of nRF24L01+ commands for [`SpiWaveform`](crate::spi::synth::SpiWaveform): the
//! host's bytes on MOSI, the chip's on MISO, one transaction each. The chip sends STATUS while
//! the command byte goes in; after it, MOSI carries NOP (`0xFF`) while the host only reads,
//! and MISO `0x00` while the chip has nothing the decoder reads.

use super::commands::{ACTIVATE, ACTIVATE_KEY};

/// What MOSI carries while the host only clocks the chip's bytes out.
const HOST_IDLE: u8 = 0xff;

/// What MISO carries while the host writes.
const CHIP_IDLE: u8 = 0x00;

/// A command without data — FLUSH_TX, FLUSH_RX, REUSE_TX_PL, NOP — or any byte alone, with
/// the chip's `status`.
pub fn command(byte: u8, status: u8) -> Vec<(u8, u8)> {
    vec![(byte, status)]
}

/// A command that writes `data` into the chip: W_REGISTER, the payload writes.
pub fn write(byte: u8, status: u8, data: &[u8]) -> Vec<(u8, u8)> {
    let mut words = command(byte, status);
    words.extend(data.iter().map(|byte| (*byte, CHIP_IDLE)));
    words
}

/// A command that reads `data` out of the chip: R_REGISTER, R_RX_PAYLOAD, R_RX_PL_WID.
pub fn read(byte: u8, status: u8, data: &[u8]) -> Vec<(u8, u8)> {
    let mut words = command(byte, status);
    words.extend(data.iter().map(|byte| (HOST_IDLE, *byte)));
    words
}

/// R_REGISTER of the register at `address`, which holds `data`, least significant byte
/// first.
pub fn read_register(address: u8, status: u8, data: &[u8]) -> Vec<(u8, u8)> {
    read(address & 0x1f, status, data)
}

/// W_REGISTER of `data`, least significant byte first, to the register at `address`.
pub fn write_register(address: u8, status: u8, data: &[u8]) -> Vec<(u8, u8)> {
    write(0x20 | (address & 0x1f), status, data)
}

/// ACTIVATE with its key.
pub fn activate(status: u8) -> Vec<(u8, u8)> {
    write(ACTIVATE, status, &[ACTIVATE_KEY])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_comes_with_the_command_byte() {
        assert_eq!(command(0xff, 0x0e), vec![(0xff, 0x0e)]);
        assert_eq!(
            write_register(0x10, 0x0e, &[0xe7, 0xe6]),
            vec![(0x30, 0x0e), (0xe7, 0x00), (0xe6, 0x00)]
        );
        assert_eq!(
            read_register(0x07, 0x0e, &[0x0e]),
            vec![(0x07, 0x0e), (0xff, 0x0e)]
        );
        assert_eq!(activate(0x0e), vec![(0x50, 0x0e), (0x73, 0x00)]);
    }
}
