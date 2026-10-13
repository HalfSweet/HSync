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

#[cfg(test)]
mod tests {
    use super::*;

    /// A capture holding samples `offset..offset + samples` of a signal whose channel `c` is
    /// bit `c` of the sample's index, on the channels listed.
    fn capture(offset: u64, samples: u64, channels: &[ChannelIndex]) -> LogicCapture {
        let mut capture = LogicCapture::new(1_000_000, channels.iter().copied());
        for &channel in channels {
            let words: Vec<u64> = (0..samples.div_ceil(64))
                .map(|word| {
                    (0..64).fold(0u64, |acc, bit| {
                        let sample = word * 64 + bit;
                        let level = sample < samples && ((offset + sample) >> channel) & 1 == 1;
                        acc | (u64::from(level) << bit)
                    })
                })
                .collect();
            let data = capture.channel_mut(channel).expect("the channel exists");
            data.append_words(&words);
            data.truncate(samples);
        }
        capture
    }

    fn block(item: Option<SignalItem>) -> Arc<LogicBlock> {
        match item {
            Some(SignalItem::Data(block)) => block,
            other => panic!("expected a block, got {other:?}"),
        }
    }

    /// Checks every level of `block` against the signal, `present` naming the channels the
    /// capture had (the others read low).
    fn assert_block_matches(block: &LogicBlock, present: &[ChannelIndex]) {
        for tick in block.start().0..block.end().0 {
            let levels = block.levels(Tick(tick)).expect("the tick is in the block");
            for channel in 0..block.channels() {
                let expected =
                    present.contains(&ChannelIndex::from(channel)) && (tick >> channel) & 1 == 1;
                assert_eq!(
                    levels.0 >> channel & 1 == 1,
                    expected,
                    "tick {tick}, channel {channel}"
                );
            }
        }
    }

    #[test]
    fn blocks_end_on_words_until_the_capture_finishes() {
        let capture = capture(0, 1000, &[0, 1, 2]);
        let mut feeder = Feeder::new(3, 128).expect("a feeder");
        let mut ends = Vec::new();
        while let Some(item) = feeder.next(&capture, 0, false) {
            let block = block(Some(item));
            assert_eq!(block.channels(), 3);
            assert_block_matches(&block, &[0, 1, 2]);
            ends.push(block.end().0);
        }
        // 1000 samples hold 15 whole words: 960 samples, in blocks of 128.
        assert_eq!(ends, [128, 256, 384, 512, 640, 768, 896, 960]);
        assert_eq!(feeder.fed(), Tick(960));
        let tail = block(feeder.next(&capture, 0, true));
        assert_eq!((tail.start(), tail.end()), (Tick(960), Tick(1000)));
        assert_block_matches(&tail, &[0, 1, 2]);
        assert!(feeder.next(&capture, 0, true).is_none());
    }

    #[test]
    fn a_growing_capture_is_fed_as_it_grows() {
        let whole = capture(0, 320, &[0, 1]);
        let mut growing = LogicCapture::new(1_000_000, 0..2);
        let mut feeder = Feeder::new(2, 64).expect("a feeder");
        let mut fed = Vec::new();
        for words in 0..5usize {
            for channel in 0..2 {
                let word = whole.channel(channel).expect("the channel").word(words);
                growing
                    .channel_mut(channel)
                    .expect("the channel")
                    .append_words(&[word]);
            }
            while let Some(item) = feeder.next(&growing, 0, false) {
                let block = block(Some(item));
                assert_block_matches(&block, &[0, 1]);
                fed.push((block.start().0, block.end().0));
            }
        }
        assert_eq!(
            fed,
            [(0, 64), (64, 128), (128, 192), (192, 256), (256, 320)]
        );
    }

    #[test]
    fn a_channel_the_capture_lacks_reads_low() {
        let capture = capture(0, 128, &[0, 2]);
        let mut feeder = Feeder::new(3, 128).expect("a feeder");
        let block = block(feeder.next(&capture, 0, false));
        assert_block_matches(&block, &[0, 2]);
    }

    #[test]
    fn samples_dropped_before_they_were_fed_become_a_gap() {
        // The capture kept samples 256.. after dropping its first four words.
        let capture = capture(256, 192, &[0, 1]);
        let mut feeder = Feeder::new(2, 64).expect("a feeder");
        match feeder.next(&capture, 256, false) {
            Some(SignalItem::Gap(gap)) => assert_eq!(gap, TickRange::new(Tick(0), Tick(256))),
            other => panic!("expected a gap, got {other:?}"),
        }
        let first = block(feeder.next(&capture, 256, false));
        assert_eq!((first.start(), first.end()), (Tick(256), Tick(320)));
        assert_block_matches(&first, &[0, 1]);
        let fed: Vec<u64> = std::iter::from_fn(|| feeder.next(&capture, 256, false))
            .map(|item| block(Some(item)).end().0)
            .collect();
        assert_eq!(fed, [384, 448]);
    }

    #[test]
    fn a_front_dropped_off_a_word_boundary_shifts_the_words() {
        let capture = capture(40, 200, &[0, 1, 2]);
        let mut feeder = Feeder::new(3, 64).expect("a feeder");
        assert!(matches!(
            feeder.next(&capture, 40, false),
            Some(SignalItem::Gap(_))
        ));
        // 200 samples: 192 whole words' worth before finishing.
        let mut ends = Vec::new();
        while let Some(item) = feeder.next(&capture, 40, false) {
            let block = block(Some(item));
            assert_block_matches(&block, &[0, 1, 2]);
            ends.push(block.end().0);
        }
        assert_eq!(ends, [104, 168, 232]);
        let tail = block(feeder.next(&capture, 40, true));
        assert_eq!((tail.start(), tail.end()), (Tick(232), Tick(240)));
        assert_block_matches(&tail, &[0, 1, 2]);
    }

    #[test]
    fn the_channel_count_is_checked() {
        assert_eq!(Feeder::new(0, 64).err(), Some(FeedError::Channels(0)));
        assert_eq!(Feeder::new(65, 64).err(), Some(FeedError::Channels(65)));
        assert!(Feeder::new(64, 64).is_ok());
    }
}
