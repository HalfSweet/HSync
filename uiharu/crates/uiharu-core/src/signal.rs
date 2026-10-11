// SPDX-License-Identifier: Apache-2.0

//! Logic samples as decoders read them: blocks of bit-packed samples, and the cursor that
//! walks a stream of blocks looking for edges and reading levels.
//!
//! The cursor remembers the levels just before its position across blocks, so an edge on the
//! first sample of a block is found like any other: where the samples are cut into blocks does
//! not change what a decoder sees. Only the start of the stream and the first sample after a
//! gap have no earlier sample to compare with.

use std::fmt;
use std::future::poll_fn;
use std::sync::Arc;
use std::task::Poll;

use crate::node::{Inbox, Wait};
use crate::schema::{EventSchema, SchemaId};
use crate::stream::Item;
use crate::time::{Tick, TickRange};

/// The most channels a source may have.
pub const MAX_CHANNELS: u8 = 64;

/// The schema of a source's output: blocks of logic samples.
pub const LOGIC_SAMPLES: SchemaId = SchemaId::new("logic.samples", 1);

impl EventSchema for Arc<LogicBlock> {
    const SCHEMA: SchemaId = LOGIC_SAMPLES;
}

/// A set of channels: bit `i` stands for channel `i`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct ChannelMask(pub u64);

impl ChannelMask {
    /// No channels.
    pub const EMPTY: ChannelMask = ChannelMask(0);

    /// Channel `channel` alone.
    pub const fn one(channel: u8) -> Self {
        ChannelMask(1u64 << (channel & 63))
    }

    /// The set with `channel` added.
    pub const fn with(self, channel: u8) -> Self {
        ChannelMask(self.0 | (1u64 << (channel & 63)))
    }

    /// Whether `channel` is in the set.
    pub const fn has(self, channel: u8) -> bool {
        self.0 & (1u64 << (channel & 63)) != 0
    }

    /// Whether the set is empty.
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// The channels in the set, lowest first.
    pub fn iter(self) -> impl Iterator<Item = u8> {
        let mut bits = self.0;
        std::iter::from_fn(move || {
            if bits == 0 {
                return None;
            }
            let channel = bits.trailing_zeros() as u8;
            bits &= bits - 1;
            Some(channel)
        })
    }
}

/// The levels of every channel at one tick: bit `i` set means channel `i` is high.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Levels(pub u64);

impl Levels {
    /// Whether `channel` is high.
    pub const fn high(self, channel: u8) -> bool {
        self.0 & (1u64 << (channel & 63)) != 0
    }

    /// The channels, among `mask`, whose level differs from `other`.
    pub const fn changed_from(self, other: Levels, mask: ChannelMask) -> ChannelMask {
        ChannelMask((self.0 ^ other.0) & mask.0)
    }
}

/// A block of samples: `len` ticks from `start` on `channels` channels, bit-packed per
/// channel, least significant bit first.
#[derive(Clone, PartialEq, Eq)]
pub struct LogicBlock {
    start: Tick,
    len: u64,
    channels: u8,
    /// Words per channel.
    stride: usize,
    /// Channel `c` holds words `c * stride .. (c + 1) * stride`.
    words: Vec<u64>,
}

/// Why samples do not make a block.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum BlockError {
    /// A block holds at least one sample.
    #[error("a block holds at least one sample")]
    Empty,
    /// More channels than a block can hold.
    #[error("{0} channels: a block holds at most {MAX_CHANNELS}")]
    TooManyChannels(u8),
    /// The words given do not match the channel count and length.
    #[error("{found} words given for {channels} channels of {len} samples ({expected} needed)")]
    WrongSize {
        /// The channels of the block.
        channels: u8,
        /// Its samples.
        len: u64,
        /// The words that would hold them.
        expected: usize,
        /// The words given.
        found: usize,
    },
}

