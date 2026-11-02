// SPDX-License-Identifier: GPL-3.0-or-later
// The wire formats are those of sipeed/libsigrok, slogic-dev, commit
// 0d728b3e1226a61738c99c3333ad16d5c2814728 (`slogic_unpack_range()` in
// src/hardware/sipeed-slogic-analyzer/slogic/slogic_data.c, and the Combo 8's two-bit samples
// at bf6b346e597d112f48102fe6a7b2318c6d7b49e2); Copyright (C) 2023-2025 Shenzhen Sipeed
// Technology Co., Ltd. The transposition is HSync's own.
//! Turns SLogic transfer bytes straight into HSync's cross format, blocks of 64 samples with
//! one `u64` per channel, without unpacking the samples one by one.

/// Samples in a block.
pub(crate) const BLOCK: usize = 64;

/// Transposes eight bytes as an 8×8 matrix of bits: bit `c` of byte `i` becomes bit `i` of
/// byte `c`.
fn transpose(mut word: u64) -> u64 {
    let mut swap = (word ^ (word >> 7)) & 0x00aa_00aa_00aa_00aa;
    word ^= swap ^ (swap << 7);
    swap = (word ^ (word >> 14)) & 0x0000_cccc_0000_cccc;
    word ^= swap ^ (swap << 14);
    swap = (word ^ (word >> 28)) & 0x0000_0000_f0f0_f0f0;
    word ^ swap ^ (swap << 28)
}

/// Transposes eight words as an 8×8 matrix of bytes: byte `c` of word `g` becomes byte `g`
/// of word `c`.
fn transpose_bytes(rows: &mut [u64; 8]) {
    for (span, mask) in [
        (1, 0x00ff_00ff_00ff_00ff_u64),
        (2, 0x0000_ffff_0000_ffff),
        (4, 0x0000_0000_ffff_ffff),
    ] {
        let shift = span * 8;
        for base in (0..8).step_by(span * 2) {
            for row in base..base + span {
                let (low, high) = (rows[row], rows[row + span]);
                let swap = ((low >> shift) ^ high) & mask;
                rows[row] = low ^ (swap << shift);
                rows[row + span] = high ^ swap;
            }
        }
    }
}

/// One channel of a word of packed samples, `BITS` bits each with the earliest lowest, its
/// bits compacted in time order (`packed_channel` of HSync a72e8b3); the channel is the
/// word's lowest bit after shifting it down.
fn packed_channel<const BITS: usize>(mut word: u64) -> u64 {
    if BITS == 2 {
        word &= 0x5555_5555_5555_5555;
        word = (word | word >> 1) & 0x3333_3333_3333_3333;
        word = (word | word >> 2) & 0x0f0f_0f0f_0f0f_0f0f;
        word = (word | word >> 4) & 0x00ff_00ff_00ff_00ff;
        word = (word | word >> 8) & 0x0000_ffff_0000_ffff;
        (word | word >> 16) & 0xffff_ffff
    } else {
        word &= 0x1111_1111_1111_1111;
        word = (word | word >> 3) & 0x0303_0303_0303_0303;
        word = (word | word >> 6) & 0x000f_000f_000f_000f;
        word = (word | word >> 12) & 0x0000_00ff_0000_00ff;
        (word | word >> 24) & 0xffff
    }
}

/// Eight bytes `stride` apart from `start`, the first in the low byte.
fn gather(bytes: &[u8], start: usize, stride: usize) -> u64 {
    u64::from_le_bytes(std::array::from_fn(|i| bytes[start + i * stride]))
}

/// Reverses the bits of each of eight bytes.
fn reverse_bytes(mut x: u64) -> u64 {
    x = (x >> 4 & 0x0f0f_0f0f_0f0f_0f0f) | (x & 0x0f0f_0f0f_0f0f_0f0f) << 4;
    x = (x >> 2 & 0x3333_3333_3333_3333) | (x & 0x3333_3333_3333_3333) << 2;
    (x >> 1 & 0x5555_5555_5555_5555) | (x & 0x5555_5555_5555_5555) << 1
}

/// The blocks a transfer completed.
#[derive(Debug, Default)]
pub(crate) struct Blocks {
    /// One little-endian word per kept logic channel for each block, in channel order: the
    /// cross format's bytes.
    pub(crate) logic: Vec<u8>,
    /// The analog groups' codes, sample by sample, one per group.
    pub(crate) codes: Vec<u16>,
    /// Blocks of 64 samples.
    pub(crate) count: usize,
}

