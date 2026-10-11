// SPDX-License-Identifier: Apache-2.0

//! Making Microwire waveforms: for tests, for tools and for benchmarks.
//!
//! [`MicrowireWaveform`] lays out transactions bit by bit: the controller sets DI while SK is
//! low, the device changes DO a short delay after SK rises, as a 93Cxx EEPROM does. Generators
//! of device traffic (`eeprom93xx::synth`) build bit lists for it rather than samples.

use uiharu_core::{Levels, LogicBlock, LogicBlockBuilder, Tick};

use super::decoder::MicrowireConfig;

/// Builds the samples of a Microwire bus one transaction at a time; SK idles low, chip select
/// deasserted, DI low and DO high (released, pulled up).
#[derive(Debug)]
pub struct MicrowireWaveform {
    config: MicrowireConfig,
    builder: LogicBlockBuilder,
    /// Samples per half clock period.
    half_period: u64,
    /// Samples from SK's rising edge to DO's change.
    out_delay: u64,
    levels: Levels,
}

impl MicrowireWaveform {
    /// A waveform on `channels` channels for a bus wired as `config`, clocked at `half_period`
    /// samples per half period, starting at `start` with the bus idle. DO changes one sample
    /// after each rising edge.
    pub fn new(config: MicrowireConfig, channels: u8, half_period: u64, start: Tick) -> Self {
        let mut waveform = Self {
            config,
            builder: LogicBlockBuilder::new(start, channels),
            half_period: half_period.max(2),
            out_delay: 1,
            levels: Levels(0),
        };
        waveform.set(config.cs, config.cs_active_low);
        waveform.set(config.r#do, true);
        waveform
    }

    /// The tick the next sample will have.
    pub fn position(&self) -> Tick {
        self.builder.next_tick()
    }

    /// Changes DO `samples` after each rising edge from here on; less than half a period.
    pub fn set_out_delay(&mut self, samples: u64) {
        self.out_delay = samples.min(self.half_period - 1);
    }

    fn set(&mut self, channel: Option<u8>, high: bool) {
        if let Some(channel) = channel {
            if high {
                self.levels.0 |= 1 << channel;
            } else {
                self.levels.0 &= !(1 << channel);
            }
        }
    }

    /// Holds the bus as it is for `samples` samples.
    pub fn idle(&mut self, samples: u64) {
        self.builder.push_repeated(self.levels, samples);
    }

    /// Drives DO, as a device showing its ready or busy state does.
    pub fn set_out(&mut self, high: bool) {
        self.set(self.config.r#do, high);
    }

    /// Asserts chip select, then waits half a period before the first clock pulse.
    pub fn select(&mut self) {
        self.set(self.config.cs, !self.config.cs_active_low);
        self.idle(self.half_period);
    }

    /// Releases chip select after half a period of idle clock; DO is released with it.
    pub fn release(&mut self) {
        self.idle(self.half_period);
        self.set(self.config.cs, self.config.cs_active_low);
        self.set(self.config.r#do, true);
    }

    /// One clock pulse: DI set while SK is low, SK high, DO changed after the delay, SK low.
    pub fn bit(&mut self, di: bool, out: bool) {
        self.set(Some(self.config.di), di);
        self.idle(self.half_period);
        self.set(Some(self.config.sk), true);
        self.idle(self.out_delay);
        self.set(self.config.r#do, out);
        self.idle(self.half_period - self.out_delay);
        self.set(Some(self.config.sk), false);
    }

    /// Clock pulses, DI and DO pairwise.
    pub fn bits(&mut self, bits: impl IntoIterator<Item = (bool, bool)>) {
        for (di, out) in bits {
            self.bit(di, out);
        }
    }

    /// A whole transaction: select, the bits, release.
    pub fn transaction(&mut self, bits: impl IntoIterator<Item = (bool, bool)>) {
        self.select();
        self.bits(bits);
        self.release();
    }

    /// The samples so far; at least one must have been produced.
    pub fn finish(self) -> LogicBlock {
        self.builder.finish().unwrap_or_else(|_| {
            LogicBlockBuilder::new(Tick::ZERO, 1)
                .finish()
                .expect("no samples were produced")
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_in_is_set_before_the_rise_and_data_out_after_it() {
        let config = MicrowireConfig::default();
        let mut waveform = MicrowireWaveform::new(config, 4, 4, Tick(0));
        waveform.transaction([(true, false)]);
        let block = waveform.finish();
        // CS high from 0; DI set at 4, SK rising at 8, DO low from 9, SK falling at 12; CS
        // released at 16.
        assert_eq!(block.len(), 16);
        assert_eq!(block.levels(Tick(0)), Some(Levels(0b1001)));
        assert_eq!(block.levels(Tick(4)), Some(Levels(0b1101)));
        assert_eq!(block.levels(Tick(8)), Some(Levels(0b1111)));
        assert_eq!(block.levels(Tick(9)), Some(Levels(0b0111)));
        assert_eq!(block.levels(Tick(12)), Some(Levels(0b0101)));
    }
}