impl LogicBlock {
    /// A block from its words, `channels * ⌈len / 64⌉` of them, channel by channel; bits past
    /// `len` in a channel's last word are ignored.
    pub fn from_words(
        start: Tick,
        len: u64,
        channels: u8,
        mut words: Vec<u64>,
    ) -> Result<Self, BlockError> {
        if len == 0 {
            return Err(BlockError::Empty);
        }
        if channels > MAX_CHANNELS {
            return Err(BlockError::TooManyChannels(channels));
        }
        let stride = usize::try_from(len.div_ceil(64)).map_err(|_| BlockError::WrongSize {
            channels,
            len,
            expected: usize::MAX,
            found: words.len(),
        })?;
        let expected = stride * usize::from(channels);
        if words.len() != expected {
            return Err(BlockError::WrongSize {
                channels,
                len,
                expected,
                found: words.len(),
            });
        }
        // The padding bits must be clear: the edge scan relies on it.
        let valid = len % 64;
        if valid != 0 {
            let keep = (1u64 << valid) - 1;
            for channel in 0..usize::from(channels) {
                words[channel * stride + stride - 1] &= keep;
            }
        }
        Ok(Self {
            start,
            len,
            channels,
            stride,
            words,
        })
    }

    /// A block from samples interleaved sample by sample, `⌈channels / 8⌉` bytes each, bit `i`
    /// of byte `j` being channel `8 j + i`: the layout of raw binary exports.
    pub fn from_interleaved(start: Tick, channels: u8, bytes: &[u8]) -> Result<Self, BlockError> {
        if channels > MAX_CHANNELS {
            return Err(BlockError::TooManyChannels(channels));
        }
        let unit = usize::from(channels).div_ceil(8).max(1);
        let len = bytes.len() / unit;
        let mut builder = LogicBlockBuilder::new(start, channels);
        for sample in bytes.chunks_exact(unit).take(len) {
            let mut levels = 0u64;
            for (group, byte) in sample.iter().enumerate() {
                levels |= u64::from(*byte) << (8 * group);
            }
            builder.push(Levels(levels));
        }
        builder.finish()
    }

    /// The first tick of the block.
    pub fn start(&self) -> Tick {
        self.start
    }

    /// The samples in the block.
    pub fn len(&self) -> u64 {
        self.len
    }

    /// Whether the block holds no sample; never, for a block that was built.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The first tick after the block.
    pub fn end(&self) -> Tick {
        self.start.offset(self.len)
    }

    /// The ticks of the block.
    pub fn range(&self) -> TickRange {
        TickRange::new(self.start, self.end())
    }

    /// The channels of the block.
    pub fn channels(&self) -> u8 {
        self.channels
    }

    /// Whether `tick` is in the block.
    pub fn contains(&self, tick: Tick) -> bool {
        self.range().contains(tick)
    }

    /// The words of `channel`, least significant bit first.
    pub fn words(&self, channel: u8) -> &[u64] {
        let channel = usize::from(channel);
        &self.words[channel * self.stride..(channel + 1) * self.stride]
    }

    fn word(&self, channel: u8, index: usize) -> u64 {
        self.words[usize::from(channel) * self.stride + index]
    }

    /// The level of `channel` at `tick`, if the block holds it.
    pub fn level(&self, channel: u8, tick: Tick) -> Option<bool> {
        if channel >= self.channels || !self.contains(tick) {
            return None;
        }
        let offset = self.start.until(tick);
        let word = self.word(channel, (offset / 64) as usize);
        Some(word >> (offset % 64) & 1 != 0)
    }

    /// The levels of every channel at `tick`, if the block holds it.
    pub fn levels(&self, tick: Tick) -> Option<Levels> {
        if !self.contains(tick) {
            return None;
        }
        let offset = self.start.until(tick);
        let (index, bit) = ((offset / 64) as usize, offset % 64);
        let mut levels = 0u64;
        for channel in 0..self.channels {
            levels |= (self.word(channel, index) >> bit & 1) << channel;
        }
        Some(Levels(levels))
    }

