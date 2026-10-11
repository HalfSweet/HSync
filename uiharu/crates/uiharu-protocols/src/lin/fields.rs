// SPDX-License-Identifier: Apache-2.0

//! The computed fields of a LIN frame: the parity bits of the protected identifier, the classic
//! and enhanced checksums, and the data length LIN 1.x may code into the identifier.
//!
//! From the LIN Specification Package Revision 2.2A, §2.3.1.3 (the protected identifier and its
//! parity equations), §2.3.1.5 (the checksum, an inverted eight-bit sum with carry) and the
//! example of §2.8.3; and from the LIN Specification Package Revision 1.3, §3.1.2, Table 3.2
//! (the number of data bytes coded in identifier bits 4 and 5).

/// The data of the sync byte field that follows the break field.
pub const SYNC: u8 = 0x55;

/// The frame identifier a protected identifier carries: its bits 0 to 5.
pub const fn frame_id(pid: u8) -> u8 {
    pid & 0x3f
}

/// The protected identifier of the frame identifier `id`, of which the low six bits count: the
/// identifier in bits 0 to 5, P0 = ID0 ⊕ ID1 ⊕ ID2 ⊕ ID4 in bit 6 and
/// P1 = ¬(ID1 ⊕ ID3 ⊕ ID4 ⊕ ID5) in bit 7.
pub const fn protected_id(id: u8) -> u8 {
    let id = frame_id(id);
    let p0 = (id ^ (id >> 1) ^ (id >> 2) ^ (id >> 4)) & 1;
    let p1 = !((id >> 1) ^ (id >> 3) ^ (id >> 4) ^ (id >> 5)) & 1;
    id | (p0 << 6) | (p1 << 7)
}

/// Whether the parity bits of the protected identifier `pid` match its frame identifier.
pub const fn pid_ok(pid: u8) -> bool {
    protected_id(pid) == pid
}

/// `sum` with `byte` added the way LIN sums: in eight bits, the carry added back in, which is
/// subtracting 255 whenever the sum reaches 256.
pub(crate) const fn add_with_carry(sum: u8, byte: u8) -> u8 {
    let total = sum as u16 + byte as u16;
    if total >= 256 {
        (total - 255) as u8
    } else {
        total as u8
    }
}

/// The eight-bit sum with carry of `bytes`, starting from `start`.
const fn sum_with_carry(start: u8, bytes: &[u8]) -> u8 {
    let mut sum = start;
    let mut i = 0;
    while i < bytes.len() {
        sum = add_with_carry(sum, bytes[i]);
        i += 1;
    }
    sum
}

/// The classic checksum of `data`: the inverted eight-bit sum with carry of the data bytes, as
/// LIN 1.x frames and the diagnostic frames carry it.
pub const fn classic_checksum(data: &[u8]) -> u8 {
    !sum_with_carry(0, data)
}

/// The enhanced checksum of `data` in a frame with the protected identifier `pid`: the inverted
/// eight-bit sum with carry of the protected identifier and the data bytes, as LIN 2.x frames
/// carry it.
pub const fn enhanced_checksum(pid: u8, data: &[u8]) -> u8 {
    !sum_with_carry(pid, data)
}

/// The number of data bytes LIN 1.x codes into bits 4 and 5 of the frame identifier `id`: 2 for
/// identifiers 0 to 31, 4 for 32 to 47, 8 for 48 to 63.
pub const fn data_length_by_id(id: u8) -> u8 {
    match (id >> 4) & 0b11 {
        0b10 => 4,
        0b11 => 8,
        _ => 2,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protected_identifiers_match_the_table_of_valid_frame_identifiers() {
        // LIN 2.2A, Table 2.4.
        let table = [
            (0x00, 0x80),
            (0x01, 0xc1),
            (0x02, 0x42),
            (0x03, 0x03),
            (0x10, 0x50),
            (0x15, 0x55),
            (0x20, 0x20),
            (0x2a, 0x6a),
            (0x30, 0xf0),
            (0x3b, 0xfb),
            (0x3c, 0x3c),
            (0x3d, 0x7d),
            (0x3e, 0xfe),
            (0x3f, 0xbf),
        ];
        for (id, pid) in table {
            assert_eq!(protected_id(id), pid, "identifier {id:#04x}");
            assert!(pid_ok(pid));
            assert_eq!(frame_id(pid), id);
        }
        // Only the low six bits of the identifier count.
        assert_eq!(protected_id(0xc1), 0xc1);
        assert!(!pid_ok(0x00));
        assert!(!pid_ok(0x3c ^ 0x80));
    }

    #[test]
    fn checksums_sum_with_carry_and_invert() {
        // The example of LIN 2.2A §2.8.3: the sum is 0x19, the checksum 0xE6.
        let data = [0x4a, 0x55, 0x93, 0xe5];
        assert_eq!(sum_with_carry(0, &data), 0x19);
        assert_eq!(classic_checksum(&data), 0xe6);
        // The same four bytes as a protected identifier and three data bytes.
        assert_eq!(enhanced_checksum(0x4a, &data[1..]), 0xe6);
        assert_eq!(classic_checksum(&[]), 0xff);
        assert_eq!(add_with_carry(0xff, 0xff), 0xff);
        assert_eq!(add_with_carry(0x80, 0x80), 0x01);
        assert_eq!(add_with_carry(0x7f, 0x80), 0xff);
    }

    #[test]
    fn identifier_bits_4_and_5_code_the_data_length() {
        assert_eq!(data_length_by_id(0x00), 2);
        assert_eq!(data_length_by_id(0x1f), 2);
        assert_eq!(data_length_by_id(0x20), 4);
        assert_eq!(data_length_by_id(0x2f), 4);
        assert_eq!(data_length_by_id(0x30), 8);
        assert_eq!(data_length_by_id(0x3c), 8);
        assert_eq!(data_length_by_id(0x3f), 8);
    }
}
