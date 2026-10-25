//! Samples of one analog channel: the converter's codes, the scale that makes them volts, and
//! minimum/maximum summaries, so that a column of the waveform spanning millions of samples
//! costs a few lookups.

use std::collections::VecDeque;
use std::ops::Range;

use hsync_core::{AnalogScale, SampleIndex};

/// Samples per chunk of storage.
const CHUNK: usize = 1 << 16;
/// Samples per summary block within a chunk.
const BLOCK: usize = 1 << 10;

/// The voltages of the codes `low` and `high`, lower first (a negative step swaps them).
fn span(scale: AnalogScale, (low, high): (u16, u16)) -> (f32, f32) {
    let (a, b) = (scale.volts(low), scale.volts(high));
    if a <= b { (a, b) } else { (b, a) }
}

/// The least and greatest code of a run of samples.
fn extent(codes: &[u16]) -> (u16, u16) {
    codes
        .iter()
        .fold((u16::MAX, u16::MIN), |(low, high), &code| {
            (low.min(code), high.max(code))
        })
}

fn join(a: Option<(u16, u16)>, b: (u16, u16)) -> (u16, u16) {
    a.map_or(b, |a| (a.0.min(b.0), a.1.max(b.1)))
}

#[derive(Debug, Clone, Default)]
struct Chunk {
    codes: Vec<u16>,
    /// Extent of each [`BLOCK`] of `codes`; the last may be partial.
    blocks: Vec<(u16, u16)>,
}

impl Chunk {
    fn extend(&mut self, codes: &[u16]) {
        let first = self.codes.len() / BLOCK;
        self.codes.extend_from_slice(codes);
        self.blocks.truncate(first);
        self.blocks
            .extend(self.codes[first * BLOCK..].chunks(BLOCK).map(extent));
    }

    fn truncate(&mut self, len: usize) {
        self.codes.truncate(len);
        self.blocks.truncate(len / BLOCK);
        if !len.is_multiple_of(BLOCK) {
            let last = len / BLOCK * BLOCK;
            self.blocks.push(extent(&self.codes[last..]));
        }
    }

    /// Extent of `codes[range]`, `None` for an empty range.
    fn extent(&self, range: Range<usize>) -> Option<(u16, u16)> {
        if range.is_empty() {
            return None;
        }
        let (from, to) = (range.start, range.end);
        let first_whole = from.div_ceil(BLOCK);
        let last_whole = to / BLOCK;
        if first_whole >= last_whole {
            return Some(extent(&self.codes[from..to]));
        }
        let mut result = None;
        if from < first_whole * BLOCK {
            result = Some(extent(&self.codes[from..first_whole * BLOCK]));
        }
        for &block in &self.blocks[first_whole..last_whole] {
            result = Some(join(result, block));
        }
        if last_whole * BLOCK < to {
            result = Some(join(result, extent(&self.codes[last_whole * BLOCK..to])));
        }
        result
    }
}

/// Codes of one analog channel with summaries for drawing.
///
/// Samples are appended at the end; [`Self::drop_front`] forgets samples at the start, as loop
/// captures do.
#[derive(Debug, Clone)]
pub struct AnalogData {
    scale: AnalogScale,
    chunks: VecDeque<Chunk>,
    /// Stored samples before sample 0: dropped from the front, held until their chunk goes.
    head: usize,
    len: SampleIndex,
}

impl AnalogData {
    /// Creates an empty channel whose codes `scale` turns into volts.
    pub fn new(scale: AnalogScale) -> Self {
        Self {
            scale,
            chunks: VecDeque::new(),
            head: 0,
            len: 0,
        }
    }

    /// How the codes become volts.
    pub fn scale(&self) -> AnalogScale {
        self.scale
    }

    /// Number of samples.
    pub fn len(&self) -> SampleIndex {
        self.len
    }

    /// Whether the channel holds no samples.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Appends samples.
    pub fn extend(&mut self, mut codes: &[u16]) {
        self.len += codes.len() as u64;
        while !codes.is_empty() {
            if self
                .chunks
                .back()
                .is_none_or(|chunk| chunk.codes.len() == CHUNK)
            {
                self.chunks.push_back(Chunk::default());
            }
            let chunk = self.chunks.back_mut().expect("a chunk with room");
            let take = codes.len().min(CHUNK - chunk.codes.len());
            chunk.extend(&codes[..take]);
            codes = &codes[take..];
        }
    }