    /// The part of the block covering `range`, or `None` when `range` reaches outside it or
    /// is empty.
    pub fn slice(&self, range: TickRange) -> Option<LogicBlock> {
        if range.is_empty() || range.start < self.start || range.end > self.end() {
            return None;
        }
        let mut builder = LogicBlockBuilder::new(range.start, self.channels);
        let mut tick = range.start;
        while tick < range.end {
            builder.push(self.levels(tick)?);
            tick = tick.next();
        }
        builder.finish().ok()
    }

    /// The first tick at or after `from` whose levels, on the channels of `mask`, differ from
    /// the tick before it, taking `prev` as the levels of `from - 1`; `from` must be in the
    /// block. Returns the tick and the channels that changed.
    pub fn next_change(
        &self,
        mask: ChannelMask,
        from: Tick,
        prev: Levels,
    ) -> Option<(Tick, ChannelMask)> {
        if !self.contains(from) || mask.is_empty() {
            return None;
        }
        let first = self.start.until(from) as usize;
        let (first_word, shift) = (first / 64, first % 64);
        for index in first_word..self.stride {
            let mut diff = 0u64;
            for channel in mask.iter() {
                if channel >= self.channels {
                    continue;
                }
                let current = self.word(channel, index);
                // Each bit against the one before it; `prev` stands in for the tick before
                // `from`, and the last bit of the previous word for the first of this one.
                let before = if index == first_word {
                    ((current << 1) & !(1u64 << shift)) | (u64::from(prev.high(channel)) << shift)
                } else {
                    (current << 1) | (self.word(channel, index - 1) >> 63)
                };
                diff |= current ^ before;
            }
            if index == first_word {
                diff &= u64::MAX << shift;
            }
            if index == self.stride - 1 {
                let valid = self.len - 64 * index as u64;
                if valid < 64 {
                    diff &= (1u64 << valid) - 1;
                }
            }
            if diff != 0 {
                let tick = self
                    .start
                    .offset(64 * index as u64 + u64::from(diff.trailing_zeros()));
                let before = if tick == from {
                    prev
                } else {
                    self.levels(Tick(tick.0 - 1))?
                };
                let changed = self.levels(tick)?.changed_from(before, mask);
                return Some((tick, changed));
            }
        }
        None
    }
}

impl fmt::Debug for LogicBlock {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LogicBlock")
            .field("start", &self.start)
            .field("len", &self.len)
            .field("channels", &self.channels)
            .finish_non_exhaustive()
    }
}

/// Builds a [`LogicBlock`] sample by sample.
#[derive(Clone, Debug)]
pub struct LogicBlockBuilder {
    start: Tick,
    channels: u8,
    len: u64,
    /// Words per channel, grown as samples come.
    lanes: Vec<Vec<u64>>,
}

impl LogicBlockBuilder {
    /// An empty block of `channels` channels starting at `start`.
    pub fn new(start: Tick, channels: u8) -> Self {
        let channels = channels.min(MAX_CHANNELS);
        Self {
            start,
            channels,
            len: 0,
            lanes: vec![Vec::new(); usize::from(channels)],
        }
    }

    /// The samples pushed so far.
    pub fn len(&self) -> u64 {
        self.len
    }

    /// Whether no sample was pushed.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The tick the next sample will have.
    pub fn next_tick(&self) -> Tick {
        self.start.offset(self.len)
    }

    /// Appends one sample.
    pub fn push(&mut self, levels: Levels) {
        let bit = self.len % 64;
        if bit == 0 {
            for lane in &mut self.lanes {
                lane.push(0);
            }
        }
        for (channel, lane) in self.lanes.iter_mut().enumerate() {
            if levels.0 >> channel & 1 != 0
                && let Some(word) = lane.last_mut()
            {
                *word |= 1u64 << bit;
            }
        }
        self.len += 1;
    }

    /// Appends `count` samples with the same levels.
    pub fn push_repeated(&mut self, levels: Levels, count: u64) {
        for _ in 0..count {
            self.push(levels);
        }
    }

