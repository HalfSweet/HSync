// SPDX-License-Identifier: Apache-2.0

//! Making LIN traffic: the bytes of headers and responses, and whole frames sent through the
//! UART synthesizer.

use super::fields::{SYNC, classic_checksum, enhanced_checksum, protected_id};
use crate::uart::synth::UartWaveform;

/// The checksum a response carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Checksum {
    /// Over the data bytes, as LIN 1.x frames and the diagnostic frames carry it.
    Classic,
    /// Over the protected identifier and the data bytes, as LIN 2.x frames carry it.
    Enhanced,
}

/// The bytes of a header after its break field: the sync byte and the protected identifier of
/// the frame identifier `id`.
pub fn header(id: u8) -> [u8; 2] {
    [SYNC, protected_id(id)]
}

/// The bytes of a response in the frame with identifier `id`: `data` and its checksum.
pub fn response(id: u8, data: &[u8], checksum: Checksum) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(data.len() + 1);
    bytes.extend_from_slice(data);
    bytes.push(match checksum {
        Checksum::Classic => classic_checksum(data),
        Checksum::Enhanced => enhanced_checksum(protected_id(id), data),
    });
    bytes
}

/// Sends a break field: `break_samples` samples at dominant, then a break delimiter of
/// `delimiter_samples` samples at recessive.
pub fn break_field(waveform: &mut UartWaveform, break_samples: u64, delimiter_samples: u64) {
    waveform.break_(break_samples);
    waveform.idle(delimiter_samples);
}

/// Sends a frame without any space in it: a break field of 13 bit times and a delimiter of one,
/// the header of `id`, then `response` (from [`response`], or any other bytes), each byte right
/// after the one before; `bit_samples` is the waveform's bit time in samples.
pub fn frame(waveform: &mut UartWaveform, bit_samples: u64, id: u8, response: &[u8]) {
    break_field(waveform, 13 * bit_samples, bit_samples);
    waveform.bytes(header(id));
    waveform.bytes(response.iter().copied());
}

#[cfg(test)]
mod tests {
    use uiharu_core::{Tick, TickRange};

    use super::*;
    use crate::uart::UartConfig;

    #[test]
    fn responses_end_with_their_checksum() {
        let data = [0x4a, 0x55, 0x93, 0xe5];
        assert_eq!(
            response(0x10, &data, Checksum::Classic),
            vec![0x4a, 0x55, 0x93, 0xe5, 0xe6]
        );
        // Identifier 0x10 is protected as 0x50.
        assert_eq!(header(0x10), [0x55, 0x50]);
        let enhanced = response(0x10, &data, Checksum::Enhanced);
        assert_eq!(enhanced[4], enhanced_checksum(0x50, &data));
        assert_ne!(enhanced[4], 0xe6);
    }

    #[test]
    fn a_frame_is_a_break_field_a_header_and_a_response() {
        let mut waveform = UartWaveform::new(UartConfig::default(), 1, 1, Tick::ZERO);
        frame(&mut waveform, 1, 0x3c, &[0xff]);
        let block = waveform.finish();
        let range = TickRange::new(block.start(), block.end());
        let levels: String = (range.start.0..range.end.0)
            .map(|tick| {
                if block.level(0, Tick(tick)).unwrap_or(false) {
                    '1'
                } else {
                    '0'
                }
            })
            .collect();
        // Thirteen bits at dominant and the delimiter, 0x55 and 0x3C least significant bit
        // first, each between a start and a stop bit, then 0xFF.
        assert_eq!(
            levels,
            "0000000000000 1 0101010101 0001111001 0111111111".replace(' ', "")
        );
    }
}
