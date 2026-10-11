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
        if len == 0 {
            return Err(BlockError::Empty);
        }
        let stride = len.div_ceil(64);
        let channels_len = usize::from(channels);
        let mut words = vec![0u64; stride * channels_len];
        // 64 samples of one byte group are a 64 × 8 bit matrix, sample by channel: eight rows
        // of eight samples each. Transposing each row's 8 × 8 bits, then the 8 × 8 bytes of
        // the rows, gives the eight channels' words of the tile.
        for (index, tile) in bytes[..len * unit].chunks(64 * unit).enumerate() {
            for group in 0..channels_len.div_ceil(8) {
                let mut rows = [0u64; 8];
                for (row, samples) in tile.chunks(8 * unit).enumerate() {
                    rows[row] = if unit == 1 && samples.len() == 8 {
                        u64::from_le_bytes([
                            samples[0], samples[1], samples[2], samples[3], samples[4], samples[5],
                            samples[6], samples[7],
                        ])
                    } else {
                        samples
                            .chunks_exact(unit)
                            .enumerate()
                            .fold(0u64, |row, (sample, bytes)| {
                                row | u64::from(bytes[group]) << (8 * sample)
                            })
                    };
                }
                let lanes = transpose_64x8(rows);
                let first = 8 * group;
                for channel in first..(first + 8).min(channels_len) {
                    words[channel * stride + index] = lanes[channel - first];
                }
            }
        }
        Ok(Self {
            start,
            len: len as u64,
            channels,
            stride,
            words,
        })
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
        // A plain indexed loop; iterator adaptors over the strided words measured slower.
        let mut levels = 0u64;
        for channel in 0..self.channels {
            levels |= (self.word(channel, index) >> bit & 1) << channel;
        }
        Some(Levels(levels))
    }

    /// The same samples starting at `start` instead.
    pub fn rebased(&self, start: Tick) -> LogicBlock {
        LogicBlock {
            start,
            ..self.clone()
        }
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
        if !self.contains(from) {
            return None;
        }
        // The watched channels the block has; a channel it does not have never changes.
        let present = if self.channels >= MAX_CHANNELS {
            u64::MAX
        } else {
            (1u64 << self.channels) - 1
        };
        let watched = mask.0 & present;
        if watched == 0 {
            return None;
        }
        let first = self.start.until(from) as usize;
        let (first_word, shift) = (first / 64, first % 64);
        let (stride, words) = (self.stride, &self.words);
        // Where the samples of `channel` in word `index` differ from the sample before each:
        // the last sample of the previous word stands before the first of this one, and `prev`
        // before `from`. Bits below `shift` in the first word are masked out by the caller.
        let edges = |channel: usize, index: usize| -> u64 {
            let current = words[channel * stride + index];
            let before = if index == first_word {
                ((current << 1) & !(1u64 << shift)) | (u64::from(prev.high(channel as u8)) << shift)
            } else {
                (current << 1) | (words[channel * stride + index - 1] >> 63)
            };
            current ^ before
        };
        let last_word = stride - 1;
        let valid_in_last = self.len - 64 * last_word as u64;
        for index in first_word..stride {
            let mut diff = 0u64;
            let mut bits = watched;
            while bits != 0 {
                let channel = bits.trailing_zeros() as usize;
                bits &= bits - 1;
                diff |= edges(channel, index);
            }
            if index == first_word {
                diff &= u64::MAX << shift;
            }
            if index == last_word && valid_in_last < 64 {
                diff &= (1u64 << valid_in_last) - 1;
            }
            if diff == 0 {
                continue;
            }
            let bit = diff.trailing_zeros();
            let tick = self.start.offset(64 * index as u64 + u64::from(bit));
            // The channels that changed at `tick`, from the same edge words: no second look
            // at the samples.
            let mut changed = 0u64;
            let mut bits = watched;
            while bits != 0 {
                let channel = bits.trailing_zeros() as usize;
                bits &= bits - 1;
                changed |= (edges(channel, index) >> bit & 1) << channel;
            }
            return Some((tick, ChannelMask(changed)));
        }
        None
    }
}

