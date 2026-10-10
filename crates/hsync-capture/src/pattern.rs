//! Level patterns, as DSView's search dock finds them (DESIGN.md §9.8): where every listed
//! channel shows a level or an edge.

use hsync_core::{ChannelIndex, SampleIndex};
use serde::Deserialize;
use ts_rs::TS;

use crate::capture::LogicCapture;
use crate::channel::ChannelData;

/// What a channel shows at a pattern's sample (DSView's `0`, `1`, `R`, `F` and `C`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum PatternLevel {
    Low,
    High,
    /// Low at the sample before, high at this one.
    Rising,
    /// High at the sample before, low at this one.
    Falling,
    /// Either edge.
    Edge,
}

/// One channel's part of a pattern.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, TS)]
#[ts(export)]
pub struct PatternCondition {
    pub channel: ChannelIndex,
    pub level: PatternLevel,
}

impl PatternLevel {
    fn is_edge(self) -> bool {
        matches!(self, Self::Rising | Self::Falling | Self::Edge)
    }
}

impl LogicCapture {
    /// The pattern's hit nearest `from`: the first at or after it, or with `forward` false the
    /// last at or before it.
    ///
    /// A sample matches when every channel shows its condition there; an edge is between the
    /// sample before and this one, so sample 0 never matches one. Each matching sample is a hit
    /// for patterns with an edge; for patterns of levels only, the first sample of each stretch
    /// that matches is, so that stepping from hit to hit moves on. A channel the capture lacks
    /// never matches.
    pub fn find_pattern(
        &self,
        conditions: &[PatternCondition],
        from: SampleIndex,
        forward: bool,
    ) -> Option<SampleIndex> {
        let len = self.len();
        if conditions.is_empty() || len == 0 {
            return None;
        }
        let channels = conditions
            .iter()
            .map(|condition| Some((self.channel(condition.channel)?, condition.level)))
            .collect::<Option<Vec<_>>>()?;
        let edges = channels.iter().any(|(_, level)| level.is_edge());
        let words = len.div_ceil(64) as usize;
        let matches = |word: usize| matches(&channels, word, len, edges);
        let hits = |word: usize| {
            let found = matches(word);
            if edges {
                return found;
            }
            let before = if word == 0 {
                0
            } else {
                matches(word - 1) >> 63
            };
            found & !((found << 1) | before)
        };
        let from = from.min(len - 1);
        let first = (from / 64) as usize;
        let bit = from % 64;
        if forward {
            let at_or_after = !0u64 << bit;
            (first..words).find_map(|word| {
                let found = hits(word) & if word == first { at_or_after } else { !0 };
                (found != 0).then(|| word as u64 * 64 + u64::from(found.trailing_zeros()))
            })
        } else {
            let at_or_before = if bit == 63 {
                !0
            } else {
                (1u64 << (bit + 1)) - 1
            };
            (0..=first).rev().find_map(|word| {
                let found = hits(word) & if word == first { at_or_before } else { !0 };
                (found != 0).then(|| word as u64 * 64 + 63 - u64::from(found.leading_zeros()))
            })
        }
    }
}

