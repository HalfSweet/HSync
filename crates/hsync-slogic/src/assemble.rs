// SPDX-License-Identifier: GPL-3.0-or-later
// Ported from sipeed/libsigrok, slogic-dev, commit
// 0d728b3e1226a61738c99c3333ad16d5c2814728:
// src/hardware/sipeed-slogic-analyzer/slogic/{slogic16u3.c,slogic32u3.c,slogic_data.c}.
// Copyright (C) 2023-2025 Shenzhen Sipeed Technology Co., Ltd.
//! Unpacks the physical-channel samples before software triggering and cross packing.

/// Converts SLogic transfer bytes into one physical-channel bit mask per sample.
/// It keeps at most one partial sample and discards the model's firmware head.
#[derive(Debug)]
pub(crate) struct WireDecoder {
    channels: u8,
    drop_left: usize,
    partial: [u8; 4],
    pending: usize,
}

impl WireDecoder {
    /// Decodes a 2-, 4-, 8-, 16- or 32-channel wire mode, all starting at channel D0, after
    /// dropping `head_bytes` once per RUN, even if USB splits them across reads.
    pub(crate) fn new(channels: u8, head_bytes: usize) -> Self {
        assert!(matches!(channels, 2 | 4 | 8 | 16 | 32));
        Self {
            channels,
            drop_left: head_bytes,
            partial: [0; 4],
            pending: 0,
        }
    }

    /// Adds a transfer of arbitrary length and appends every complete sample to `samples`.
    pub(crate) fn push(&mut self, bytes: &[u8], samples: &mut Vec<u32>) {
        let skipped = self.drop_left.min(bytes.len());
        self.drop_left -= skipped;
        let bytes = &bytes[skipped..];
        samples.reserve(bytes.len() * 8 / usize::from(self.channels) + 1);
        match self.channels {
            2 => {
                // Combo 8: four samples per byte, earliest in the low two bits
                // (`slogic_unpack_range`, sipeed/libsigrok bf6b346e597d112f48102fe6a7b2318c6d7b49e2).
                for &byte in bytes {
                    for shift in [0, 2, 4, 6] {
                        samples.push(u32::from((byte >> shift) & 3));
                    }
                }
            }
            4 => {
                for &byte in bytes {
                    // The earlier sample is the low nibble.
                    samples.push(u32::from(byte & 0x0f));
                    samples.push(u32::from(byte >> 4));
                }
            }
            8 => samples.extend(bytes.iter().map(|&byte| u32::from(byte))),
            16 => self.words::<2>(bytes, samples),
            32 => self.words::<4>(bytes, samples),
            _ => unreachable!("validated wire mode"),
        }
    }

    /// Little-endian samples of `N` bytes, which may span transfers.
    fn words<const N: usize>(&mut self, mut bytes: &[u8], samples: &mut Vec<u32>) {
        let word = |bytes: &[u8]| {
            let mut word = [0; 4];
            word[..N].copy_from_slice(bytes);
            u32::from_le_bytes(word)
        };
        if self.pending > 0 {
            let take = (N - self.pending).min(bytes.len());
            self.partial[self.pending..self.pending + take].copy_from_slice(&bytes[..take]);
            self.pending += take;
            bytes = &bytes[take..];
            if self.pending < N {
                return;
            }
            samples.push(word(&self.partial[..N]));
            self.pending = 0;
        }
        let (whole, rest) = bytes.as_chunks::<N>();
        samples.extend(whole.iter().map(|bytes| word(bytes)));
        self.partial[..rest.len()].copy_from_slice(rest);
        self.pending = rest.len();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode(decoder: &mut WireDecoder, bytes: &[u8]) -> Vec<u32> {
        let mut samples = Vec::new();
        decoder.push(bytes, &mut samples);
        samples
    }

    #[test]
    fn follows_nibble_and_little_endian_sample_order() {
        assert_eq!(
            decode(&mut WireDecoder::new(2, 0), &[0xe4, 0x1b]),
            [0, 1, 2, 3, 3, 2, 1, 0]
        );
        assert!(decode(&mut WireDecoder::new(4, 4), &[0xff; 4]).is_empty());
        let mut decoder = WireDecoder::new(4, 4);
        assert_eq!(
            decode(&mut decoder, &[0, 0, 0, 0, 0x21, 0xf8]),
            [1, 2, 8, 15]
        );
        let mut decoder = WireDecoder::new(8, 4);
        assert_eq!(
            decode(&mut decoder, &[0, 0, 0, 0, 0x12, 0x80]),
            [0x12, 0x80]
        );
        let mut decoder = WireDecoder::new(16, 4);
        assert_eq!(decode(&mut decoder, &[0, 0, 0, 0, 0x12, 0x80]), [0x8012]);
        let mut decoder = WireDecoder::new(32, 0);
        assert_eq!(
            decode(&mut decoder, &[0x12, 0x80, 0x34, 0xf1]),
            [0xf134_8012]
        );
    }

    #[test]
    fn keeps_sample_order_across_every_transfer_boundary() {
        for (channels, head) in [
            (2, 0),
            (4, 4),
            (8, 4),
            (16, 4),
            (4, 0),
            (8, 0),
            (16, 0),
            (32, 0),
        ] {
            let mask = u32::MAX >> (32 - channels);
            let samples: Vec<u32> = (0..512u32)
                .map(|i| i.wrapping_mul(0x9e37_79b9) & mask)
                .collect();
            let mut wire = vec![0xff; head];
            match channels {
                2 => wire.extend(
                    samples
                        .as_chunks::<4>()
                        .0
                        .iter()
                        .map(|s| (s[0] | s[1] << 2 | s[2] << 4 | s[3] << 6) as u8),
                ),
                4 => wire.extend(
                    samples
                        .as_chunks::<2>()
                        .0
                        .iter()
                        .map(|s| (s[0] | s[1] << 4) as u8),
                ),
                8 => wire.extend(samples.iter().map(|&s| s as u8)),
                16 => wire.extend(samples.iter().flat_map(|&s| (s as u16).to_le_bytes())),
                32 => wire.extend(samples.iter().flat_map(|s| s.to_le_bytes())),
                _ => unreachable!(),
            }
            for cut in 1..=wire.len() {
                let mut decoder = WireDecoder::new(channels, head);
                let mut actual = Vec::new();
                for chunk in wire.chunks(cut) {
                    decoder.push(chunk, &mut actual);
                    let before = actual.len();
                    decoder.push(&[], &mut actual);
                    assert_eq!(actual.len(), before);
                }
                assert_eq!(
                    actual, samples,
                    "{channels} channels, {head}-byte head, chunks of {cut}"
                );
            }
        }
    }
}
