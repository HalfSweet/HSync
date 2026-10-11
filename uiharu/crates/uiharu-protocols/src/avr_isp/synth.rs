// SPDX-License-Identifier: Apache-2.0

//! Word lists of AVR serial programming instructions for
//! [`SpiWaveform`](crate::spi::synth::SpiWaveform): the programmer's bytes on MOSI, the
//! device's on MISO.
//!
//! The device answers one byte late: while the programmer sends a byte, the device shifts
//! out the byte before it, so it echoes the second byte during the third — the check
//! Programming Enable relies on — and a read's answer takes the place of the third byte's
//! echo during the fourth.

/// What MISO carries during an instruction's first byte, which no instruction reads.
pub const FIRST_ANSWER: u8 = 0xff;

/// An instruction answered by echoes alone: a load, a write, Chip Erase.
pub fn instruction(bytes: [u8; 4]) -> Vec<(u8, u8)> {
    let [first, second, third, fourth] = bytes;
    vec![
        (first, FIRST_ANSWER),
        (second, first),
        (third, second),
        (fourth, third),
    ]
}

/// A read, or Poll RDY/BSY: the three bytes that ask, a fourth of `0x00` from the
/// programmer, and `answer` from the device during it.
pub fn read(bytes: [u8; 3], answer: u8) -> Vec<(u8, u8)> {
    let [first, second, third] = bytes;
    let mut words = instruction([first, second, third, 0x00]);
    words[3].1 = answer;
    words
}

/// Programming Enable, `AC 53 00 00`. A device in sync echoes `53` during the third byte; one
/// out of sync shifts out something else, here `53` a bit late.
pub fn programming_enable(in_sync: bool) -> Vec<(u8, u8)> {
    let mut words = instruction([0xac, 0x53, 0x00, 0x00]);
    if !in_sync {
        words[2].1 = 0x53 >> 1;
    }
    words
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_device_answers_a_byte_late() {
        assert_eq!(
            instruction([0x40, 0x00, 0x05, 0xab]),
            vec![(0x40, 0xff), (0x00, 0x40), (0x05, 0x00), (0xab, 0x05)]
        );
        assert_eq!(
            read([0x30, 0x00, 0x01], 0x95),
            vec![(0x30, 0xff), (0x00, 0x30), (0x01, 0x00), (0x00, 0x95)]
        );
        assert_eq!(programming_enable(true)[2], (0x00, 0x53));
        assert_ne!(programming_enable(false)[2].1, 0x53);
    }
}
