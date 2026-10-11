// SPDX-License-Identifier: Apache-2.0

//! Making PWM waveforms: for tests, for tools and for benchmarks.

use uiharu_core::{Levels, LogicBlock, LogicBlockBuilder, Tick};

use super::decoder::{Polarity, PwmConfig};

/// Builds a pulse-width modulated line cycle by cycle; the other channels stay low.
#[derive(Debug)]
pub struct PwmWaveform {
    channel: u8,
    polarity: Polarity,
    builder: LogicBlockBuilder,
    active: bool,
}

impl PwmWaveform {
    /// A waveform on `channels` channels for the line of `config`, inactive from `start` on.
    pub fn new(config: &PwmConfig, channels: u8, start: Tick) -> Self {
        Self {
            channel: config.channel,
            polarity: config.polarity,
            builder: LogicBlockBuilder::new(start, channels),
            active: false,
        }
    }

    /// The tick the next sample will have.
    pub fn position(&self) -> Tick {
        self.builder.next_tick()
    }

    /// Whether the line is active.
    pub fn is_active(&self) -> bool {
        self.active
    }

    /// Holds the line active, or inactive, for `samples` samples.
    pub fn level(&mut self, active: bool, samples: u64) {
        self.active = active;
        self.hold(samples);
    }

    /// Holds the line at its current level for `samples` samples.
    pub fn hold(&mut self, samples: u64) {
        let high = self.active == matches!(self.polarity, Polarity::ActiveHigh);
        let levels = Levels(u64::from(high) << self.channel);
        self.builder.push_repeated(levels, samples);
    }

    /// One cycle: the line active for `active` samples from a leading edge, then inactive for
    /// the rest of `period`. A cycle without active or without inactive time has an edge
    /// less and merges with its neighbours.
    pub fn cycle(&mut self, period: u64, active: u64) {
        self.level(true, active.min(period));
        self.level(false, period.saturating_sub(active));
    }

    /// Cycles one after the other, as `(period, active)` pairs.
    pub fn cycles(&mut self, cycles: impl IntoIterator<Item = (u64, u64)>) {
        for (period, active) in cycles {
            self.cycle(period, active);
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