    /// The block; an error when no sample was pushed.
    pub fn finish(self) -> Result<LogicBlock, BlockError> {
        let stride = self.len.div_ceil(64) as usize;
        let mut words = Vec::with_capacity(stride * usize::from(self.channels));
        for lane in self.lanes {
            words.extend(lane);
        }
        LogicBlock::from_words(self.start, self.len, self.channels, words)
    }
}

/// What a source feeds into a pipeline.
#[derive(Clone, Debug)]
pub enum SignalItem {
    /// The next samples; they follow the previous block or the previous gap directly.
    Data(Arc<LogicBlock>),
    /// The source's clock has reached this tick; no sample before it is still to come.
    Progress(Tick),
    /// These samples were lost; the next block starts at the end of the gap.
    Gap(TickRange),
    /// The source has stopped for good at this tick.
    End(Tick),
}

/// What a wait for an edge finds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    /// A tick where some watched channels changed.
    At {
        /// The tick of the change.
        tick: Tick,
        /// The watched channels whose level differs from the tick before.
        changed: ChannelMask,
        /// The levels of every channel at `tick`.
        levels: Levels,
    },
    /// Samples are missing; the cursor now stands at the end of the gap.
    Gap(TickRange),
    /// No change up to this tick, which the source has reached.
    Progress(Tick),
    /// The samples have ended.
    End(Tick),
}

/// What reading the levels at the cursor finds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sample {
    /// The levels at the cursor's tick.
    At {
        /// The tick read.
        tick: Tick,
        /// The levels of every channel.
        levels: Levels,
    },
    /// Samples are missing; the cursor now stands at the end of the gap.
    Gap(TickRange),
    /// The source has reached this tick without delivering the cursor's sample yet.
    Progress(Tick),
    /// The samples have ended.
    End(Tick),
}

/// A decoder's view of a source: a position in its samples, with the levels just before it.
pub struct SignalCursor {
    inbox: Inbox<Arc<LogicBlock>>,
    /// The block the cursor stands in, until it is used up.
    block: Option<Arc<LogicBlock>>,
    /// The next tick to look at.
    pos: Tick,
    /// The levels at `pos - 1`, when known.
    prev: Levels,
    /// False at the start and right after a gap: the tick before `pos` was never seen.
    prev_known: bool,
}

impl fmt::Debug for SignalCursor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SignalCursor")
            .field("pos", &self.pos)
            .field("prev", &self.prev)
            .field("prev_known", &self.prev_known)
            .finish_non_exhaustive()
    }
}

impl SignalCursor {
    pub(crate) fn new(inbox: Inbox<Arc<LogicBlock>>) -> Self {
        Self {
            inbox,
            block: None,
            pos: Tick::ZERO,
            prev: Levels(0),
            prev_known: false,
        }
    }

    /// The next tick the cursor will look at.
    pub fn position(&self) -> Tick {
        self.pos
    }

    /// Takes the next item from the source if one is waiting; `Pending` sets the node to wait
    /// for input. A block is kept as the current block; the others are returned.
    fn poll_source(&mut self) -> Poll<Option<Item<Arc<LogicBlock>>>> {
        let taken = match self.inbox.poll_peek() {
            Poll::Pending => return Poll::Pending,
            Poll::Ready(item) => match &*item {
                Item::Event(event) => {
                    self.block = Some(Arc::clone(&event.payload));
                    self.pos = event.payload.start();
                    None
                }
                Item::Gap(range) => Some(Item::Gap(*range)),
                Item::Progress(tick) => Some(Item::Progress(*tick)),
                Item::End(tick) => Some(Item::End(*tick)),
            },
        };
        // The end stays in the queue: every later wait finds it again.
        if !matches!(taken, Some(Item::End(_))) {
            self.inbox.advance();
        }
        if let Some(Item::Gap(range)) = taken {
            self.pos = range.end;
            self.prev_known = false;
        }
        Poll::Ready(taken)
    }