/// Transposes an 8 × 8 bit matrix held as eight bytes: byte `r` is row `r`, bit `c` of it
/// column `c`. Swaps the 1 × 1, 2 × 2 and 4 × 4 blocks across the diagonal in turn.
fn transpose_8x8(mut matrix: u64) -> u64 {
    let swap = (matrix ^ (matrix >> 7)) & 0x00aa_00aa_00aa_00aa;
    matrix ^= swap ^ (swap << 7);
    let swap = (matrix ^ (matrix >> 14)) & 0x0000_cccc_0000_cccc;
    matrix ^= swap ^ (swap << 14);
    let swap = (matrix ^ (matrix >> 28)) & 0x0000_0000_f0f0_f0f0;
    matrix ^ swap ^ (swap << 28)
}

/// Transposes a 64 × 8 bit matrix: `rows[r]` holds samples `8 r .. 8 r + 8` as bytes, bit
/// `c` of each being channel `c`; the result holds channel `c`'s 64 samples in element `c`.
/// Each row's bits are transposed, then the bytes of the rows, by halves, quarters and single
/// bytes.
fn transpose_64x8(rows: [u64; 8]) -> [u64; 8] {
    let mut t = rows.map(transpose_8x8);
    for row in 0..4 {
        let (a, b) = (t[row], t[row + 4]);
        t[row] = (a & 0x0000_0000_ffff_ffff) | (b << 32);
        t[row + 4] = (a >> 32) | (b & 0xffff_ffff_0000_0000);
    }
    for row in [0, 1, 4, 5] {
        let (a, b) = (t[row], t[row + 2]);
        t[row] = (a & 0x0000_ffff_0000_ffff) | ((b & 0x0000_ffff_0000_ffff) << 16);
        t[row + 2] = ((a & 0xffff_0000_ffff_0000) >> 16) | (b & 0xffff_0000_ffff_0000);
    }
    for row in [0, 2, 4, 6] {
        let (a, b) = (t[row], t[row + 1]);
        t[row] = (a & 0x00ff_00ff_00ff_00ff) | ((b & 0x00ff_00ff_00ff_00ff) << 8);
        t[row + 1] = ((a & 0xff00_ff00_ff00_ff00) >> 8) | (b & 0xff00_ff00_ff00_ff00);
    }
    t
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
    /// No watched channel changed before the deadline of [`SignalCursor::next_change_before`];
    /// the cursor now stands at this tick.
    Quiet(Tick),
    /// Samples are missing; the cursor now stands at the end of the gap.
    Gap(TickRange),
    /// No change up to this tick, which the source has reached: either the source said so, or
    /// the cursor used up the block ending there and the next one has not arrived.
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
///
/// The cursor moves forward only. [`SignalCursor::next_change`] scans for the next edge on the
/// watched channels, [`SignalCursor::next_change_before`] does the same up to a deadline,
/// [`SignalCursor::seek`] jumps ahead to a tick and [`SignalCursor::sample`] reads the levels
/// where the cursor stands. Blocks the cursor has moved past are released; when it uses a block
/// up and the next one has not arrived, it reports the block's end as progress once, so that
/// decoders built on time can see time pass.
pub struct SignalCursor {
    inbox: Inbox<Arc<LogicBlock>>,
    /// The block the cursor stands in, until it is used up.
    block: Option<Arc<LogicBlock>>,
    /// The next tick to look at.
    pos: Tick,
    /// The levels at `prev_tick`, the last tick seen before `pos`.
    prev: Levels,
    /// `None` at the start and right after a gap: no tick before `pos` was seen.
    prev_tick: Option<Tick>,
    /// The end of a block used up with the next one not yet there, until it is announced.
    exhausted: Option<Tick>,
}

impl fmt::Debug for SignalCursor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SignalCursor")
            .field("pos", &self.pos)
            .field("prev", &self.prev)
            .field("prev_tick", &self.prev_tick)
            .finish_non_exhaustive()
    }
}

