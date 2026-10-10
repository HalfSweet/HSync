//! Waiting for pin conditions over bit-packed samples, with the semantics of sigrok's
//! `Decoder.wait()` as DSView's libsigrokdecode4DSL implements it (`instance.c`, DESIGN.md
//! §7.3):
//!
//! - a wait takes conditions that are OR-ed; each condition AND-s its terms;
//! - the first wait starts at sample 0, whose "previous" levels are its own, so no edge matches
//!   there; after a match at `m` the next wait starts at `m + 1`;
//! - a `skip` of `n` matches `n + 1` samples after the previous match (`n` samples after the
//!   start of the wait); `skip: 0` right after a match matches that same sample again.
//!
//! Scanning works a 64-sample word at a time instead of sample by sample.

use hsync_capture::ChannelData;

/// What a term wants of one channel (`'l' 'h' 'r' 'f' 'e' 's'`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pin {
    Low,
    High,
    Rising,
    Falling,
    Either,
    Stable,
}

/// One term of a condition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Term {
    /// Decoder channel `slot` must show `pin`.
    Pin(usize, Pin),
    /// `{'skip': n}`.
    Skip(u64),
}

/// Terms that must all hold at one sample.
pub type Condition = Vec<Term>;

/// A match of [`Sampler::wait`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Match {
    /// The sample that matched.
    pub sample: u64,
    /// Bit `i` set: condition `i` holds at `sample`.
    pub matched: u32,
}

impl Match {
    /// Whether condition `index` holds.
    pub fn has(&self, index: usize) -> bool {
        self.matched & (1 << index) != 0
    }
}

/// Walks the channels of one decoder through a capture.
#[derive(Debug)]
pub struct Sampler<'a> {
    channels: Vec<&'a ChannelData>,
    end: u64,
    /// The last match, if any.
    current: Option<u64>,
}

/// Bits `from..64` of a word.
fn from_bit(from: u64) -> u64 {
    if from >= 64 { 0 } else { u64::MAX << from }
}

/// Bits `0..to` of a word.
fn below_bit(to: u64) -> u64 {
    if to >= 64 { u64::MAX } else { (1u64 << to) - 1 }
}

impl<'a> Sampler<'a> {
    /// A sampler over `channels` (by decoder slot) and their first `end` samples.
    pub fn new(channels: Vec<&'a ChannelData>, end: u64) -> Self {
        let end = channels
            .iter()
            .map(|channel| channel.len())
            .fold(end, u64::min);
        Self {
            channels,
            end,
            current: None,
        }
    }

    /// Continues from a previous sampler's last match, as `samplenum` was after it.
    pub fn resume(mut self, current: Option<u64>) -> Self {
        self.current = current;
        self
    }

    /// Samples the sampler walks through.
    pub fn len(&self) -> u64 {
        self.end
    }

    /// Whether there are no samples.
    pub fn is_empty(&self) -> bool {
        self.end == 0
    }

    /// The sample of the last match (`self.samplenum`), 0 before the first.
    pub fn samplenum(&self) -> u64 {
        self.current.unwrap_or(0)
    }

    /// Level of decoder channel `slot` at `sample`.
    pub fn level(&self, slot: usize, sample: u64) -> bool {
        self.channels[slot].sample(sample)
    }

    /// Level of decoder channel `slot` at the last match.
    pub fn pin(&self, slot: usize) -> bool {
        self.level(slot, self.samplenum())
    }

    /// The word of `slot` at `word` and the level of the sample before it.
    fn word_and_previous(&self, slot: usize, word: usize) -> (u64, bool) {
        let channel = self.channels[slot];
        let value = channel.word(word);
        let previous = if word == 0 {
            // Sample 0 counts as its own predecessor.
            value & 1 != 0
        } else {
            channel.word(word - 1) >> 63 != 0
        };
        (value, previous)
    }

