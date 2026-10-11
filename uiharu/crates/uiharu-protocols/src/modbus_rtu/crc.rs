// SPDX-License-Identifier: Apache-2.0

//! The CRC that closes a Modbus RTU frame, as the serial line specification defines it.

/// The CRC's value before the first byte.
pub(crate) const INITIAL: u16 = 0xffff;

/// The Modbus CRC-16 of `bytes`: the polynomial 0x8005 processed least significant bit first
/// (0xA001), starting from 0xFFFF, without a final XOR. A frame sends it low byte first.
pub fn crc16(bytes: &[u8]) -> u16 {
    bytes
        .iter()
        .fold(INITIAL, |crc, byte| crc16_update(crc, *byte))
}

/// `crc` with `byte` added: the CRC of a frame built as its bytes arrive.
pub(crate) const fn crc16_update(crc: u16, byte: u8) -> u16 {
    let mut crc = crc ^ byte as u16;
    let mut bit = 0;
    while bit < 8 {
        crc = if crc & 1 != 0 {
            (crc >> 1) ^ 0xa001
        } else {
            crc >> 1
        };
        bit += 1;
    }
    crc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_the_published_check_values() {
        // The check value of CRC-16/MODBUS over the ASCII digits 1 to 9.
        assert_eq!(crc16(b"123456789"), 0x4b37);
        // Reading three holding registers from server 17, and its answer: 76 87 and 49 AD on
        // the line.
        assert_eq!(crc16(&[0x11, 0x03, 0x00, 0x6b, 0x00, 0x03]), 0x8776);
        assert_eq!(
            crc16(&[0x11, 0x03, 0x06, 0xae, 0x41, 0x56, 0x52, 0x43, 0x40]),
            0xad49
        );
        assert_eq!(crc16(&[]), INITIAL);
    }
}