    fn poll_change(&mut self, mask: ChannelMask) -> Poll<Change> {
        let cx = self.inbox.cx().clone();
        if !cx.has_budget() {
            cx.wait(Wait::Budget);
            return Poll::Pending;
        }
        loop {
            if let Some(block) = self.block.clone() {
                if self.pos < block.end() {
                    let (from, prev) = if self.prev_known {
                        (self.pos, self.prev)
                    } else {
                        // Nothing before the first sample: it cannot be an edge.
                        let first = block.levels(self.pos).unwrap_or_default();
                        (self.pos.next(), first)
                    };
                    if from < block.end()
                        && let Some((tick, changed)) = block.next_change(mask, from, prev)
                    {
                        let levels = block.levels(tick).unwrap_or_default();
                        self.prev = levels;
                        self.prev_known = true;
                        self.pos = tick.next();
                        cx.spend();
                        return Poll::Ready(Change::At {
                            tick,
                            changed,
                            levels,
                        });
                    }
                    let last = Tick(block.end().0 - 1);
                    self.prev = block.levels(last).unwrap_or_default();
                    self.prev_known = true;
                    self.pos = block.end();
                }
                self.block = None;
            }
            match self.poll_source() {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(None) => continue,
                Poll::Ready(Some(Item::Gap(range))) => {
                    cx.spend();
                    return Poll::Ready(Change::Gap(range));
                }
                Poll::Ready(Some(Item::Progress(tick))) => {
                    cx.spend();
                    return Poll::Ready(Change::Progress(tick));
                }
                Poll::Ready(Some(Item::End(tick))) => {
                    cx.spend();
                    return Poll::Ready(Change::End(tick));
                }
                Poll::Ready(Some(Item::Event(_))) => unreachable!("blocks are kept, not returned"),
            }
        }
    }

    fn poll_sample(&mut self) -> Poll<Sample> {
        let cx = self.inbox.cx().clone();
        if !cx.has_budget() {
            cx.wait(Wait::Budget);
            return Poll::Pending;
        }
        loop {
            if let Some(block) = &self.block {
                if let Some(levels) = block.levels(self.pos) {
                    cx.spend();
                    return Poll::Ready(Sample::At {
                        tick: self.pos,
                        levels,
                    });
                }
                self.block = None;
            }
            match self.poll_source() {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(None) => continue,
                Poll::Ready(Some(Item::Gap(range))) => {
                    cx.spend();
                    return Poll::Ready(Sample::Gap(range));
                }
                Poll::Ready(Some(Item::Progress(tick))) => {
                    cx.spend();
                    return Poll::Ready(Sample::Progress(tick));
                }
                Poll::Ready(Some(Item::End(tick))) => {
                    cx.spend();
                    return Poll::Ready(Sample::End(tick));
                }
                Poll::Ready(Some(Item::Event(_))) => unreachable!("blocks are kept, not returned"),
            }
        }
    }

