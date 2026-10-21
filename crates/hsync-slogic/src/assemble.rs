// SPDX-License-Identifier: GPL-3.0-or-later
// Ported from sipeed/libsigrok, slogic-dev, commit
// 0d728b3e1226a61738c99c3333ad16d5c2814728:
// src/hardware/sipeed-slogic-analyzer/slogic/{slogic16u3.c,slogic_data.c}.
// Copyright (C) 2023-2025 Shenzhen Sipeed Technology Co., Ltd.
//! Unpacks the physical-channel samples before software triggering and cross packing.

/// Converts SLogic16U3 transfer bytes into one physical-channel bit mask per sample.
/// It keeps at most one partial sample and discards the firmware's four-byte head.
#[derive(Debug)]
pub(crate) struct WireDecoder {
    channels: u8,
    drop_left: usize,
    low_byte: Option<u8>,
}

impl WireDecoder {
    /// Decodes a 4-, 8-, or 16-channel wire mode, all starting at channel D0.
    pub(crate) fn new(channels: u8) -> Self {
        assert!(matches!(channels, 4 | 8 | 16));
        Self {
            channels,
            // SLogic16U3 gates the stream mid-word. The official model drops
            // four bytes once per RUN, even if USB splits them across reads.
            drop_left: 4,
            low_byte: None,
        }
    }

    /// Adds a transfer of arbitrary length and returns every complete sample.
    pub(crate) fn push(&mut self, bytes: &[u8]) -> Vec<u16> {
        let skipped = self.drop_left.min(bytes.len());
        self.drop_left -= skipped;
        let bytes = &bytes[skipped..];
        let mut samples = Vec::with_capacity(bytes.len() * 8 / usize::from(self.channels) + 1);
        match self.channels {
            4 => {
                for &byte in bytes {
                    // The earlier sample is the low nibble.
                    samples.push(u16::from(byte & 0x0f));
                    samples.push(u16::from(byte >> 4));
                }
            }
            8 => samples.extend(bytes.iter().map(|&byte| u16::from(byte))),
            16 => {
                let mut bytes = bytes.iter().copied();
                if let Some(low) = self.low_byte.take() {
                    if let Some(high) = bytes.next() {
                        samples.push(u16::from_le_bytes([low, high]));
                    } else {
                        self.low_byte = Some(low);
                        return samples;
                    }
                }
                while let Some(low) = bytes.next() {
                    if let Some(high) = bytes.next() {
                        samples.push(u16::from_le_bytes([low, high]));
                    } else {
                        self.low_byte = Some(low);
                    }
                }
            }
            _ => unreachable!("validated wire mode"),
        }
        samples
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn follows_nibble_and_little_endian_sample_order() {
        assert!(WireDecoder::new(4).push(&[0xff; 4]).is_empty());
        let mut decoder = WireDecoder::new(4);
        assert_eq!(decoder.push(&[0, 0, 0, 0, 0x21, 0xf8]), [1, 2, 8, 15]);
        let mut decoder = WireDecoder::new(8);
        assert_eq!(decoder.push(&[0, 0, 0, 0, 0x12, 0x80]), [0x12, 0x80]);
        let mut decoder = WireDecoder::new(16);
        assert_eq!(decoder.push(&[0, 0, 0, 0, 0x12, 0x80]), [0x8012]);
    }

    #[test]
    fn keeps_sample_order_across_every_transfer_boundary() {
        for channels in [4, 8, 16] {
            let mask = if channels == 16 {
                u16::MAX
            } else {
                (1 << channels) - 1
            };
            let samples: Vec<u16> = (0..510).map(|i| (i * 997) as u16 & mask).collect();
            let mut wire = vec![0xff; 4];
            match channels {
                4 => wire.extend(
                    samples
                        .as_chunks::<2>()
                        .0
                        .iter()
                        .map(|s| (s[0] | s[1] << 4) as u8),
                ),
                8 => wire.extend(samples.iter().map(|&s| s as u8)),
                16 => wire.extend(samples.iter().flat_map(|s| s.to_le_bytes())),
                _ => unreachable!(),
            }
            for cut in 1..=wire.len() {
                let mut decoder = WireDecoder::new(channels);
                let mut actual = Vec::new();
                for chunk in wire.chunks(cut) {
                    actual.extend(decoder.push(chunk));
                    assert!(decoder.push(&[]).is_empty());
                }
                assert_eq!(actual, samples, "{channels} channels, chunks of {cut}");
            }
        }
    }
}