    /// Code of sample `index`.
    ///
    /// # Panics
    ///
    /// Panics if `index` is out of range.
    pub fn code(&self, index: SampleIndex) -> u16 {
        assert!(index < self.len, "sample {index} out of {}", self.len);
        let stored = self.head + index as usize;
        self.chunks[stored / CHUNK].codes[stored % CHUNK]
    }

    /// Voltage of sample `index`.
    ///
    /// # Panics
    ///
    /// Panics if `index` is out of range.
    pub fn volts(&self, index: SampleIndex) -> f32 {
        self.scale.volts(self.code(index))
    }

    /// Least and greatest code of samples `from..to`, clipped to the channel; `None` when that
    /// leaves no sample.
    pub fn extent(&self, from: SampleIndex, to: SampleIndex) -> Option<(u16, u16)> {
        let to = to.min(self.len);
        if from >= to {
            return None;
        }
        let from = self.head + from as usize;
        let to = self.head + to as usize;
        let mut result = None;
        for chunk in from / CHUNK..=(to - 1) / CHUNK {
            let base = chunk * CHUNK;
            let range = from.max(base) - base..to.min(base + CHUNK) - base;
            if let Some(part) = self.chunks[chunk].extent(range) {
                result = Some(join(result, part));
            }
        }
        result
    }

    /// Least and greatest voltage of samples `from..to`, as [`Self::extent`].
    pub fn volt_extent(&self, from: SampleIndex, to: SampleIndex) -> Option<(f32, f32)> {
        self.extent(from, to).map(|codes| span(self.scale, codes))
    }

    /// The least and greatest voltage of each of `bins` columns over `start..end`; column `i`
    /// covers `start + ⌊i·n/bins⌋ .. start + ⌊(i+1)·n/bins⌋` and is `None` beyond the samples.
    pub fn bins(
        &self,
        start: SampleIndex,
        end: SampleIndex,
        bins: usize,
    ) -> Vec<Option<(f32, f32)>> {
        let n = end.saturating_sub(start);
        (0..bins as u64)
            .map(|column| {
                let from = start + n * column / bins as u64;
                let to = start + n * (column + 1) / bins as u64;
                self.volt_extent(from, to.max(from + 1))
            })
            .collect()
    }

    /// Forgets the first `samples` samples.
    ///
    /// # Panics
    ///
    /// Panics if `samples` exceeds the channel.
    pub fn drop_front(&mut self, samples: SampleIndex) {
        assert!(samples <= self.len, "dropping {samples} of {}", self.len);
        self.len -= samples;
        self.head += samples as usize;
        while self.head >= CHUNK && self.chunks.len() > 1 {
            self.chunks.pop_front();
            self.head -= CHUNK;
        }
    }

    /// Lines the chunks up with sample 0 again after [`Self::drop_front`].
    pub fn compact(&mut self) {
        if self.head == 0 {
            return;
        }
        let codes: Vec<u16> = (0..self.len).map(|index| self.code(index)).collect();
        *self = Self::new(self.scale);
        self.extend(&codes);
    }

    /// Shortens the channel to `len` samples.
    pub fn truncate(&mut self, len: SampleIndex) {
        if len >= self.len {
            return;
        }
        let stored = self.head + len as usize;
        self.chunks.truncate(stored.div_ceil(CHUNK).max(1));
        let before = (self.chunks.len() - 1) * CHUNK;
        if let Some(last) = self.chunks.back_mut() {
            last.truncate(stored - before);
        }
        self.len = len;
    }

    /// Codes of samples `from..to` as a channel of their own.
    pub fn slice(&self, from: SampleIndex, to: SampleIndex) -> Self {
        let mut out = Self::new(self.scale);
        let to = to.min(self.len);
        let codes: Vec<u16> = (from.min(to)..to).map(|index| self.code(index)).collect();
        out.extend(&codes);
        out
    }