/// Bit `j` set when sample `64 · word + j` matches every condition.
fn matches(
    channels: &[(&ChannelData, PatternLevel)],
    word: usize,
    len: SampleIndex,
    edges: bool,
) -> u64 {
    let first = word as u64 * 64;
    let valid = len - first;
    let mut mask = if valid >= 64 { !0 } else { (1u64 << valid) - 1 };
    if edges && word == 0 {
        mask &= !1;
    }
    for &(data, level) in channels {
        let value = data.word(word);
        let carry = if word == 0 {
            0
        } else {
            data.word(word - 1) >> 63
        };
        let previous = (value << 1) | carry;
        mask &= match level {
            PatternLevel::Low => !value,
            PatternLevel::High => value,
            PatternLevel::Rising => !previous & value,
            PatternLevel::Falling => previous & !value,
            PatternLevel::Edge => previous ^ value,
        };
    }
    mask
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pattern(conditions: &[(ChannelIndex, PatternLevel)]) -> Vec<PatternCondition> {
        conditions
            .iter()
            .map(|&(channel, level)| PatternCondition { channel, level })
            .collect()
    }

    /// A capture of the channels' levels, one string of `0`/`1` per channel.
    fn capture(levels: &[&str]) -> LogicCapture {
        let mut capture = LogicCapture::new(1_000_000, 0..levels.len() as ChannelIndex);
        for (index, text) in levels.iter().enumerate() {
            let bits: Vec<bool> = text.bytes().map(|b| b == b'1').collect();
            let words: Vec<u64> = bits
                .chunks(64)
                .map(|chunk| {
                    chunk
                        .iter()
                        .enumerate()
                        .fold(0, |word, (bit, &high)| word | (u64::from(high) << bit))
                })
                .collect();
            let data = capture.channel_mut(index as ChannelIndex).expect("channel");
            data.append_words(&words);
            data.truncate(bits.len() as SampleIndex);
        }
        capture
    }

    /// The pattern as found sample by sample, for comparison.
    fn naive(
        levels: &[&str],
        conditions: &[(ChannelIndex, PatternLevel)],
        from: usize,
        forward: bool,
    ) -> Option<u64> {
        let at = |channel: ChannelIndex, sample: usize| {
            levels[channel as usize].as_bytes()[sample] == b'1'
        };
        let len = levels[0].len();
        let edges = conditions.iter().any(|(_, level)| level.is_edge());
        let matches = |sample: usize| {
            conditions.iter().all(|&(channel, level)| {
                let now = at(channel, sample);
                let before = sample.checked_sub(1).map(|s| at(channel, s));
                match level {
                    PatternLevel::Low => !now,
                    PatternLevel::High => now,
                    PatternLevel::Rising => before == Some(false) && now,
                    PatternLevel::Falling => before == Some(true) && !now,
                    PatternLevel::Edge => before.is_some_and(|b| b != now),
                }
            })
        };
        let hit = |sample: usize| matches(sample) && (edges || sample == 0 || !matches(sample - 1));
        let from = from.min(len - 1);
        if forward {
            (from..len).find(|&s| hit(s)).map(|s| s as u64)
        } else {
            (0..=from).rev().find(|&s| hit(s)).map(|s| s as u64)
        }
    }

    #[test]
    fn finds_levels_at_the_start_of_each_stretch() {
        let levels = ["0011100111", "1111011110"];
        let capture = capture(&levels);
        let both = [(0, PatternLevel::High), (1, PatternLevel::High)];
        assert_eq!(capture.find_pattern(&pattern(&both), 0, true), Some(2));
        assert_eq!(capture.find_pattern(&pattern(&both), 3, true), Some(7));
        assert_eq!(capture.find_pattern(&pattern(&both), 9, false), Some(7));
        assert_eq!(capture.find_pattern(&pattern(&both), 5, false), Some(2));
        assert_eq!(capture.find_pattern(&pattern(&both), 1, false), None);
    }

    #[test]
    fn finds_every_edge() {
        let levels = ["0101100", "1111111"];
        let capture = capture(&levels);
        let rising = [(0, PatternLevel::Rising), (1, PatternLevel::High)];
        assert_eq!(capture.find_pattern(&pattern(&rising), 0, true), Some(1));
        assert_eq!(capture.find_pattern(&pattern(&rising), 2, true), Some(3));
        let either = [(0, PatternLevel::Edge)];
        assert_eq!(capture.find_pattern(&pattern(&either), 2, true), Some(2));
        assert_eq!(capture.find_pattern(&pattern(&either), 4, true), Some(5));
        assert_eq!(capture.find_pattern(&pattern(&either), 6, false), Some(5));
    }

    #[test]
    fn never_matches_missing_channels() {
        let capture = capture(&["0110"]);
        assert_eq!(
            capture.find_pattern(&pattern(&[(3, PatternLevel::High)]), 0, true),
            None
        );
        assert_eq!(capture.find_pattern(&[], 0, true), None);
    }

    #[test]
    fn agrees_with_a_sample_by_sample_search_across_words() {
        // Pseudo-random levels over several words, with runs of different lengths.
        let mut seed = 0x2545_f491_4f6c_dd1du64;
        let mut random = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        let make = |random: &mut dyn FnMut() -> u64| -> String {
            let mut text = String::new();
            let mut level = false;
            while text.len() < 300 {
                let run = (random() % 70 + 1) as usize;
                text.extend(std::iter::repeat_n(if level { '1' } else { '0' }, run));
                level = !level;
            }
            text.truncate(300);
            text
        };
        let a = make(&mut random);
        let b = make(&mut random);
        let levels = [a.as_str(), b.as_str()];
        let capture = capture(&levels);
        let all = [
            PatternLevel::Low,
            PatternLevel::High,
            PatternLevel::Rising,
            PatternLevel::Falling,
            PatternLevel::Edge,
        ];
        for first in all {
            for second in all {
                let conditions = [(0, first), (1, second)];
                for from in [0, 1, 63, 64, 65, 127, 128, 200, 299] {
                    for forward in [true, false] {
                        assert_eq!(
                            capture.find_pattern(&pattern(&conditions), from, forward),
                            naive(&levels, &conditions, from as usize, forward),
                            "{conditions:?} from {from} forward {forward}"
                        );
                    }
                }
            }
        }
    }
}
