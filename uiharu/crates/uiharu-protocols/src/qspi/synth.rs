// SPDX-License-Identifier: Apache-2.0

//! Fixed-lane SDR waveform builder; retains samples only in this test utility.
use super::QspiConfig;
use uiharu_core::{Levels, LogicBlock, LogicBlockBuilder, Tick};

/// Builds clocked lane groups using the decoder's wiring and mode.
#[derive(Debug)]
pub struct QspiWaveform {
    config: QspiConfig,
    builder: LogicBlockBuilder,
    levels: Levels,
    half: u64,
}
impl QspiWaveform {
    /// New idle bus; configuration must be valid and channels must include its wiring.
    pub fn new(config: QspiConfig, channels: u8, half_period: u64, start: Tick) -> Self {
        config.validate().expect("valid QSPI configuration");
        let mut this = Self {
            config,
            builder: LogicBlockBuilder::new(start, channels),
            levels: Levels(0),
            half: half_period.max(1),
        };
        this.set(this.config.clk, this.config.idle_high());
        if let Some(cs) = this.config.cs {
            this.set(cs, this.config.cs_active_low);
        }
        this
    }
    fn set(&mut self, c: u8, high: bool) {
        if high {
            self.levels.0 |= 1u64 << c;
        } else {
            self.levels.0 &= !(1u64 << c);
        }
    }
    /// Tick of the next sample.
    pub fn position(&self) -> Tick {
        self.builder.next_tick()
    }
    /// Hold current levels.
    pub fn idle(&mut self, samples: u64) {
        self.builder.push_repeated(self.levels, samples);
    }
    /// Assert CS, with setup before the first clock.
    pub fn select(&mut self) {
        if let Some(cs) = self.config.cs {
            self.set(cs, !self.config.cs_active_low);
        }
        self.idle(self.half);
    }
    /// Release CS after holding idle clock for a half period.
    pub fn release(&mut self) {
        self.idle(self.half);
        if let Some(cs) = self.config.cs {
            self.set(cs, self.config.cs_active_low);
        }
    }
    /// Clock one lane group, low bit on IO0. Useful for incomplete words.
    pub fn group(&mut self, value: u8) {
        let idle = self.config.idle_high();
        if self.config.mode & 1 != 0 {
            self.set(self.config.clk, !idle);
        }
        for i in 0..self.config.data.len() {
            self.set(self.config.data[i], value & (1 << i) != 0);
        }
        self.idle(self.half);
        self.set(
            self.config.clk,
            if self.config.mode & 1 != 0 {
                idle
            } else {
                !idle
            },
        );
        self.idle(self.half);
        self.set(self.config.clk, idle);
    }
    /// Clock a configured word; bits above its width are ignored.
    pub fn word(&mut self, value: u32) {
        let lanes = self.config.data.len() as u8;
        for i in 0..self.config.word_bits / lanes {
            let shift = if self.config.lsb_first {
                i * lanes
            } else {
                self.config.word_bits - (i + 1) * lanes
            };
            self.group((value >> shift) as u8);
        }
    }
    /// Finish after producing at least one sample.
    pub fn finish(self) -> LogicBlock {
        self.builder.finish().expect("waveform contains samples")
    }
}
