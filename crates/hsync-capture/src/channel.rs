//! Samples of one logic channel and the queries the UI, measurements and decoders need.

use hsync_core::SampleIndex;

use crate::summary::Summary;
use crate::words::{CHUNK_WORDS, ChunkView, ChunkedWords};

/// One cell of a [`ChannelData::bins`] result: how the samples of a pixel column look.
pub mod cell {
    /// Level of the first sample in the bin.
    pub const FIRST_HIGH: u8 = 1 << 0;
    /// Level of the last sample in the bin.
    pub const LAST_HIGH: u8 = 1 << 1;
    /// The bin contains at least one transition.
    pub const TRANSITION: u8 = 1 << 2;
    /// The bin lies beyond the captured samples.
    pub const EMPTY: u8 = 1 << 3;
}

/// Edge counts over a range of samples.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct EdgeCount {
    /// Low to high transitions.
    pub rising: u64,
    /// High to low transitions.
    pub falling: u64,
}

/// The run of equal samples around a position.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pulse {
    /// Level of the run.
    pub high: bool,
    /// First sample of the run.
    pub start: SampleIndex,
    /// First sample after the run.
    pub end: SampleIndex,
    /// Whether `start` is a real edge rather than the start of the capture.
    pub start_is_edge: bool,
    /// Whether `end` is a real edge rather than the end of the capture.
    pub end_is_edge: bool,
}

/// Bit-packed samples of one channel with summaries for fast searches.
///
/// Sample `i` is bit `i % 64` of word `i / 64`. Words are appended whole; [`Self::truncate`]
/// can end the channel inside a word.
#[derive(Debug, Default, Clone)]
pub struct ChannelData {
    words: ChunkedWords,
    summary: Summary,
    len: SampleIndex,
}

impl ChannelData {
    /// Creates an empty channel.
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of samples.
    pub fn len(&self) -> SampleIndex {
        self.len
    }

    /// Whether the channel holds no samples.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Appends whole 64-sample words.
    pub fn append_words(&mut self, words: &[u64]) {
        assert!(
            self.len.is_multiple_of(64),
            "a truncated channel cannot be extended"
        );
        if words.is_empty() {
            return;
        }
        let first = self.words.len();
        let full_chunks_before = first / CHUNK_WORDS;
        self.words.extend(words);
        self.len += words.len() as u64 * 64;
        let stored = &self.words;
        self.summary
            .update(first, words.len(), 64, |index| stored.get(index));

        // Fold every chunk this append completed, if all its samples share one level.
        for chunk in full_chunks_before..self.words.len() / CHUNK_WORDS {
            if let Some(level) = self.summary.top_unit(chunk) {
                self.words.fold(chunk, level);
            }
        }
    }

    /// Appends bytes in `.dsl` block order: sample `8k + b` is bit `b` of byte `k`. The length
    /// must be a multiple of 8.
    pub fn append_bytes(&mut self, bytes: &[u8]) {
        let (words, rest) = bytes.as_chunks::<8>();
        assert!(rest.is_empty(), "logic blocks hold whole 64-sample words");
        let words: Vec<u64> = words.iter().map(|word| u64::from_le_bytes(*word)).collect();
        self.append_words(&words);
    }

    /// Shortens the channel to `len` samples.
    pub fn truncate(&mut self, len: SampleIndex) {
        if len >= self.len {
            return;
        }
        let words = usize::try_from(len.div_ceil(64)).unwrap_or(usize::MAX);
        self.words.truncate(words);
        self.len = len;
        let valid = match len % 64 {
            0 => 64,
            bits => bits as u32,
        };
        let stored = &self.words;
        let mut summary = Summary::default();
        summary.update(0, words, valid, |index| stored.get(index));
        self.summary = summary;
    }

    /// Level of sample `index`.
    ///
    /// # Panics
    ///
    /// Panics if `index` is out of range.
    pub fn sample(&self, index: SampleIndex) -> bool {
        assert!(index < self.len, "sample {index} out of {}", self.len);
        self.words.get((index / 64) as usize) >> (index % 64) & 1 != 0
    }

    /// Number of 64-sample words, counting a partly filled last one.
    pub fn word_count(&self) -> usize {
        self.words.len()
    }

    /// Word `index`: sample `64 · index + j` is bit `j`. Bits past the end of the channel are
    /// unspecified.
    ///
    /// # Panics
    ///
    /// Panics if `index` is out of range.
    pub fn word(&self, index: usize) -> u64 {
        assert!(
            index < self.words.len(),
            "word {index} out of {}",
            self.words.len()
        );
        self.words.get(index)
    }

    /// Mask of the valid samples in word `word`.
    fn valid_mask(&self, word: usize) -> u64 {
        let first = word as u64 * 64;
        let valid = self.len.saturating_sub(first);
        if valid >= 64 { !0 } else { (1u64 << valid) - 1 }
    }

