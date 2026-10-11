// SPDX-License-Identifier: Apache-2.0

//! The CRC registers of the CAN frame check, as ISO 11898-1 describes them: each next bit is
//! XORed with the register's top bit, the register shifts left by one, and the polynomial is
//! XORed in when that result is 1. After the last bit of the protected stream the register
//! holds the CRC sequence.

/// One CRC shift register.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Crc {
    value: u32,
    width: u32,
    /// The generator polynomial without its top term.
    poly: u32,
}

impl Crc {
    /// CRC-15 of classic frames, x¹⁵ + x¹⁴ + x¹⁰ + x⁸ + x⁷ + x⁴ + x³ + 1, starting from zero.
    pub(super) const fn crc15() -> Self {
        Self {
            value: 0,
            width: 15,
            poly: 0x4599,
        }
    }

    /// CRC-17 of FD frames with up to 16 data bytes, x¹⁷ + x¹⁶ + x¹⁴ + x¹³ + x¹¹ + x⁶ + x⁴ + x³
    /// + x + 1, starting from a 1 in the top bit (ISO 11898-1:2015).
    pub(super) const fn crc17() -> Self {
        Self {
            value: 1 << 16,
            width: 17,
            poly: 0x1_685b,
        }
    }

    /// CRC-21 of FD frames with more than 16 data bytes, x²¹ + x²⁰ + x¹³ + x¹¹ + x⁷ + x⁴ + x³ +
    /// 1, starting from a 1 in the top bit (ISO 11898-1:2015).
    pub(super) const fn crc21() -> Self {
        Self {
            value: 1 << 20,
            width: 21,
            poly: 0x10_2899,
        }
    }

    /// Feeds the next bit of the protected stream (recessive is 1).
    pub(super) fn push(&mut self, bit: bool) {
        let top = self.value >> (self.width - 1) & 1 != 0;
        self.value = (self.value << 1) & ((1 << self.width) - 1);
        if bit != top {
            self.value ^= self.poly;
        }
    }

    /// The register: the CRC sequence of the bits fed so far.
    pub(super) fn value(&self) -> u32 {
        self.value
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The remainder of `(init · x^len + message · x^width) mod generator`, by long division
    /// over bits: what the shift register computes, written the other way.
    fn long_division(message: &[bool], generator: &[u8], init: u32) -> u32 {
        let width = generator.len() - 1;
        let mut work: Vec<bool> = message.to_vec();
        work.extend(std::iter::repeat_n(false, width));
        // `init · x^len` adds the initial register to the top `width` bits of the message.
        for (bit, cell) in work.iter_mut().take(width).enumerate() {
            if init >> (width - 1 - bit) & 1 != 0 {
                *cell = !*cell;
            }
        }
        for at in 0..message.len() {
            if work[at] {
                for (offset, term) in generator.iter().enumerate() {
                    if *term == 1 {
                        work[at + offset] = !work[at + offset];
                    }
                }
            }
        }
        work[message.len()..]
            .iter()
            .fold(0, |value, bit| value << 1 | u32::from(*bit))
    }

    fn generator(terms: &[usize]) -> Vec<u8> {
        let top = terms[0];
        (0..=top)
            .map(|power| u8::from(terms.contains(&(top - power))))
            .collect()
    }

    fn pseudo_random_bits(seed: u32, len: usize) -> Vec<bool> {
        let mut state = seed;
        (0..len)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                state & 1 != 0
            })
            .collect()
    }

    #[test]
    fn registers_match_long_division() {
        let crc15 = generator(&[15, 14, 10, 8, 7, 4, 3, 0]);
        let crc17 = generator(&[17, 16, 14, 13, 11, 6, 4, 3, 1, 0]);
        let crc21 = generator(&[21, 20, 13, 11, 7, 4, 3, 0]);
        for seed in 1..40u32 {
            // Long enough that the initial value has moved through the whole register.
            let message = pseudo_random_bits(seed, 30 + seed as usize * 7);
            let run = |mut crc: Crc| {
                for bit in &message {
                    crc.push(*bit);
                }
                crc.value()
            };
            assert_eq!(run(Crc::crc15()), long_division(&message, &crc15, 0));
            assert_eq!(run(Crc::crc17()), long_division(&message, &crc17, 1 << 16));
            assert_eq!(run(Crc::crc21()), long_division(&message, &crc21, 1 << 20));
        }
    }

    #[test]
    fn a_classic_frame_has_the_crc_long_division_gives() {
        // ID 0x123, DLC 2, data AB CD, destuffed from SOF through the data field; the value was
        // computed separately by dividing the stream with 15 zeros appended by the generator.
        let bits: Vec<bool> = "0 00100100011 0 0 0 0010 10101011 11001101"
            .chars()
            .filter(|c| !c.is_whitespace())
            .map(|c| c == '1')
            .collect();
        let mut crc = Crc::crc15();
        for bit in bits {
            crc.push(bit);
        }
        assert_eq!(crc.value(), 0x7f3c);
    }
}