    /// Waits until one of `conditions` holds; `None` when the samples run out first.
    pub fn wait(&mut self, conditions: &[Condition]) -> Option<Match> {
        debug_assert!(conditions.len() <= 32);
        let start = self.current.map_or(0, |previous| previous + 1);
        // `skip: 0` after a match answers the matched sample at once.
        if let Some(previous) = self.current {
            let mut matched = 0;
            for (index, condition) in conditions.iter().enumerate() {
                if !condition.is_empty() && condition.iter().all(|term| *term == Term::Skip(0)) {
                    matched |= 1 << index;
                }
            }
            if matched != 0 {
                return Some(Match {
                    sample: previous,
                    matched,
                });
            }
        }
        if conditions.iter().all(Vec::is_empty) {
            // sigrok: an empty condition list matches at once.
            let sample = start.min(self.end.saturating_sub(1));
            self.current = Some(sample);
            return Some(Match { sample, matched: 0 });
        }

        let mut word = (start / 64) as usize;
        while (word as u64) * 64 < self.end {
            let base = word as u64 * 64;
            let valid = from_bit(start.saturating_sub(base)) & below_bit(self.end - base);
            let mut masks = [0u64; 32];
            let mut any = 0;
            for (index, condition) in conditions.iter().enumerate() {
                if condition.is_empty() {
                    continue;
                }
                let mut mask = valid;
                for term in condition {
                    mask &= match *term {
                        Term::Skip(count) => {
                            let target = start + count;
                            if target >= base && target < base + 64 {
                                1u64 << (target - base)
                            } else {
                                0
                            }
                        }
                        Term::Pin(slot, pin) => {
                            let (value, previous) = self.word_and_previous(slot, word);
                            let before = (value << 1) | u64::from(previous);
                            match pin {
                                Pin::Low => !value,
                                Pin::High => value,
                                Pin::Rising => value & !before,
                                Pin::Falling => !value & before,
                                Pin::Either => value ^ before,
                                Pin::Stable => !(value ^ before),
                            }
                        }
                    };
                    if mask == 0 {
                        break;
                    }
                }
                masks[index] = mask;
                any |= mask;
            }
            if any != 0 {
                let bit = any.trailing_zeros();
                let matched = masks[..conditions.len()]
                    .iter()
                    .enumerate()
                    .fold(0u32, |matched, (index, mask)| {
                        matched | (u32::from((mask >> bit) & 1 == 1) << index)
                    });
                let sample = base + u64::from(bit);
                self.current = Some(sample);
                return Some(Match { sample, matched });
            }
            word += 1;
        }
        self.current = Some(self.end.saturating_sub(1));
        None
    }

    /// The first sample at or after `from` where `slot` changes level relative to the sample
    /// before it, using the channel's summaries to skip quiet stretches.
    pub fn next_edge(&self, slot: usize, from: u64) -> Option<u64> {
        if from >= self.end {
            return None;
        }
        let channel = self.channels[slot];
        let edge = if from == 0 {
            channel.next_change(0)
        } else {
            channel.next_change(from - 1)
        }?;
        (edge < self.end).then_some(edge)
    }
}

#[cfg(test)]
mod tests {
    use hsync_capture::LogicCapture;
    use proptest::prelude::*;

    use super::*;

    fn capture(channels: &[Vec<bool>]) -> LogicCapture {
        let len = channels.first().map_or(0, Vec::len);
        let mut capture = LogicCapture::new(1_000_000, 0..channels.len() as u16);
        for (index, levels) in channels.iter().enumerate() {
            let mut words = vec![0u64; len.div_ceil(64)];
            for (sample, &level) in levels.iter().enumerate() {
                if level {
                    words[sample / 64] |= 1 << (sample % 64);
                }
            }
            let data = capture.channel_mut(index as u16).expect("channel");
            data.append_words(&words);
            data.truncate(len as u64);
        }
        capture
    }