/// Cuts a wire stream into blocks of 64 samples and transposes each into the words of the
/// channels a capture keeps, after dropping the model's firmware head once per RUN.
#[derive(Debug)]
pub(crate) struct Assembler {
    wire: u8,
    drop_left: usize,
    /// The start of a block that a transfer did not complete.
    carry: Vec<u8>,
    /// The logic channels kept, in order.
    channels: Vec<u8>,
    /// Byte planes of a sample up to the highest kept channel (8, 16 and 32 channels).
    planes: usize,
    /// The analog groups read, whose bytes are codes.
    groups: Vec<u8>,
}

impl Assembler {
    /// Assembles a 2-, 4-, 8-, 16- or 32-channel wire mode, all starting at D0, keeping the
    /// logic channels of `logic` and reading the analog groups `groups`.
    pub(crate) fn new(wire: u8, head_bytes: usize, logic: u32, groups: Vec<u8>) -> Self {
        assert!(matches!(wire, 2 | 4 | 8 | 16 | 32));
        let logic = logic & (u32::MAX >> (32 - wire));
        Self {
            wire,
            drop_left: head_bytes,
            carry: Vec::new(),
            channels: (0..32)
                .filter(|channel| logic & (1 << channel) != 0)
                .collect(),
            planes: logic
                .checked_ilog2()
                .map_or(0, |highest| highest as usize / 8 + 1),
            groups,
        }
    }

    /// The logic channels kept, in order.
    pub(crate) fn channels(&self) -> &[u8] {
        &self.channels
    }

    /// The analog groups read.
    pub(crate) fn groups(&self) -> usize {
        self.groups.len()
    }

    fn block_bytes(&self) -> usize {
        BLOCK * usize::from(self.wire) / 8
    }

    /// Adds a transfer of any length and leaves in `out` the blocks it completed.
    pub(crate) fn push(&mut self, bytes: &[u8], out: &mut Blocks) {
        out.logic.clear();
        out.codes.clear();
        out.count = 0;
        let skipped = self.drop_left.min(bytes.len());
        self.drop_left -= skipped;
        let mut bytes = &bytes[skipped..];
        let size = self.block_bytes();
        if !self.carry.is_empty() {
            let take = (size - self.carry.len()).min(bytes.len());
            self.carry.extend_from_slice(&bytes[..take]);
            bytes = &bytes[take..];
            if self.carry.len() < size {
                return;
            }
            let block = std::mem::take(&mut self.carry);
            self.block(&block, out);
            self.carry = block;
            self.carry.clear();
        }
        let whole = bytes.len() / size * size;
        for block in bytes[..whole].chunks_exact(size) {
            self.block(block, out);
        }
        self.carry.extend_from_slice(&bytes[whole..]);
    }

    fn block(&self, bytes: &[u8], out: &mut Blocks) {
        match self.wire {
            2 => self.keep(&Self::packed::<2>(bytes), out),
            4 => self.keep(&Self::packed::<4>(bytes), out),
            8 => self.keep(&self.planes::<8>(bytes), out),
            16 => self.keep(&self.planes::<16>(bytes), out),
            32 => self.keep(&self.planes::<32>(bytes), out),
            _ => unreachable!("validated wire mode"),
        }
        if !self.groups.is_empty() {
            self.codes(bytes, usize::from(self.wire / 8), out);
        }
        out.count += 1;
    }

    fn keep(&self, words: &[u64], out: &mut Blocks) {
        for &channel in &self.channels {
            out.logic
                .extend_from_slice(&words[usize::from(channel)].to_le_bytes());
        }
    }

    /// The words of an 8-, 16- or 32-channel block, `W` channels of `W / 8` bytes a sample,
    /// up to the highest kept channel's byte.
    fn planes<const W: usize>(&self, bytes: &[u8]) -> [u64; W] {
        let stride = W / 8;
        let mut words = [0u64; W];
        for (plane, rows) in words.as_chunks_mut::<8>().0[..self.planes]
            .iter_mut()
            .enumerate()
        {
            // Row `g`: byte `plane` of samples 8g..8g + 8, then their channels as bytes, and
            // at last each channel's 64 samples.
            for (group, row) in rows.iter_mut().enumerate() {
                *row = transpose(gather(bytes, group * 8 * stride + plane, stride));
            }
            transpose_bytes(rows);
        }
        words
    }

