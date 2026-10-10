//! Signal generators of the demo device. Each produces one 64-sample word per channel at a
//! time, so the acquisition can emit cross-format data directly.

use std::sync::Arc;

use hsync_capture::{CHUNK_WORDS, ChunkView, LogicCapture};
use hsync_core::ChannelIndex;

/// Channels the demo device offers.
pub const CHANNELS: u16 = 16;

/// The samples of a capture to replay, as words per channel.
#[derive(Debug)]
pub struct ReplaySource {
    samplerate: u64,
    /// `words[channel]`; channels the capture lacks stay low.
    words: Vec<Vec<u64>>,
    len: usize,
}

impl ReplaySource {
    /// Takes whole words of the capture's first [`CHANNELS`] channels; `None` if it holds fewer
    /// than 64 samples.
    pub fn from_capture(capture: &LogicCapture) -> Option<Self> {
        let len = usize::try_from(capture.len() / 64).ok()?;
        if len == 0 {
            return None;
        }
        let mut words = vec![vec![0u64; len]; usize::from(CHANNELS)];
        for (index, data) in capture.channels() {
            let Some(target) = words.get_mut(usize::from(index)) else {
                continue;
            };
            let mut filled = 0;
            for chunk in 0..data.chunk_count() {
                let remaining = len - filled;
                if remaining == 0 {
                    break;
                }
                match data.chunk(chunk) {
                    ChunkView::Words(chunk_words) => {
                        let take = chunk_words.len().min(remaining);
                        target[filled..filled + take].copy_from_slice(&chunk_words[..take]);
                        filled += take;
                    }
                    ChunkView::Constant(level) => {
                        let take = CHUNK_WORDS.min(remaining);
                        target[filled..filled + take].fill(if level { u64::MAX } else { 0 });
                        filled += take;
                    }
                }
            }
        }
        Some(Self {
            samplerate: capture.samplerate(),
            words,
            len,
        })
    }

    /// The rate the capture was taken at.
    pub fn samplerate(&self) -> u64 {
        self.samplerate
    }

    /// Samples replayed before the source repeats.
    pub fn samples(&self) -> u64 {
        self.len as u64 * 64
    }
}

/// Which signals the demo device produces.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PatternKind {
    /// The bundled protocol capture (I²C, UART, SPI, CAN-FD), repeated.
    Protocol,
    /// Pulses of random width with occasional one-sample glitches; edges fall on any sample.
    Random,
    /// A binary counter: channel `n` toggles every `2^n` samples.
    Counter,
}

impl PatternKind {
    pub const ALL: [Self; 3] = [Self::Protocol, Self::Random, Self::Counter];

    pub fn id(self) -> &'static str {
        match self {
            Self::Protocol => "protocol",
            Self::Random => "random",
            Self::Counter => "counter",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.id() == id)
    }
}

/// `xorshift64*`.
#[derive(Debug, Clone)]
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }
}

#[derive(Debug, Clone)]
struct RandomState {
    rng: Rng,
    level: [bool; CHANNELS as usize],
    /// Samples until each channel toggles.
    run: [u64; CHANNELS as usize],
}

impl RandomState {
    fn new(seed: u64) -> Self {
        let mut state = Self {
            rng: Rng(seed | 1),
            level: [false; CHANNELS as usize],
            run: [0; CHANNELS as usize],
        };
        for channel in 0..usize::from(CHANNELS) {
            state.level[channel] = state.rng.next() & 1 != 0;
            state.run[channel] = state.next_run(channel);
        }
        state
    }

    fn next_run(&mut self, channel: usize) -> u64 {
        let draw = self.rng.next();
        if draw.is_multiple_of(32) {
            return 1;
        }
        let mean = 8u64 << (channel % 8);
        1 + (draw >> 8) % (2 * mean)
    }

    fn word(&mut self, channel: usize) -> u64 {
        let mut word = 0u64;
        let mut bit = 0u64;
        while bit < 64 {
            let take = self.run[channel].min(64 - bit);
            if self.level[channel] {
                let ones = if take == 64 {
                    u64::MAX
                } else {
                    (1 << take) - 1
                };
                word |= ones << bit;
            }
            bit += take;
            self.run[channel] -= take;
            if self.run[channel] == 0 {
                self.level[channel] = !self.level[channel];
                self.run[channel] = self.next_run(channel);
            }
        }
        word
    }
}

/// Bit `j` of the word for counter channel `n < 6`: `(j >> n) & 1`.
const COUNTER_WORDS: [u64; 6] = [
    0xaaaa_aaaa_aaaa_aaaa,
    0xcccc_cccc_cccc_cccc,
    0xf0f0_f0f0_f0f0_f0f0,
    0xff00_ff00_ff00_ff00,
    0xffff_0000_ffff_0000,
    0xffff_ffff_0000_0000,
];

/// A generator positioned at a word of its signal.
#[derive(Debug, Clone)]
pub struct Pattern(Generator);

#[derive(Debug, Clone)]
enum Generator {
    Replay {
        source: Arc<ReplaySource>,
        word: usize,
    },
    Random {
        state: Box<RandomState>,
        word: u64,
    },
    Counter {
        word: u64,
    },
}