    /// The per-sample loop of libsigrokdecode4DSL's `find_match()`.
    struct Naive<'a> {
        levels: &'a [Vec<bool>],
        current: Option<usize>,
    }

    impl Naive<'_> {
        fn wait(&mut self, conditions: &[Condition]) -> Option<(usize, u32)> {
            let len = self.levels[0].len();
            let start = self.current.map_or(0, |previous| previous + 1);
            if let Some(previous) = self.current {
                let mut matched = 0;
                for (index, condition) in conditions.iter().enumerate() {
                    if !condition.is_empty() && condition.iter().all(|t| *t == Term::Skip(0)) {
                        matched |= 1 << index;
                    }
                }
                if matched != 0 {
                    return Some((previous, matched));
                }
            }
            for sample in start..len {
                let mut matched = 0u32;
                for (index, condition) in conditions.iter().enumerate() {
                    if condition.is_empty() {
                        continue;
                    }
                    let holds = condition.iter().all(|term| match *term {
                        Term::Skip(count) => sample as u64 == start as u64 + count,
                        Term::Pin(slot, pin) => {
                            let now = self.levels[slot][sample];
                            let before = self.levels[slot][sample.saturating_sub(1)];
                            match pin {
                                Pin::Low => !now,
                                Pin::High => now,
                                Pin::Rising => !before && now,
                                Pin::Falling => before && !now,
                                Pin::Either => before != now,
                                Pin::Stable => before == now,
                            }
                        }
                    });
                    if holds {
                        matched |= 1 << index;
                    }
                }
                if matched != 0 {
                    self.current = Some(sample);
                    return Some((sample, matched));
                }
            }
            None
        }
    }

    fn pin() -> impl Strategy<Value = Pin> {
        prop_oneof![
            Just(Pin::Low),
            Just(Pin::High),
            Just(Pin::Rising),
            Just(Pin::Falling),
            Just(Pin::Either),
            Just(Pin::Stable),
        ]
    }

    fn term() -> impl Strategy<Value = Term> {
        prop_oneof![
            4 => (0..2usize, pin()).prop_map(|(slot, pin)| Term::Pin(slot, pin)),
            1 => (0..200u64).prop_map(Term::Skip),
        ]
    }

    proptest! {
        #[test]
        fn matches_the_sample_by_sample_definition(
            levels in prop::collection::vec(prop::collection::vec(any::<bool>(), 300), 2),
            waits in prop::collection::vec(
                prop::collection::vec(prop::collection::vec(term(), 1..3), 1..4),
                1..12,
            ),
        ) {
            let capture = capture(&levels);
            let channels = (0..2).map(|i| capture.channel(i).expect("channel")).collect();
            let mut fast = Sampler::new(channels, 300);
            let mut naive = Naive { levels: &levels, current: None };
            for conditions in &waits {
                let expected = naive.wait(conditions);
                let got = fast.wait(conditions).map(|m| (m.sample as usize, m.matched));
                prop_assert_eq!(got, expected);
                if expected.is_none() {
                    break;
                }
            }
        }
    }

    #[test]
    fn sample_zero_has_no_edge() {
        let capture = capture(&[vec![true, true, false, true]]);
        let channels = vec![capture.channel(0).expect("channel")];
        let mut sampler = Sampler::new(channels, 4);
        let rising = vec![vec![Term::Pin(0, Pin::Rising)]];
        assert_eq!(sampler.wait(&rising).map(|m| m.sample), Some(3));
        assert_eq!(sampler.wait(&rising), None);
    }

    #[test]
    fn skips_count_from_the_sample_after_the_match() {
        let capture = capture(&[vec![false; 100]]);
        let channels = vec![capture.channel(0).expect("channel")];
        let mut sampler = Sampler::new(channels, 100);
        let low = vec![vec![Term::Pin(0, Pin::Low)]];
        assert_eq!(sampler.wait(&low).map(|m| m.sample), Some(0));
        assert_eq!(
            sampler.wait(&[vec![Term::Skip(10)]]).map(|m| m.sample),
            Some(11)
        );
        assert_eq!(
            sampler.wait(&[vec![Term::Skip(0)]]).map(|m| m.sample),
            Some(11)
        );
    }

    #[test]
    fn finds_edges_with_summaries() {
        let mut levels = vec![false; 5000];
        levels[4000..].fill(true);
        let capture = capture(&[levels]);
        let channels = vec![capture.channel(0).expect("channel")];
        let sampler = Sampler::new(channels, 5000);
        assert_eq!(sampler.next_edge(0, 0), Some(4000));
        assert_eq!(sampler.next_edge(0, 4000), Some(4000));
        assert_eq!(sampler.next_edge(0, 4001), None);
    }
}