    /// Waits for the next tick at or after the cursor where a channel of `mask` changes level,
    /// and moves past it. Channels that change at the same tick are reported together.
    pub fn next_change(&mut self, mask: ChannelMask) -> impl Future<Output = Change> + '_ {
        poll_fn(move |_| self.poll_change(mask))
    }

    /// Waits for the sample at the cursor and reads every channel's level there, without
    /// moving the cursor.
    pub fn sample(&mut self) -> impl Future<Output = Sample> + '_ {
        poll_fn(move |_| self.poll_sample())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block(start: u64, samples: &[u64], channels: u8) -> LogicBlock {
        let mut builder = LogicBlockBuilder::new(Tick(start), channels);
        for levels in samples {
            builder.push(Levels(*levels));
        }
        builder.finish().expect("a block")
    }

    #[test]
    fn builder_packs_bits_per_channel() {
        let block = block(10, &[0b01, 0b11, 0b10, 0b00], 2);
        assert_eq!(block.len(), 4);
        assert_eq!(block.end(), Tick(14));
        assert_eq!(block.words(0), &[0b0011]);
        assert_eq!(block.words(1), &[0b0110]);
        assert_eq!(block.level(0, Tick(10)), Some(true));
        assert_eq!(block.level(1, Tick(10)), Some(false));
        assert_eq!(block.level(0, Tick(14)), None);
        assert_eq!(block.levels(Tick(11)), Some(Levels(0b11)));
    }

    #[test]
    fn interleaved_bytes_follow_the_export_layout() {
        let block =
            LogicBlock::from_interleaved(Tick(0), 12, &[0x01, 0x08, 0xff, 0x0f]).expect("a block");
        assert_eq!(block.len(), 2);
        assert_eq!(block.levels(Tick(0)), Some(Levels(0x801)));
        assert_eq!(block.levels(Tick(1)), Some(Levels(0xfff)));
        let four = LogicBlock::from_interleaved(Tick(5), 4, &[0x3, 0x0, 0xc]).expect("a block");
        assert_eq!(four.len(), 3);
        assert_eq!(four.levels(Tick(7)), Some(Levels(0xc)));
    }

    #[test]
    fn rejects_bad_blocks() {
        assert_eq!(
            LogicBlockBuilder::new(Tick(0), 1)
                .finish()
                .expect_err("fails"),
            BlockError::Empty
        );
        assert!(matches!(
            LogicBlock::from_words(Tick(0), 65, 1, vec![0]),
            Err(BlockError::WrongSize {
                expected: 2,
                found: 1,
                ..
            })
        ));
        assert_eq!(
            LogicBlock::from_words(Tick(0), 1, 65, vec![]).expect_err("fails"),
            BlockError::TooManyChannels(65)
        );
    }

    #[test]
    fn finds_changes_within_and_across_words() {
        let mut samples = vec![0u64; 100];
        samples[3] = 1;
        samples[4] = 1;
        samples[64] = 2;
        samples[99] = 3;
        let block = block(0, &samples, 2);
        let both = ChannelMask::one(0).with(1);
        assert_eq!(
            block.next_change(both, Tick(0), Levels(0)),
            Some((Tick(3), ChannelMask::one(0)))
        );
        assert_eq!(
            block.next_change(both, Tick(4), Levels(1)),
            Some((Tick(5), ChannelMask::one(0)))
        );
        assert_eq!(
            block.next_change(both, Tick(6), Levels(0)),
            Some((Tick(64), ChannelMask::one(1)))
        );
        assert_eq!(
            block.next_change(ChannelMask::one(0), Tick(6), Levels(0)),
            Some((Tick(99), ChannelMask::one(0)))
        );
        assert_eq!(block.next_change(both, Tick(99), Levels(0b11)), None);
        assert_eq!(block.next_change(both, Tick(100), Levels(0)), None);
        // `prev` stands for the tick before `from`: a difference there is a change at `from`.
        assert_eq!(
            block.next_change(both, Tick(0), Levels(0b11)),
            Some((Tick(0), both))
        );
    }

    #[test]
    fn padding_bits_are_no_change() {
        let block = block(0, &[1, 1, 1], 1);
        assert_eq!(
            block.next_change(ChannelMask::one(0), Tick(1), Levels(1)),
            None
        );
    }

    #[test]
    fn slices_copy_the_samples() {
        let samples: Vec<u64> = (0..200).map(|i| i % 3).collect();
        let block = block(1000, &samples, 2);
        let slice = block
            .slice(TickRange::new(Tick(1063), Tick(1130)))
            .expect("a slice");
        assert_eq!(slice.start(), Tick(1063));
        assert_eq!(slice.len(), 67);
        for tick in 1063..1130 {
            assert_eq!(slice.levels(Tick(tick)), block.levels(Tick(tick)));
        }
        assert!(block.slice(TickRange::new(Tick(999), Tick(1001))).is_none());
        assert!(block.slice(TickRange::at(Tick(1001))).is_none());
    }

    #[test]
    fn masks_iterate_channels() {
        let mask = ChannelMask::one(3).with(0).with(63);
        assert_eq!(mask.iter().collect::<Vec<_>>(), vec![0, 3, 63]);
        assert!(ChannelMask::EMPTY.iter().next().is_none());
    }
}
