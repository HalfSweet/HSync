// SPDX-License-Identifier: Apache-2.0

//! The ROM level of 1-Wire: the ROM commands every device understands, and the CRC-8 that
//! guards ROM codes (and, in many devices, their memory) — `x⁸ + x⁵ + x⁴ + 1`, computed least
//! significant bit first from zero, as application note 937 (Book of iButton Standards) and
//! the device datasheets define it.

/// Read ROM: the only slave on the bus sends its ROM code.
pub const READ_ROM: u8 = 0x33;
/// Match ROM: the master sends the ROM code of the device it addresses.
pub const MATCH_ROM: u8 = 0x55;
/// Skip ROM: the master addresses every device at once.
pub const SKIP_ROM: u8 = 0xcc;
/// Search ROM: the master walks the ROM codes of the devices, one bit per triplet.
pub const SEARCH_ROM: u8 = 0xf0;
/// Alarm Search: Search ROM among the devices whose alarm flag is set.
pub const ALARM_SEARCH: u8 = 0xec;
/// Overdrive Skip ROM: Skip ROM, after which the devices that support it run at overdrive.
pub const OVERDRIVE_SKIP_ROM: u8 = 0x3c;
/// Overdrive Match ROM: Match ROM, the ROM code sent at overdrive speed.
pub const OVERDRIVE_MATCH_ROM: u8 = 0x69;

/// The CRC-8 of `bytes`: `x⁸ + x⁵ + x⁴ + 1`, least significant bit first, from zero. A ROM code
/// or a block followed by its CRC yields zero.
pub fn crc8(bytes: &[u8]) -> u8 {
    let mut crc = 0u8;
    for byte in bytes {
        let mut byte = *byte;
        for _ in 0..8 {
            let feedback = (crc ^ byte) & 1;
            crc >>= 1;
            if feedback != 0 {
                // The polynomial without its x⁸ term, bit-reversed for the least significant
                // bit first order.
                crc ^= 0x8c;
            }
            byte >>= 1;
        }
    }
    crc
}

/// Whether the last byte of a ROM code (its first bit on the line as bit 0) is the CRC-8 of
/// the first seven.
pub fn rom_crc_ok(id: u64) -> bool {
    let bytes = id.to_le_bytes();
    crc8(&bytes[..7]) == bytes[7]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc8_matches_the_published_check_value() {
        // The check value of this CRC (known as CRC-8/MAXIM) over the ASCII digits 1 to 9.
        assert_eq!(crc8(b"123456789"), 0xa1);
        assert_eq!(crc8(&[]), 0);
    }

    #[test]
    fn a_block_followed_by_its_crc_yields_zero() {
        let mut block = vec![0x28, 0xff, 0x4c, 0x06, 0x15, 0x16, 0x03];
        block.push(crc8(&block));
        assert_eq!(crc8(&block), 0);
        let id = u64::from_le_bytes(block.try_into().expect("eight bytes"));
        assert!(rom_crc_ok(id));
        assert!(!rom_crc_ok(id ^ 1 << 9));
    }
}
