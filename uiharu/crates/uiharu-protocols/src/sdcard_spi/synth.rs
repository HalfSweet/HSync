// SPDX-License-Identifier: Apache-2.0

//! Word lists of SD card exchanges in SPI mode for
//! [`SpiWaveform`](crate::spi::synth::SpiWaveform): the host's bytes on MOSI, the card's on
//! MISO, `0xFF` on a line that carries nothing. The CRCs come from plain bit-by-bit divisions
//! by the specification's polynomials, independent of the decoder's table-driven ones.

use super::events::ResponseFormat;

/// What a line carries when nothing is sent on it.
const FILL: u8 = 0xff;

/// CRC7 of `bytes`, G(x) = x⁷ + x³ + 1, shifted through one bit at a time.
pub fn crc7(bytes: &[u8]) -> u8 {
    let mut crc = 0u8;
    for byte in bytes {
        for bit in (0..8).rev() {
            let feedback = ((crc >> 6) & 1) ^ ((byte >> bit) & 1);
            crc = (crc << 1) & 0x7f;
            if feedback != 0 {
                crc ^= 0x09;
            }
        }
    }
    crc
}

/// CRC16 of `bytes`, G(x) = x¹⁶ + x¹² + x⁵ + 1, shifted through one bit at a time.
pub fn crc16(bytes: &[u8]) -> u16 {
    let mut crc = 0u16;
    for byte in bytes {
        for bit in (0..8).rev() {
            let feedback = ((crc >> 15) & 1) ^ u16::from((byte >> bit) & 1);
            crc <<= 1;
            if feedback != 0 {
                crc ^= 0x1021;
            }
        }
    }
    crc
}

/// The six bytes of command `index` with `argument`: the start and transmission bits with the
/// index, the argument most significant byte first, then the CRC7 and the end bit.
pub fn frame(index: u8, argument: u32) -> [u8; 6] {
    let [a3, a2, a1, a0] = argument.to_be_bytes();
    let head = [0x40 | (index & 0x3f), a3, a2, a1, a0];
    [head[0], a3, a2, a1, a0, (crc7(&head) << 1) | 1]
}

/// A command frame from the host.
pub fn command(index: u8, argument: u32) -> Vec<(u8, u8)> {
    frame(index, argument)
        .into_iter()
        .map(|byte| (byte, FILL))
        .collect()
}

/// `count` bytes of fill on both lines.
pub fn fill(count: usize) -> Vec<(u8, u8)> {
    vec![(FILL, FILL); count]
}

/// A response of `format` after one byte of fill: `bytes`, R1 first, from the card; for R1b,
/// then `busy` busy bytes and the byte that shows the card ready.
pub fn response(format: ResponseFormat, bytes: &[u8], busy: u16) -> Vec<(u8, u8)> {
    response_after(1, format, bytes, busy)
}

/// A response after `fill_bytes` bytes of fill, as [`response`].
pub fn response_after(
    fill_bytes: usize,
    format: ResponseFormat,
    bytes: &[u8],
    busy: u16,
) -> Vec<(u8, u8)> {
    let mut words = fill(fill_bytes);
    words.extend(bytes.iter().map(|byte| (FILL, *byte)));
    if format == ResponseFormat::R1b {
        words.extend(std::iter::repeat_n((FILL, 0x00), usize::from(busy)));
        words.push((FILL, FILL));
    }
    words
}

/// A block from the card after one byte of fill: the start block token, `data`, its CRC16.
pub fn read_block(data: &[u8]) -> Vec<(u8, u8)> {
    read_block_with_crc(data, crc16(data))
}

/// A block from the card, as [`read_block`], whose CRC16 field is `crc`, right or wrong.
pub fn read_block_with_crc(data: &[u8], crc: u16) -> Vec<(u8, u8)> {
    let mut words = fill(1);
    words.push((FILL, 0xfe));
    words.extend(data.iter().map(|byte| (FILL, *byte)));
    let [high, low] = crc.to_be_bytes();
    words.extend([(FILL, high), (FILL, low)]);
    words
}

/// A data error token from the card, after one byte of fill, in place of a block.
pub fn error_token(value: u8) -> Vec<(u8, u8)> {
    vec![(FILL, FILL), (FILL, value)]
}

/// A block from the host after one byte of fill: `token` (`0xFE`, or `0xFC` in a
/// multiple-block write), `data` and its CRC16; then from the card the data response
/// `response`, `busy` busy bytes and the byte that shows the card ready.
pub fn write_block(token: u8, data: &[u8], response: u8, busy: u16) -> Vec<(u8, u8)> {
    let mut words = fill(1);
    words.push((token, FILL));
    words.extend(data.iter().map(|byte| (*byte, FILL)));
    let [high, low] = crc16(data).to_be_bytes();
    words.extend([(high, FILL), (low, FILL)]);
    words.push((FILL, response));
    words.extend(std::iter::repeat_n((FILL, 0x00), usize::from(busy)));
    words.push((FILL, FILL));
    words
}

/// The host's stop token of a multiple-block write after one byte of fill, then a byte, `busy`
/// busy bytes and the byte that shows the card ready.
pub fn stop_tran(busy: u16) -> Vec<(u8, u8)> {
    let mut words = fill(1);
    words.push((0xfd, FILL));
    words.push((FILL, FILL));
    words.extend(std::iter::repeat_n((FILL, 0x00), usize::from(busy)));
    words.push((FILL, FILL));
    words
}

/// The data response token for status `status`: `0b010` accepted, `0b101` CRC error, `0b110`
/// write error; the undefined upper bits are sent as ones.
pub const fn data_response(status: u8) -> u8 {
    0xe1 | ((status & 0x07) << 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_carry_the_published_crcs() {
        assert_eq!(frame(0, 0), [0x40, 0, 0, 0, 0, 0x95]);
        assert_eq!(frame(8, 0x1aa), [0x48, 0, 0, 0x01, 0xaa, 0x87]);
        // §4.5's examples: CMD17 with argument 0, and a response to it.
        assert_eq!(frame(17, 0)[5], (0b010_1010 << 1) | 1);
        assert_eq!(crc7(&[0x11, 0, 0, 0x09, 0]), 0b011_0011);
    }

    #[test]
    fn crc16_gives_the_published_value() {
        assert_eq!(crc16(&[0xff; 512]), 0x7fa1);
        assert_eq!(crc16(b"123456789"), 0x31c3);
    }

    #[test]
    fn data_responses_have_their_form() {
        assert_eq!(data_response(0b010), 0xe5);
        assert_eq!(data_response(0b101), 0xeb);
        assert_eq!(data_response(0b110), 0xed);
    }

    #[test]
    fn blocks_end_with_their_crc() {
        let words = read_block(&[0xff; 512]);
        assert_eq!(words.len(), 1 + 1 + 512 + 2);
        assert_eq!(words[1], (0xff, 0xfe));
        assert_eq!(words[514], (0xff, 0x7f));
        assert_eq!(words[515], (0xff, 0xa1));
        let words = write_block(0xfc, &[1, 2], data_response(0b010), 2);
        assert_eq!(
            words,
            vec![
                (0xff, 0xff),
                (0xfc, 0xff),
                (1, 0xff),
                (2, 0xff),
                (crc16(&[1, 2]).to_be_bytes()[0], 0xff),
                (crc16(&[1, 2]).to_be_bytes()[1], 0xff),
                (0xff, 0xe5),
                (0xff, 0x00),
                (0xff, 0x00),
                (0xff, 0xff),
            ]
        );
    }
}