impl Pattern {
    /// A generator of `kind`; the protocol pattern needs `replay` and falls back to random
    /// signals without it. `seed` varies the random pattern.
    pub fn new(kind: PatternKind, replay: Option<&Arc<ReplaySource>>, seed: u64) -> Self {
        Self(match (kind, replay) {
            (PatternKind::Protocol, Some(source)) => Generator::Replay {
                source: Arc::clone(source),
                word: 0,
            },
            (PatternKind::Counter, _) => Generator::Counter { word: 0 },
            _ => Generator::Random {
                state: Box::new(RandomState::new(seed)),
                word: 0,
            },
        })
    }

    /// Writes the next 64 samples of each of `channels` to `out` and advances.
    pub fn next_words(&mut self, channels: &[ChannelIndex], out: &mut [u64]) {
        match &mut self.0 {
            Generator::Replay { source, word } => {
                for (slot, &channel) in out.iter_mut().zip(channels) {
                    *slot = source
                        .words
                        .get(usize::from(channel))
                        .map_or(0, |words| words[*word]);
                }
                *word = (*word + 1) % source.len;
            }
            Generator::Random { state, word } => {
                // Every channel advances so that a channel's signal does not depend on which
                // others are enabled.
                let mut all = [0u64; CHANNELS as usize];
                for (channel, slot) in all.iter_mut().enumerate() {
                    *slot = state.word(channel);
                }
                for (slot, &channel) in out.iter_mut().zip(channels) {
                    *slot = all.get(usize::from(channel)).copied().unwrap_or(0);
                }
                *word += 1;
            }
            Generator::Counter { word } => {
                for (slot, &channel) in out.iter_mut().zip(channels) {
                    let channel = u32::from(channel);
                    *slot = if channel < 6 {
                        COUNTER_WORDS[channel as usize]
                    } else if (*word >> (channel - 6)) & 1 != 0 {
                        u64::MAX
                    } else {
                        0
                    };
                }
                *word += 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn samples(pattern: &mut Pattern, channel: ChannelIndex, words: usize) -> Vec<bool> {
        let mut out = [0u64];
        let mut bits = Vec::new();
        for _ in 0..words {
            pattern.next_words(&[channel], &mut out);
            bits.extend((0..64).map(|j| out[0] >> j & 1 != 0));
        }
        bits
    }

    #[test]
    fn counter_channels_toggle_at_powers_of_two() {
        for channel in 0..CHANNELS {
            let mut pattern = Pattern::new(PatternKind::Counter, None, 0);
            let words = (2usize << channel).div_ceil(64).max(1);
            let bits = samples(&mut pattern, channel, words);
            for (sample, &bit) in bits.iter().enumerate() {
                assert_eq!(
                    bit,
                    (sample >> channel) & 1 != 0,
                    "channel {channel} sample {sample}"
                );
            }
        }
    }

    #[test]
    fn random_edges_fall_off_byte_boundaries() {
        let mut pattern = Pattern::new(PatternKind::Random, None, 7);
        let bits = samples(&mut pattern, 0, 64);
        let edges: Vec<usize> = (1..bits.len())
            .filter(|&i| bits[i] != bits[i - 1])
            .collect();
        assert!(edges.len() > 50);
        assert!(edges.iter().any(|edge| edge % 8 != 0));
        let mut again = Pattern::new(PatternKind::Random, None, 7);
        assert_eq!(samples(&mut again, 0, 64), bits, "seeded patterns repeat");
    }

    #[test]
    fn random_channels_do_not_depend_on_each_other() {
        let mut alone = Pattern::new(PatternKind::Random, None, 3);
        let mut together = Pattern::new(PatternKind::Random, None, 3);
        let mut one = [0u64];
        let mut two = [0u64; 2];
        for _ in 0..16 {
            alone.next_words(&[5], &mut one);
            together.next_words(&[2, 5], &mut two);
            assert_eq!(one[0], two[1]);
        }
    }

    #[test]
    fn replays_and_repeats_a_capture() {
        let mut capture = LogicCapture::new(1000, [0, 1]);
        let mut deinterleaver = capture.deinterleaver();
        let mut bytes = Vec::new();
        for word in 0..3u64 {
            bytes.extend_from_slice(&(word + 1).to_le_bytes());
            bytes.extend_from_slice(&(!word).to_le_bytes());
        }
        capture.append_cross(&mut deinterleaver, &bytes);
        let source = Arc::new(ReplaySource::from_capture(&capture).expect("source"));
        assert_eq!(source.samples(), 192);
        let mut pattern = Pattern::new(PatternKind::Protocol, Some(&source), 0);
        let mut out = [0u64; 3];
        let mut seen = Vec::new();
        for _ in 0..4 {
            pattern.next_words(&[0, 1, 9], &mut out);
            seen.push(out);
        }
        assert_eq!(seen, vec![[1, !0, 0], [2, !1, 0], [3, !2, 0], [1, !0, 0]]);
    }
}
