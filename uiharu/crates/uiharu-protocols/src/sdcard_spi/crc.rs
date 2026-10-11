// SPDX-License-Identifier: Apache-2.0

//! The two CRCs of the SD card protocol, computed a byte at a time from tables, so that the
//! decoder checks a frame or a block as it streams without holding it.
//!
//! Both are the plain polynomial division the specification defines (Physical Layer
//! Simplified Specification, §4.5): initial value zero, most significant bit first, nothing
//! reflected or inverted.

/// CRC7 of a command frame: G(x) = x⁷ + x³ + 1.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Crc7 {
    /// The remainder in the upper seven bits, so that a byte is processed in one step.
    shifted: u8,
}

/// The remainders of every byte, with the polynomial shifted up by one bit to match.
const CRC7_TABLE: [u8; 256] = {
    let mut table = [0u8; 256];
    let mut byte = 0;
    while byte < 256 {
        let mut remainder = byte as u8;
        let mut bit = 0;
        while bit < 8 {
            remainder = if remainder & 0x80 != 0 {
                (remainder << 1) ^ (0x09 << 1)
            } else {
                remainder << 1
            };
            bit += 1;
        }
        table[byte] = remainder;
        byte += 1;
    }
    table
};

impl Crc7 {
    /// A CRC of nothing yet.
    pub(crate) const fn new() -> Self {
        Self { shifted: 0 }
    }

    /// Takes the next byte.
    pub(crate) fn update(&mut self, byte: u8) {
        self.shifted = CRC7_TABLE[usize::from(self.shifted ^ byte)];
    }

    /// The CRC of the bytes so far, seven bits.
    pub(crate) const fn value(self) -> u8 {
        self.shifted >> 1
    }
}

/// CRC16 of a data block: the CCITT polynomial G(x) = x¹⁶ + x¹² + x⁵ + 1.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Crc16 {
    remainder: u16,
}

const CRC16_TABLE: [u16; 256] = {
    let mut table = [0u16; 256];
    let mut byte = 0;
    while byte < 256 {
        let mut remainder = (byte as u16) << 8;
        let mut bit = 0;
        while bit < 8 {
            remainder = if remainder & 0x8000 != 0 {
                (remainder << 1) ^ 0x1021
            } else {
                remainder << 1
            };
            bit += 1;
        }
        table[byte] = remainder;
        byte += 1;
    }
    table
};

impl Crc16 {
    /// A CRC of nothing yet.
    pub(crate) const fn new() -> Self {
        Self { remainder: 0 }
    }

    /// Takes the next byte.
    pub(crate) fn update(&mut self, byte: u8) {
        let index = usize::from((self.remainder >> 8) as u8 ^ byte);
        self.remainder = (self.remainder << 8) ^ CRC16_TABLE[index];
    }

    /// The CRC of the bytes so far.
    pub(crate) const fn value(self) -> u16 {
        self.remainder
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn crc7(bytes: &[u8]) -> u8 {
        let mut crc = Crc7::new();
        bytes.iter().for_each(|byte| crc.update(*byte));
        crc.value()
    }

    fn crc16(bytes: &[u8]) -> u16 {
        let mut crc = Crc16::new();
        bytes.iter().for_each(|byte| crc.update(*byte));
        crc.value()
    }

    #[test]
    fn crc7_gives_the_published_values() {
        // CMD0 goes out as 40 00 00 00 00 95, CMD8 with 0x1AA as 48 00 00 01 AA 87: the last
        // byte is the CRC7 and the end bit.
        assert_eq!((crc7(&[0x40, 0, 0, 0, 0]) << 1) | 1, 0x95);
        assert_eq!((crc7(&[0x48, 0, 0, 0x01, 0xaa]) << 1) | 1, 0x87);
        // The examples of §4.5: CMD17 with argument 0, and a response to it.
        assert_eq!(crc7(&[0x51, 0, 0, 0, 0]), 0b010_1010);
        assert_eq!(crc7(&[0x11, 0, 0, 0x09, 0]), 0b011_0011);
    }

    #[test]
    fn crc16_gives_the_published_values() {
        // §4.5: 512 bytes of 0xFF.
        assert_eq!(crc16(&[0xff; 512]), 0x7fa1);
        // The check value of this CRC (CRC-16/XMODEM).
        assert_eq!(crc16(b"123456789"), 0x31c3);
        assert_eq!(crc16(&[]), 0);
    }
}
