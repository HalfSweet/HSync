// SPDX-License-Identifier: GPL-3.0-or-later
// Wire formats follow sipeed/libsigrok, slogic-dev, commit
// 0d728b3e1226a61738c99c3333ad16d5c2814728:
// src/hardware/sipeed-slogic-analyzer/slogic/{slogic16u3.c,slogic32u3.c,slogic_data.c}.
// Copyright (C) 2023-2025 Shenzhen Sipeed Technology Co., Ltd.
// Combo 8's two-bit sample order follows slogic_data.c at
// bf6b346e597d112f48102fe6a7b2318c6d7b49e2 (slogic_unpack_range).
//! Borrowed, width-specific sample views. No transfer-sized unpacked array is built.

use std::marker::PhantomData;

/// Native unit of a wire format and its pre-trigger history.
pub(crate) trait SampleWord: Copy + Default + Into<u32> + Send {
    const BYTES: usize;
    fn from_u32(value: u32) -> Self;
    fn read(bytes: &[u8]) -> Self;
    fn write(self, bytes: &mut [u8]);
}

macro_rules! word {
    ($($ty:ty),*) => {$(
        impl SampleWord for $ty {
            const BYTES: usize = size_of::<Self>();
            fn from_u32(value: u32) -> Self { value as Self }
            fn read(bytes: &[u8]) -> Self {
                Self::from_le_bytes(bytes.try_into().expect("one complete wire word"))
            }
            fn write(self, bytes: &mut [u8]) { bytes.copy_from_slice(&self.to_le_bytes()); }
        }
        impl WireFormat for $ty {
            type Word = Self;
            const BITS: usize = size_of::<Self>() * 8;
        }
    )*};
}

/// Selected from the physical wire width once per capture, not the enabled channel count.
pub(crate) trait WireFormat: Copy + Send + 'static {
    type Word: SampleWord;
    const BITS: usize;
    const PER_WORD: usize = Self::Word::BYTES * 8 / Self::BITS;
}

word!(u8, u16, u32);

/// Two or four physical-channel bits per sample, earliest sample in the low bits.
#[derive(Clone, Copy)]
pub(crate) struct Packed<const BITS: usize>;

impl<const BITS: usize> WireFormat for Packed<BITS> {
    type Word = u8;
    const BITS: usize = BITS;
}

/// A possibly sample-offset view of complete wire words; samples stay packed in the USB bytes.
#[derive(Clone, Copy)]
pub(crate) struct Samples<'a, F: WireFormat> {
    bytes: &'a [u8],
    start: usize,
    len: usize,
    format: PhantomData<F>,
}

impl<'a, F: WireFormat> Samples<'a, F> {
    fn new(bytes: &'a [u8]) -> Self {
        debug_assert!(bytes.len().is_multiple_of(F::Word::BYTES));
        Self {
            bytes,
            start: 0,
            len: bytes.len() * 8 / F::BITS,
            format: PhantomData,
        }
    }

    pub(crate) fn len(self) -> usize {
        self.len
    }

    pub(crate) fn slice(self, start: usize, end: usize) -> Self {
        assert!(start <= end && end <= self.len);
        Self {
            start: self.start + start,
            len: end - start,
            ..self
        }
    }

    /// Only trigger comparisons and incomplete output blocks need individual sample values.
    pub(crate) fn at(self, index: usize) -> F::Word {
        debug_assert!(index < self.len);
        let sample = self.start + index;
        let offset = sample / F::PER_WORD * F::Word::BYTES;
        let word = F::Word::read(&self.bytes[offset..offset + F::Word::BYTES]);
        if F::PER_WORD == 1 {
            word
        } else {
            F::Word::from_u32(
                (word.into() >> (sample % F::PER_WORD * F::BITS)) & ((1 << F::BITS) - 1),
            )
        }
    }

