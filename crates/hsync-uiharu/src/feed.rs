//! Cutting a capture's samples into the engine's blocks.

use std::sync::Arc;

use hsync_capture::LogicCapture;
use hsync_core::{ChannelIndex, SampleIndex};
use uiharu_core::{LogicBlock, SignalItem, Tick, TickRange};

/// Why a feeder could not be made.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum FeedError {
    /// A block holds 1 to 64 channels.
    #[error("{0} channels; a block holds 1 to 64")]
    Channels(u8),
}

/// Cuts the samples of a capture into the items of a pipeline's source, in order and as far
/// as the capture has them, so that a capture can be decoded while a device fills it.
///
/// Block channel `i` is capture channel `i`; a channel the capture lacks reads low. Ticks
/// count from the first sample the capture ever held: when a capture drops samples from its
/// front (a loop capture does, to stay within its limit), the feeder is told how many
/// (`dropped`) and the samples it never got to are announced as a gap.
///
/// Blocks end on a multiple of 64 samples, so that a channel's words are copied whole,
/// except the last one of a finished capture.
#[derive(Debug)]
pub struct Feeder {
    channels: u8,
    block_samples: u64,
    fed: u64,
}

impl Feeder {
    /// Samples per block by default.
    pub const DEFAULT_BLOCK: u64 = 1 << 16;

    /// A feeder of `channels` channels, in blocks of `block_samples` (rounded up to a
    /// multiple of 64).
    pub fn new(channels: u8, block_samples: u64) -> Result<Self, FeedError> {
        if !(1..=64).contains(&channels) {
            return Err(FeedError::Channels(channels));
        }
        Ok(Self {
            channels,
            block_samples: block_samples.max(1).next_multiple_of(64),
            fed: 0,
        })
    }

    /// The channels of every block.
    pub fn channels(&self) -> u8 {
        self.channels
    }

    /// The tick up to which samples have been fed.
    pub fn fed(&self) -> Tick {
        Tick(self.fed)
    }

    /// The next item beyond what was fed so far: a gap for samples `capture` dropped before
    /// they were fed, else the next block of samples it holds; `None` when there is not a
    /// whole word's worth yet — or, with `finishing`, nothing at all. The last block of a
    /// finishing capture may end on any sample.
    pub fn next(
        &mut self,
        capture: &LogicCapture,
        dropped: SampleIndex,
        finishing: bool,
    ) -> Option<SignalItem> {
        if self.fed < dropped {
            let gap = TickRange::new(Tick(self.fed), Tick(dropped));
            self.fed = dropped;
            return Some(SignalItem::Gap(gap));
        }
        let len = capture.len();
        let end = if finishing {
            dropped + len
        } else {
            dropped + len - len % 64
        };
        if end <= self.fed {
            return None;
        }
        let end = end.min(self.fed + self.block_samples);
        let samples = end - self.fed;
        let first = self.fed - dropped;
        let words_per_channel = samples.div_ceil(64) as usize;
        let mut words = Vec::with_capacity(words_per_channel * usize::from(self.channels));
        for channel in 0..ChannelIndex::from(self.channels) {
            match capture.channel(channel) {
                Some(data) => {
                    let count = data.word_count();
                    let word = |index: usize| if index < count { data.word(index) } else { 0 };
                    let base = (first / 64) as usize;
                    let shift = (first % 64) as u32;
                    words.extend((0..words_per_channel).map(|j| {
                        if shift == 0 {
                            word(base + j)
                        } else {
                            word(base + j) >> shift | word(base + j + 1) << (64 - shift)
                        }
                    }));
                }
                None => words.extend(std::iter::repeat_n(0, words_per_channel)),
            }
        }
        let block = LogicBlock::from_words(Tick(self.fed), samples, self.channels, words)
            .expect("the channels were checked and the words counted");
        self.fed = end;
        Some(SignalItem::Data(Arc::new(block)))
    }
}
