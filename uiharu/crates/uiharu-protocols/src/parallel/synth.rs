// SPDX-License-Identifier: Apache-2.0

//! Making parallel bus waveforms: for tests, for tools and for benchmarks.

use uiharu_core::{Levels, LogicBlock, LogicBlockBuilder, Tick};

use super::decoder::{ClockEdge, ParallelConfig};

/// Builds the samples of a parallel bus word by word.
#[derive(Debug)]
pub struct ParallelWaveform {
    config: ParallelConfig,
    builder: LogicBlockBuilder,
    /// Samples per half clock period.
    half_period: u64,
    levels: Levels,
}

impl ParallelWaveform {
    /// A waveform on `channels` channels for a bus wired as `config`, clocked at `half_period`
    /// samples per half period, starting at `start` with every data line low and the clock,
    /// if any, at the level before the edge that reads a word (low when either edge does).
    pub fn new(config: ParallelConfig, channels: u8, half_period: u64, start: Tick) -> Self {
        let mut waveform = Self {
            builder: LogicBlockBuilder::new(start, channels),
            half_period: half_period.max(1),
            levels: Levels(0),
            config,
        };
        if let Some(clock) = waveform.config.clock {
            waveform.set_line(clock.channel, clock.edge == ClockEdge::Falling);
        }
        waveform
    }

    /// The tick the next sample will have.
    pub fn position(&self) -> Tick {
        self.builder.next_tick()
    }

    fn set_line(&mut self, channel: u8, high: bool) {
        set(&mut self.levels, channel, high);
    }

    fn set_data(&mut self, value: u64) {
        for (bit, &channel) in self.config.data.iter().enumerate() {
            set(&mut self.levels, channel, value >> bit & 1 != 0);
        }
    }

    fn hold(&mut self, samples: u64) {
        self.builder.push_repeated(self.levels, samples);
    }

    /// Holds every line as it is for `samples` samples.
    pub fn idle(&mut self, samples: u64) {
        self.hold(samples);
    }

    /// Puts `value` on the data lines, bit `i` on the `i`-th line, every line changing at the
    /// same tick, and holds it for `samples` samples, at least one.
    pub fn set(&mut self, value: u64, samples: u64) {
        self.set_data(value);
        self.hold(samples.max(1));
    }

    /// Clocks `value` out: puts it on the data lines, makes the clock edge that reads it half
    /// a period later and holds another half period. A clock read on one edge goes back to
    /// its other level as the data changes; a clock read on either edge changes once per
    /// word. Without a clock, `value` is held for a whole period.
    pub fn clocked(&mut self, value: u64) {
        let Some(clock) = self.config.clock else {
            self.set(value, 2 * self.half_period);
            return;
        };
        let reading = match clock.edge {
            ClockEdge::Rising => {
                self.set_line(clock.channel, false);
                true
            }
            ClockEdge::Falling => {
                self.set_line(clock.channel, true);
                false
            }
            ClockEdge::Either => !self.levels.high(clock.channel),
        };
        self.set_data(value);
        self.hold(self.half_period);
        self.set_line(clock.channel, reading);
        self.hold(self.half_period);
    }

    /// Clocks `values` out one after the other.
    pub fn clocked_words(&mut self, values: impl IntoIterator<Item = u64>) {
        for value in values {
            self.clocked(value);
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

fn set(levels: &mut Levels, channel: u8, high: bool) {
    if high {
        levels.0 |= 1 << channel;
    } else {
        levels.0 &= !(1 << channel);
    }
}