    /// Each analog group's codes, sample by sample: its byte of each sample, read MSB first
    /// ([`crate::adc::code`]), eight samples at a time.
    fn codes(&self, bytes: &[u8], stride: usize, out: &mut Blocks) {
        let groups = self.groups.len();
        let start = out.codes.len();
        out.codes.resize(start + BLOCK * groups, 0);
        let codes = &mut out.codes[start..];
        for (index, &group) in self.groups.iter().enumerate() {
            for eight in 0..BLOCK / 8 {
                let bytes = gather(bytes, eight * 8 * stride + usize::from(group), stride);
                for (sample, byte) in reverse_bytes(bytes).to_le_bytes().into_iter().enumerate() {
                    codes[(eight * 8 + sample) * groups + index] = u16::from(byte);
                }
            }
        }
    }

    /// A block of 2- or 4-channel samples, `BITS` bits each with the earliest lowest: 16
    /// or 32 bytes.
    fn packed<const BITS: usize>(bytes: &[u8]) -> [u64; 4] {
        let mut words = [0u64; 4];
        for (channel, word) in words[..BITS].iter_mut().enumerate() {
            for (part, raw) in bytes.as_chunks::<8>().0.iter().enumerate() {
                let bits = packed_channel::<BITS>(u64::from_le_bytes(*raw) >> channel);
                *word |= bits << (part * (64 / BITS));
            }
        }
        words
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reference;

    fn naive(samples: &[u32], channels: &[u8]) -> Vec<u64> {
        samples
            .as_chunks::<64>()
            .0
            .iter()
            .flat_map(|block| {
                channels.iter().map(|&channel| {
                    block.iter().enumerate().fold(0, |word, (tick, sample)| {
                        word | u64::from(sample >> channel & 1) << tick
                    })
                })
            })
            .collect()
    }

    #[test]
    fn transposes_every_wire_mode_across_every_transfer_boundary() {
        for (wire, head, masks) in [
            (2, 0, &[0b11, 0b10][..]),
            (4, 4, &[0xf, 0x9, 0x4]),
            (4, 0, &[0xf]),
            (8, 4, &[0xff, 0x81]),
            (8, 0, &[0x10]),
            (16, 4, &[0xffff, 0x8001, 0x0100]),
            (16, 0, &[0x55aa]),
            (32, 0, &[u32::MAX, 0x8000_0001, 0x00ff_0000, 1 << 31]),
        ] {
            let samples: Vec<u32> = (0..320u32)
                .map(|n| n.wrapping_mul(0x9e37_79b9) & (u32::MAX >> (32 - wire)))
                .collect();
            let mut bytes = vec![0xff; head];
            bytes.extend(reference::encode(wire, &samples));
            for &mask in masks {
                let channels: Vec<u8> = (0..32).filter(|c| mask & (1 << c) != 0).collect();
                let want = naive(&samples, &channels);
                for cut in [1, 3, 7, 16, 31, 64, 100, 129, bytes.len()] {
                    let mut assembler = Assembler::new(wire, head, mask, Vec::new());
                    let mut blocks = Blocks::default();
                    let (mut got, mut count) = (Vec::new(), 0);
                    for part in bytes.chunks(cut) {
                        assembler.push(part, &mut blocks);
                        got.extend(
                            blocks
                                .logic
                                .as_chunks::<8>()
                                .0
                                .iter()
                                .map(|&word| u64::from_le_bytes(word)),
                        );
                        count += blocks.count;
                    }
                    assert_eq!(got, want, "{wire} wire channels, mask {mask:x}, cut {cut}");
                    assert_eq!(count, samples.len() / 64);
                }
            }
        }
    }

    #[test]
    fn reverses_eight_bytes_as_the_adc_reads_each() {
        let bytes: u64 = 0x0123_4567_89ab_cdef;
        let each = bytes.to_le_bytes().map(|byte| crate::adc::code(byte) as u8);
        assert_eq!(reverse_bytes(bytes), u64::from_le_bytes(each));
    }

    #[test]
    fn reads_analog_codes_msb_first_from_their_inputs() {
        // Inputs 0–7 carry a code wired MSB first; inputs 16–23 another.
        let samples: Vec<u32> = (0u32..128)
            .map(|n| {
                u32::from((n as u8).reverse_bits()) | u32::from((!n as u8).reverse_bits()) << 16
            })
            .collect();
        let mut assembler = Assembler::new(32, 0, 0, vec![0, 2]);
        let mut blocks = Blocks::default();
        assembler.push(&reference::encode(32, &samples), &mut blocks);
        let codes: Vec<u16> = (0..128).flat_map(|n| [n, 255 - n]).collect();
        assert_eq!(blocks.codes, codes);
        assert!(blocks.logic.is_empty());
        assert_eq!(blocks.count, 2);
    }
}
