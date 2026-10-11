// SPDX-License-Identifier: Apache-2.0

//! Making edge sequences on one line: for tests, for tools and for benchmarks.

use uiharu_core::{Levels, LogicBlock, LogicBlockBuilder, Tick};

use super::decoder::TimingConfig;

/// Builds the samples of one line as levels held for given times; the other channels stay
/// low.
#[derive(Debug)]
pub struct EdgeWaveform {
    channel: u8,
    builder: LogicBlockBuilder,
    high: bool,
}

impl EdgeWaveform {
    /// A waveform on `channels` channels for the line of `config`, starting at `start` with
    /// the line high or low as `high` says.
    pub fn new(config: &TimingConfig, channels: u8, start: Tick, high: bool) -> Self {
        Self {
            channel: config.channel,
            builder: LogicBlockBuilder::new(start, channels),
            high,
        }
    }

    /// The tick the next sample will have.
    pub fn position(&self) -> Tick {
        self.builder.next_tick()
    }

    /// Whether the line is high.
    pub fn is_high(&self) -> bool {
        self.high
    }

    /// Holds the line's level for `samples` samples.
    pub fn hold(&mut self, samples: u64) {
        let levels = Levels(u64::from(self.high) << self.channel);
        self.builder.push_repeated(levels, samples);
    }

    /// Changes the line's level at the next sample, which makes an edge there, and holds the
    /// new level for `samples` samples, at least one.
    pub fn toggle(&mut self, samples: u64) {
        self.high = !self.high;
        self.hold(samples.max(1));
    }

    /// Edges one after the other: the line changes level and holds it for the first interval,
    /// changes again and holds it for the second, and so on.
    pub fn toggles(&mut self, intervals: impl IntoIterator<Item = u64>) {
        for samples in intervals {
            self.toggle(samples);
        }
    }

    /// The samples so far.
    ///
    /// # Panics
    ///
    /// When no sample was produced.
    pub fn finish(self) -> LogicBlock {
        self.builder
            .finish()
            .expect("a waveform holds at least one sample")
    }

    /// The samples so far, if any.
    pub fn try_finish(self) -> Option<LogicBlock> {
        self.builder.finish().ok()
    }
}