    /// The first sample after `index` whose level differs from sample `index`.
    pub fn next_change(&self, index: SampleIndex) -> Option<SampleIndex> {
        if index >= self.len {
            return None;
        }
        let level = self.sample(index);
        let flip = |word: u64| if level { !word } else { word };
        let word = (index / 64) as usize;
        let after = if index % 64 == 63 {
            0
        } else {
            !0u64 << (index % 64 + 1)
        };
        let differing = flip(self.words.get(word)) & after & self.valid_mask(word);
        if differing != 0 {
            return Some(word as u64 * 64 + u64::from(differing.trailing_zeros()));
        }
        let next = self.summary.next_word_not(word + 1, level)?;
        let differing = flip(self.words.get(next)) & self.valid_mask(next);
        debug_assert_ne!(differing, 0, "summary points at a uniform word");
        Some(next as u64 * 64 + u64::from(differing.trailing_zeros()))
    }

    /// The last sample before `index` whose level differs from sample `index`.
    pub fn prev_change(&self, index: SampleIndex) -> Option<SampleIndex> {
        if index >= self.len || index == 0 {
            return None;
        }
        let level = self.sample(index);
        let flip = |word: u64| if level { !word } else { word };
        let word = (index / 64) as usize;
        let before = (1u64 << (index % 64)) - 1;
        let differing = flip(self.words.get(word)) & before;
        if differing != 0 {
            return Some(word as u64 * 64 + 63 - u64::from(differing.leading_zeros()));
        }
        let prev = self.summary.prev_word_not(word.checked_sub(1)?, level)?;
        let differing = flip(self.words.get(prev)) & self.valid_mask(prev);
        debug_assert_ne!(differing, 0, "summary points at a uniform word");
        Some(prev as u64 * 64 + 63 - u64::from(differing.leading_zeros()))
    }

    /// The run of equal samples containing `index`.
    pub fn pulse(&self, index: SampleIndex) -> Option<Pulse> {
        if index >= self.len {
            return None;
        }
        let prev = self.prev_change(index);
        let next = self.next_change(index);
        Some(Pulse {
            high: self.sample(index),
            start: prev.map_or(0, |at| at + 1),
            end: next.unwrap_or(self.len),
            start_is_edge: prev.is_some(),
            end_is_edge: next.is_some(),
        })
    }

    /// Summarises samples `start..end` in `bins` equal columns, one [`cell`] byte each.
    ///
    /// Bin `i` covers `start + ⌊i·n/bins⌋ .. start + ⌊(i+1)·n/bins⌋` with `n = end - start`.
    pub fn bins(&self, start: SampleIndex, end: SampleIndex, bins: usize) -> Vec<u8> {
        let mut out = vec![cell::EMPTY; bins];
        if bins == 0 || end <= start {
            return out;
        }
        let span = u128::from(end - start);
        let bound = |i: usize| start + (span * i as u128 / bins as u128) as u64;
        // The run that contains the current position: its level and where it ends.
        let mut run: Option<(SampleIndex, bool, Option<SampleIndex>)> = None;
        for (i, out_cell) in out.iter_mut().enumerate() {
            let (lo, hi) = (bound(i), bound(i + 1).min(self.len));
            if lo >= hi {
                continue;
            }
            let (level, change) = match run {
                Some((from, level, change)) if from <= lo && change.is_none_or(|at| at > lo) => {
                    (level, change)
                }
                _ => {
                    let level = self.sample(lo);
                    let change = self.next_change(lo);
                    run = Some((lo, level, change));
                    (level, change)
                }
            };
            let mut value = if level { cell::FIRST_HIGH } else { 0 };
            match change {
                Some(at) if at < hi => {
                    value |= cell::TRANSITION;
                    if self.sample(hi - 1) {
                        value |= cell::LAST_HIGH;
                    }
                }
                _ => {
                    if level {
                        value |= cell::LAST_HIGH;
                    }
                }
            }
            *out_cell = value;
        }
        out
    }

    /// Counts rising and falling edges between consecutive samples in `start..end`.
    pub fn count_edges(&self, start: SampleIndex, end: SampleIndex) -> EdgeCount {
        let end = end.min(self.len);
        let mut count = EdgeCount::default();
        if end <= start + 1 {
            return count;
        }
        let mut position = start;
        let mut level = self.sample(start);
        while let Some(at) = self.next_change(position) {
            if at >= end {
                break;
            }
            let word = (at / 64) as usize;
            let bit = at % 64;
            // Count every transition left in this word in one go.
            let value = self.words.get(word);
            let previous = (value << 1) | u64::from(level);
            let transitions = (value ^ previous) & (!0u64 << bit) & self.valid_mask(word);
            let limit = if end / 64 == word as u64 {
                (1u64 << (end % 64)) - 1
            } else {
                !0
            };
            let transitions = transitions & limit;
            count.rising += u64::from((transitions & value).count_ones());
            count.falling += u64::from((transitions & !value).count_ones());
            let last = (word as u64 * 64 + 63).min(end - 1);
            level = self.sample(last);
            position = last;
        }
        count
    }

    /// Number of 2 MiB chunks backing the channel.
    pub fn chunk_count(&self) -> usize {
        self.words.chunk_count()
    }

    /// Contents of chunk `index`; the last chunk may be shorter than a full chunk.
    pub fn chunk(&self, index: usize) -> ChunkView<'_> {
        self.words.chunk(index)
    }

    /// Bytes of heap memory held by sample data.
    pub fn heap_bytes(&self) -> usize {
        self.words.heap_bytes()
    }
}