    /// Whole blocks can be transposed directly, without copying or expanding any samples.
    pub(crate) fn block(self) -> Option<&'a [u8]> {
        if self.len < 64 || !self.start.is_multiple_of(F::PER_WORD) {
            return None;
        }
        let offset = self.start / F::PER_WORD * F::Word::BYTES;
        Some(&self.bytes[offset..offset + 8 * F::BITS])
    }

    /// Align a whole block whose trigger-relative start lies inside a packed byte.
    pub(crate) fn copy_block(self, target: &mut [u8]) {
        debug_assert!(self.len >= 64 && target.len() == 8 * F::BITS);
        let offset = self.start / F::PER_WORD * F::Word::BYTES;
        let shift = (self.start % F::PER_WORD) * F::BITS;
        if shift == 0 {
            target.copy_from_slice(&self.bytes[offset..offset + target.len()]);
        } else {
            for (index, byte) in target.iter_mut().enumerate() {
                *byte = self.bytes[offset + index] >> shift
                    | self.bytes[offset + index + 1] << (8 - shift);
            }
        }
    }
}

/// Discards the firmware head once per RUN and carries only an incomplete native word.
/// Complete words are borrowed directly from the transfer, including packed 2/4-bit samples.
pub(crate) struct WireDecoder<F: WireFormat> {
    drop_left: usize,
    partial: [u8; 4],
    pending: usize,
    format: PhantomData<F>,
}

impl<F: WireFormat> WireDecoder<F> {
    pub(crate) fn new(head_bytes: usize) -> Self {
        Self {
            drop_left: head_bytes,
            partial: [0; 4],
            pending: 0,
            format: PhantomData,
        }
    }

    pub(crate) fn push(&mut self, bytes: &[u8], mut consume: impl FnMut(Samples<'_, F>)) {
        let skipped = self.drop_left.min(bytes.len());
        self.drop_left -= skipped;
        let mut bytes = &bytes[skipped..];
        let width = F::Word::BYTES;
        if self.pending > 0 {
            let take = (width - self.pending).min(bytes.len());
            self.partial[self.pending..self.pending + take].copy_from_slice(&bytes[..take]);
            self.pending += take;
            bytes = &bytes[take..];
            if self.pending < width {
                return;
            }
            consume(Samples::new(&self.partial[..width]));
            self.pending = 0;
        }
        let whole = bytes.len() / width * width;
        if whole != 0 {
            consume(Samples::new(&bytes[..whole]));
        }
        let rest = &bytes[whole..];
        self.partial[..rest.len()].copy_from_slice(rest);
        self.pending = rest.len();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check_borrowed<F: WireFormat>() {
        let bytes = [0x96; 1024];
        let mut decoder = WireDecoder::<F>::new(4);
        let mut calls = 0;
        decoder.push(&bytes, |samples| {
            calls += 1;
            assert_eq!(samples.bytes.as_ptr(), bytes[4..].as_ptr());
            assert_eq!(samples.len(), (bytes.len() - 4) * 8 / F::BITS);
        });
        assert_eq!(calls, 1);
    }

    #[test]
    fn complete_transfers_are_borrowed_without_an_expanded_sample_buffer() {
        check_borrowed::<Packed<2>>();
        check_borrowed::<Packed<4>>();
        check_borrowed::<u8>();
        check_borrowed::<u16>();
        check_borrowed::<u32>();
    }

    fn check_words<F: WireFormat>(bytes: &[u8], expected: &[u32]) {
        let mut input = vec![0xff; 4];
        input.extend_from_slice(bytes);
        for cut in 1..=input.len() {
            let mut decoder = WireDecoder::<F>::new(4);
            let mut actual = Vec::new();
            for part in input.chunks(cut) {
                decoder.push(part, |samples| {
                    actual.extend((0..samples.len()).map(|i| samples.at(i).into()));
                });
                decoder.push(&[], |_| panic!("empty transfer produced samples"));
            }
            assert_eq!(actual, expected, "{} bits, cut {cut}", F::BITS);
        }
    }

    #[test]
    fn preserves_bit_order_and_little_endian_words_across_partial_reads() {
        check_words::<Packed<2>>(&[0xe4, 0x1b], &[0, 1, 2, 3, 3, 2, 1, 0]);
        check_words::<Packed<4>>(&[0x21, 0xf8], &[1, 2, 8, 15]);
        check_words::<u8>(&[0x12, 0x80], &[0x12, 0x80]);
        check_words::<u16>(&[0x12, 0x80, 0x34, 0xf1], &[0x8012, 0xf134]);
        check_words::<u32>(&[0x12, 0x80, 0x34, 0xf1], &[0xf134_8012]);
    }
}
