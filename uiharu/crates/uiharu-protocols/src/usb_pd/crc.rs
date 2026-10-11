// SPDX-License-Identifier: Apache-2.0

//! The CRC-32 of USB PD packets (USB Power Delivery Specification revision 3.1, §5.6.2), run as
//! a receiver runs it: a shift register over the bits in the order they are sent — the header,
//! the data and the CRC field itself, each least significant bit first — that holds the
//! residual `0xC704DD7B` afterwards when nothing was damaged.

/// The CRC register.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Crc32(u32);

impl Crc32 {
    /// The generator polynomial without its top term: x³² + x²⁶ + x²³ + x²² + x¹⁶ + x¹² + x¹¹ +
    /// x¹⁰ + x⁸ + x⁷ + x⁵ + x⁴ + x² + x + 1.
    const POLY: u32 = 0x04c1_1db7;
    /// What a correct packet, its CRC included, leaves in the register.
    const RESIDUAL: u32 = 0xc704_dd7b;

    /// The register as a packet's header starts: all ones.
    pub(super) fn new() -> Self {
        Self(0xffff_ffff)
    }

    /// Runs the `count` low bits of `value` through the register, the least significant first:
    /// each bit is compared with the register's top bit, the register shifts left, and the
    /// polynomial is added when they differed.
    pub(super) fn push(&mut self, value: u32, count: u32) {
        for i in 0..count {
            let top = self.0 >> 31;
            self.0 <<= 1;
            if (value >> i) & 1 != top {
                self.0 ^= Self::POLY;
            }
        }
    }

    /// Whether the bits run so far end in a matching CRC.
    pub(super) fn is_residual(&self) -> bool {
        self.0 == Self::RESIDUAL
    }

    /// The CRC field the bits run so far call for: the register inverted, its top bit sent
    /// first, so read least significant bit first it is the register reversed.
    #[cfg(test)]
    fn field(&self) -> u32 {
        (!self.0).reverse_bits()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(bytes: &[u8]) -> Crc32 {
        let mut crc = Crc32::new();
        for byte in bytes {
            crc.push(u32::from(*byte), 8);
        }
        crc
    }

    #[test]
    fn computes_the_standard_check_value() {
        // The CRC-32 of the ASCII digits 1 to 9, sent least significant bit first.
        assert_eq!(run(b"123456789").field(), 0xcbf4_3926);
    }

    #[test]
    fn a_packet_and_its_crc_leave_the_residual() {
        for bytes in [
            &b""[..],
            b"\x41\x00",
            b"\x61\x01\x2c\x91\x01\x08",
            &[0xff; 30],
        ] {
            let mut crc = run(bytes);
            let field = crc.field();
            crc.push(field, 32);
            assert!(crc.is_residual(), "{bytes:02x?}");
            let mut damaged = run(bytes);
            damaged.push(field ^ 0x10, 32);
            assert!(!damaged.is_residual(), "{bytes:02x?}");
        }
    }
}