/// What the cursor found in the block it stands in.
enum Found {
    Change(Tick, ChannelMask, Levels),
    Quiet(Tick),
    /// The block is used up; its last levels were noted.
    Exhausted,
}

impl SignalCursor {
    pub(crate) fn new(inbox: Inbox<Arc<LogicBlock>>) -> Self {
        Self {
            inbox,
            block: None,
            pos: Tick::ZERO,
            prev: Levels(0),
            prev_tick: None,
            exhausted: None,
        }
    }

    /// The next tick the cursor will look at.
    pub fn position(&self) -> Tick {
        self.pos
    }

    /// Moves the cursor forward to `tick`, to sample there or to wait for edges from there on;
    /// a tick not after the position leaves the cursor where it is.
    pub fn seek(&mut self, tick: Tick) {
        if tick > self.pos {
            self.pos = tick;
        }
    }

    /// Notes the last levels of `block`, stands at its end and lets it go.
    fn release(&mut self, block: &LogicBlock) {
        let last = Tick(block.end().0 - 1);
        self.prev = block.levels(last).unwrap_or_default();
        self.prev_tick = Some(last);
        self.pos = self.pos.max(block.end());
        self.exhausted = Some(block.end());
        self.block = None;
    }

    /// Takes the next item from the source if one is waiting; `Pending` sets the node to wait
    /// for input. A block is kept as the current block; the others are returned.
    fn poll_source(&mut self) -> Poll<Option<Item<Arc<LogicBlock>>>> {
        let taken = match self.inbox.poll_peek() {
            Poll::Pending => return Poll::Pending,
            Poll::Ready(item) => match &*item {
                Item::Event(event) => {
                    self.block = Some(Arc::clone(&event.payload));
                    // The first block decides where the samples start; later ones follow the
                    // position, which may stand ahead of them after a seek.
                    self.pos = self.pos.max(event.payload.start());
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
            self.pos = self.pos.max(range.end);
            self.prev_tick = None;
        }
        self.exhausted = None;
        Poll::Ready(taken)
    }

    /// Scans the current block from the position for a change on `mask` before `until`.
    fn scan(&mut self, block: &LogicBlock, mask: ChannelMask, until: Option<Tick>) -> Found {
        if block.end() <= self.pos {
            self.release(block);
            return Found::Exhausted;
        }
        let limit = until.map_or(block.end(), |until| until.min(block.end()));
        if limit <= self.pos {
            return Found::Quiet(self.pos);
        }
        let before = Tick(self.pos.0.saturating_sub(1));
        let (from, prev) = if self.prev_tick == Some(before) && self.pos > Tick::ZERO {
            (self.pos, self.prev)
        } else if self.pos > block.start() {
            (self.pos, block.levels(before).unwrap_or_default())
        } else {
            // Nothing before the first sample of the stream or after a gap: it is no edge.
            (self.pos.next(), block.levels(self.pos).unwrap_or_default())
        };
        if from < limit
            && let Some((tick, changed)) = block.next_change(mask, from, prev)
            && tick < limit
        {
            let levels = block.levels(tick).unwrap_or_default();
            self.prev = levels;
            self.prev_tick = Some(tick);
            self.pos = tick.next();
            return Found::Change(tick, changed, levels);
        }
        if limit < block.end() {
            let last = Tick(limit.0 - 1);
            self.prev = block.levels(last).unwrap_or_default();
            self.prev_tick = Some(last);
            self.pos = limit;
            return Found::Quiet(limit);
        }
        self.release(block);
        Found::Exhausted
    }

    fn poll_change(&mut self, mask: ChannelMask, until: Option<Tick>) -> Poll<Change> {
        let cx = self.inbox.cx().clone();
        if !cx.has_budget() {
            cx.wait(Wait::Budget);
            return Poll::Pending;
        }
        loop {
            // The block is taken out while it is scanned, which spares an Arc clone on the
            // hottest path, and put back unless the scan used it up.
            if let Some(block) = self.block.take() {
                let found = self.scan(&block, mask, until);
                if !matches!(found, Found::Exhausted) {
                    self.block = Some(block);
                }
                match found {
                    Found::Change(tick, changed, levels) => {
                        cx.spend();
                        return Poll::Ready(Change::At {
                            tick,
                            changed,
                            levels,
                        });
                    }
                    Found::Quiet(tick) => {
                        cx.spend();
                        return Poll::Ready(Change::Quiet(tick));
                    }
                    // A deadline at the block's end is reached without the next block.
                    Found::Exhausted => continue,
                }
            } else if let Some(until) = until
                && until <= self.pos
            {
                cx.spend();
                return Poll::Ready(Change::Quiet(self.pos));
            }
            match self.poll_source() {
                Poll::Pending => {
                    if let Some(end) = self.exhausted.take() {
                        cx.spend();
                        return Poll::Ready(Change::Progress(end));
                    }
                    return Poll::Pending;
                }
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
            if let Some(block) = self.block.take() {
                if let Some(levels) = block.levels(self.pos) {
                    self.block = Some(block);
                    cx.spend();
                    return Poll::Ready(Sample::At {
                        tick: self.pos,
                        levels,
                    });
                }
                self.release(&block);
            }
            match self.poll_source() {
                Poll::Pending => {
                    if let Some(end) = self.exhausted.take() {
                        cx.spend();
                        return Poll::Ready(Sample::Progress(end));
                    }
                    return Poll::Pending;
                }
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
        poll_fn(move |_| self.poll_change(mask, None))
    }

    /// Like [`SignalCursor::next_change`], but gives up at `until`: when no watched channel
    /// changes before that tick, the cursor moves there and [`Change::Quiet`] is returned.
    /// With an empty mask this waits for the samples up to `until` to arrive.
    pub fn next_change_before(
        &mut self,
        mask: ChannelMask,
        until: Tick,
    ) -> impl Future<Output = Change> + '_ {
        poll_fn(move |_| self.poll_change(mask, Some(until)))
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

    /// The sample-by-sample construction `from_interleaved` replaced.
    fn interleaved_by_sample(
        start: Tick,
        channels: u8,
        bytes: &[u8],
    ) -> Result<LogicBlock, BlockError> {
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

    proptest::proptest! {
        #[test]
        fn interleaved_matches_the_sample_by_sample_build(
            channels in 0u8..=64,
            bytes in proptest::collection::vec(proptest::prelude::any::<u8>(), 0..1200),
        ) {
            let fast = LogicBlock::from_interleaved(Tick(3), channels, &bytes);
            let slow = interleaved_by_sample(Tick(3), channels, &bytes);
            proptest::prop_assert_eq!(fast, slow);
        }
    }

    #[test]
    fn transposes_eight_by_eight() {
        // Row 0 all ones becomes column 0 all ones; the identity stays itself.
        assert_eq!(transpose_8x8(0xff), 0x0101_0101_0101_0101);
        assert_eq!(transpose_8x8(0x0101_0101_0101_0101), 0xff);
        let identity = (0..8).fold(0u64, |m, i| m | 1u64 << (9 * i));
        assert_eq!(transpose_8x8(identity), identity);
        // Element (row 2, column 5) moves to (row 5, column 2).
        assert_eq!(transpose_8x8(1 << (8 * 2 + 5)), 1 << (8 * 5 + 2));
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
    fn rebasing_moves_the_samples() {
        let block = block(10, &[1, 0, 1], 1);
        let moved = block.rebased(Tick(500));
        assert_eq!(moved.range(), TickRange::new(Tick(500), Tick(503)));
        assert_eq!(moved.level(0, Tick(502)), Some(true));
        assert_eq!(moved.words(0), block.words(0));
    }

    use std::cell::RefCell;
    use std::future::Future;
    use std::pin::Pin;
    use std::rc::Rc;
    use std::task::{Context, Poll, Waker};

    use crate::node::NodeCx;
    use crate::stream::{Broadcast, Event};

    fn poll<F: Future>(future: Pin<&mut F>) -> Poll<F::Output> {
        future.poll(&mut Context::from_waker(Waker::noop()))
    }

    /// A cursor on a queue the test pushes into by hand.
    fn cursor() -> (SignalCursor, Rc<RefCell<Broadcast<Arc<LogicBlock>>>>) {
        let queue = Rc::new(RefCell::new(Broadcast::new(16)));
        let cx = NodeCx::new();
        cx.refill(1_000);
        (SignalCursor::new(Inbox::new(Rc::clone(&queue), cx)), queue)
    }

    fn push(queue: &Rc<RefCell<Broadcast<Arc<LogicBlock>>>>, block: LogicBlock) {
        let last = Tick(block.end().0 - 1);
        let item = Item::Event(Event {
            seq: 0,
            span: block.range(),
            settled: last,
            payload: Arc::new(block),
        });
        queue
            .borrow_mut()
            .try_push(item)
            .map_err(|_| ())
            .expect("fits");
    }

    #[test]
    fn seeks_ahead_and_keeps_the_level_before_the_position() {
        let (mut cursor, queue) = cursor();
        // Channel 0: low for 100 ticks, then high from 100 on; cut into blocks of 40.
        let mut samples = vec![0u64; 100];
        samples.extend(std::iter::repeat_n(1u64, 60));
        let whole = block(0, &samples, 1);
        for start in (0..160).step_by(40) {
            push(
                &queue,
                whole
                    .slice(TickRange::new(Tick(start), Tick(start + 40)))
                    .expect("a slice"),
            );
        }
        cursor.seek(Tick(100));
        // The edge at exactly the sought tick is found: the level before it is known.
        let change = poll(Box::pin(cursor.next_change(ChannelMask::one(0))).as_mut());
        assert_eq!(
            change,
            Poll::Ready(Change::At {
                tick: Tick(100),
                changed: ChannelMask::one(0),
                levels: Levels(1)
            })
        );
        cursor.seek(Tick(50));
        assert_eq!(cursor.position(), Tick(101), "a seek never moves back");
        cursor.seek(Tick(130));
        let sample = poll(Box::pin(cursor.sample()).as_mut());
        assert_eq!(
            sample,
            Poll::Ready(Sample::At {
                tick: Tick(130),
                levels: Levels(1)
            })
        );
    }

    #[test]
    fn a_deadline_ends_the_wait_quietly() {
        let (mut cursor, queue) = cursor();
        // Channel 0 rises at 30 and stays high.
        let mut samples = vec![0u64; 50];
        for sample in &mut samples[30..] {
            *sample = 1;
        }
        push(&queue, block(0, &samples, 1));
        let until =
            poll(Box::pin(cursor.next_change_before(ChannelMask::one(0), Tick(20))).as_mut());
        assert_eq!(until, Poll::Ready(Change::Quiet(Tick(20))));
        assert_eq!(cursor.position(), Tick(20));
        let found =
            poll(Box::pin(cursor.next_change_before(ChannelMask::one(0), Tick(40))).as_mut());
        assert!(matches!(
            found,
            Poll::Ready(Change::At { tick: Tick(30), .. })
        ));
        // A deadline already passed is quiet at once.
        let past =
            poll(Box::pin(cursor.next_change_before(ChannelMask::one(0), Tick(10))).as_mut());
        assert_eq!(past, Poll::Ready(Change::Quiet(Tick(31))));
        // A deadline beyond the block waits for the next block.
        let pending =
            poll(Box::pin(cursor.next_change_before(ChannelMask::one(0), Tick(80))).as_mut());
        assert_eq!(
            pending,
            Poll::Ready(Change::Progress(Tick(50))),
            "the used-up block is announced"
        );
        let pending =
            poll(Box::pin(cursor.next_change_before(ChannelMask::one(0), Tick(80))).as_mut());
        assert_eq!(pending, Poll::Pending);
        push(&queue, block(50, &[1u64; 50], 1));
        let quiet =
            poll(Box::pin(cursor.next_change_before(ChannelMask::one(0), Tick(80))).as_mut());
        assert_eq!(quiet, Poll::Ready(Change::Quiet(Tick(80))));
    }

    #[test]
    fn a_deadline_at_the_end_of_a_block_needs_no_next_block() {
        let (mut cursor, queue) = cursor();
        push(&queue, block(0, &[0u64; 50], 1));
        let quiet =
            poll(Box::pin(cursor.next_change_before(ChannelMask::one(0), Tick(50))).as_mut());
        assert_eq!(quiet, Poll::Ready(Change::Quiet(Tick(50))));
        assert_eq!(cursor.position(), Tick(50));
        // The used-up block is still announced before the cursor waits.
        assert_eq!(
            poll(Box::pin(cursor.next_change(ChannelMask::one(0))).as_mut()),
            Poll::Ready(Change::Progress(Tick(50)))
        );
        assert_eq!(
            poll(Box::pin(cursor.next_change(ChannelMask::one(0))).as_mut()),
            Poll::Pending
        );
    }

    #[test]
    fn a_used_up_block_is_announced_once() {
        let (mut cursor, queue) = cursor();
        push(&queue, block(0, &[0u64; 10], 1));
        assert_eq!(
            poll(Box::pin(cursor.next_change(ChannelMask::one(0))).as_mut()),
            Poll::Ready(Change::Progress(Tick(10)))
        );
        assert_eq!(
            poll(Box::pin(cursor.next_change(ChannelMask::one(0))).as_mut()),
            Poll::Pending
        );
        assert_eq!(poll(Box::pin(cursor.sample()).as_mut()), Poll::Pending);
        // The edge on the first sample of the next block is still found.
        push(&queue, block(10, &[1u64; 10], 1));
        assert!(matches!(
            poll(Box::pin(cursor.next_change(ChannelMask::one(0))).as_mut()),
            Poll::Ready(Change::At { tick: Tick(10), .. })
        ));
    }

    #[test]
    fn the_first_sample_after_a_gap_is_no_edge() {
        let (mut cursor, queue) = cursor();
        push(&queue, block(0, &[0u64; 10], 1));
        queue
            .borrow_mut()
            .try_push(Item::Gap(TickRange::new(Tick(10), Tick(20))))
            .map_err(|_| ())
            .expect("fits");
        push(&queue, block(20, &[1u64; 10], 1));
        assert_eq!(
            poll(Box::pin(cursor.next_change(ChannelMask::one(0))).as_mut()),
            Poll::Ready(Change::Gap(TickRange::new(Tick(10), Tick(20))))
        );
        assert_eq!(
            poll(Box::pin(cursor.sample()).as_mut()),
            Poll::Ready(Sample::At {
                tick: Tick(20),
                levels: Levels(1)
            })
        );
        assert_eq!(
            poll(Box::pin(cursor.next_change(ChannelMask::one(0))).as_mut()),
            Poll::Ready(Change::Progress(Tick(30)))
        );
    }

    #[test]
    fn masks_iterate_channels() {
        let mask = ChannelMask::one(3).with(0).with(63);
        assert_eq!(mask.iter().collect::<Vec<_>>(), vec![0, 3, 63]);
        assert!(ChannelMask::EMPTY.iter().next().is_none());
    }
}