    /// Bytes of heap memory held by the codes and their summaries.
    pub fn heap_bytes(&self) -> usize {
        self.chunks
            .iter()
            .map(|chunk| {
                chunk.codes.capacity() * size_of::<u16>()
                    + chunk.blocks.capacity() * size_of::<(u16, u16)>()
            })
            .sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const UNIT: AnalogScale = AnalogScale {
        step: 1.0,
        offset: 0.0,
    };

    fn naive(codes: &[u16], from: usize, to: usize) -> Option<(u16, u16)> {
        let to = to.min(codes.len());
        (from < to).then(|| extent(&codes[from..to]))
    }

    /// A pseudo-random walk long enough to cross chunks.
    fn walk(len: usize) -> Vec<u16> {
        let mut state = 0x2545_f491_u32;
        let mut code = 2048_i32;
        (0..len)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                code = (code + (state % 7) as i32 - 3).clamp(0, 4095);
                code as u16
            })
            .collect()
    }

    #[test]
    fn extents_match_a_scan_across_blocks_and_chunks() {
        let codes = walk(3 * CHUNK + 777);
        let mut data = AnalogData::new(UNIT);
        for piece in codes.chunks(5000) {
            data.extend(piece);
        }
        assert_eq!(data.len(), codes.len() as u64);
        let points = [
            0,
            1,
            BLOCK - 1,
            BLOCK,
            BLOCK + 3,
            CHUNK - 1,
            CHUNK,
            CHUNK + BLOCK * 5 + 9,
        ];
        for &from in &points {
            for &to in &points {
                for to in [to, to + CHUNK, codes.len(), codes.len() + 10] {
                    assert_eq!(
                        data.extent(from as u64, to as u64),
                        naive(&codes, from, to),
                        "{from}..{to}"
                    );
                }
            }
        }
    }

    #[test]
    fn bins_cover_the_range_and_end_with_the_samples() {
        let codes: Vec<u16> = (0..1000).collect();
        let mut data = AnalogData::new(AnalogScale {
            step: 0.5,
            offset: -1.0,
        });
        data.extend(&codes);
        let bins = data.bins(0, 1200, 6);
        assert_eq!(bins[0], Some((-1.0, 98.5)));
        assert_eq!(bins[4], Some((399.0, 498.5)));
        assert_eq!(bins[5], None);
        // Fewer samples than columns: each column is a sample, some repeat.
        let bins = data.bins(10, 12, 4);
        assert_eq!(
            bins,
            [
                Some((4.0, 4.0)),
                Some((4.0, 4.0)),
                Some((4.5, 4.5)),
                Some((4.5, 4.5))
            ]
        );
    }

    #[test]
    fn a_negative_step_still_gives_the_lower_voltage_first() {
        let mut data = AnalogData::new(AnalogScale {
            step: -2.0,
            offset: 10.0,
        });
        data.extend(&[1, 4]);
        assert_eq!(data.volt_extent(0, 2), Some((2.0, 8.0)));
    }

    #[test]
    fn dropping_compacting_truncating_and_slicing_keep_the_samples() {
        let codes = walk(2 * CHUNK + 300);
        let mut data = AnalogData::new(UNIT);
        data.extend(&codes);
        data.drop_front(CHUNK as u64 + 64);
        let rest = &codes[CHUNK + 64..];
        assert_eq!(data.len(), rest.len() as u64);
        assert_eq!(data.code(0), rest[0]);
        assert_eq!(data.extent(0, 1 << 40), naive(rest, 0, rest.len()));
        data.extend(&[7, 8]);
        assert_eq!(data.code(rest.len() as u64 + 1), 8);
        data.compact();
        assert_eq!(data.head, 0);
        assert_eq!(data.code(0), rest[0]);
        data.truncate(CHUNK as u64 + 5);
        assert_eq!(data.len(), CHUNK as u64 + 5);
        assert_eq!(data.extent(0, 1 << 40), naive(rest, 0, CHUNK + 5));
        let slice = data.slice(10, 20);
        assert_eq!(slice.len(), 10);
        assert_eq!(slice.code(0), rest[10]);
    }
}
