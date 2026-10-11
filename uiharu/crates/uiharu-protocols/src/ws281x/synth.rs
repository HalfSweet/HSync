// SPDX-License-Identifier: Apache-2.0

//! Making WS281x waveforms: for tests, for tools and for benchmarks.

use uiharu_core::{Levels, LogicBlock, LogicBlockBuilder, Tick};

use super::decoder::Ws281xConfig;

/// How long each half of a bit lasts, in samples.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BitTiming {
    /// The high of a 0 bit.
    pub zero_high: u64,
    /// The low of a 0 bit.
    pub zero_low: u64,
    /// The high of a 1 bit.
    pub one_high: u64,
    /// The low of a 1 bit.
    pub one_low: u64,
}

impl BitTiming {
    /// The datasheets' typical timing at `rate_hz` samples per second: a 0 high for 0.4 µs then
    /// low for 0.85 µs, a 1 high for 0.8 µs then low for 0.45 µs, each rounded to the nearest
    /// sample and at least one.
    pub fn typical(rate_hz: u64) -> Self {
        let samples = |ns: u64| {
            let samples = (u128::from(rate_hz) * u128::from(ns) + 500_000_000) / 1_000_000_000;
            u64::try_from(samples).unwrap_or(u64::MAX).max(1)
        };
        Self {
            zero_high: samples(400),
            zero_low: samples(850),
            one_high: samples(800),
            one_low: samples(450),
        }
    }
}

/// Builds the samples of a WS281x data line bit by bit; the line idles low, and the other
/// channels stay low.
#[derive(Debug)]
pub struct Ws281xWaveform {
    data: u8,
    timing: BitTiming,
    builder: LogicBlockBuilder,
}

impl Ws281xWaveform {
    /// A waveform on `channels` channels for the line of `config`, its bits timed as `timing`
    /// says, starting at `start` with the line low.
    pub fn new(config: &Ws281xConfig, channels: u8, timing: BitTiming, start: Tick) -> Self {
        Self {
            data: config.data,
            timing,
            builder: LogicBlockBuilder::new(start, channels),
        }
    }

    /// The tick the next sample will have.
    pub fn position(&self) -> Tick {
        self.builder.next_tick()
    }

    /// Holds the line low for `samples` samples: idle time, or a reset when it is long enough.
    pub fn low(&mut self, samples: u64) {
        self.builder.push_repeated(Levels(0), samples);
    }

    /// Holds the line high for `samples` samples.
    pub fn high(&mut self, samples: u64) {
        self.builder
            .push_repeated(Levels(1u64 << self.data), samples);
    }

    /// A pulse: high for `high` samples, then low for `low`.
    pub fn pulse(&mut self, high: u64, low: u64) {
        self.high(high);
        self.low(low);
    }

    /// One bit with the configured timing.
    pub fn bit(&mut self, one: bool) {
        let timing = self.timing;
        if one {
            self.pulse(timing.one_high, timing.one_low);
        } else {
            self.pulse(timing.zero_high, timing.zero_low);
        }
    }

    /// The `count` low bits of `value`, the most significant first.
    pub fn bits(&mut self, value: u32, count: u8) {
        for shift in (0..u32::from(count.min(32))).rev() {
            self.bit(value >> shift & 1 != 0);
        }
    }

    /// One byte, the most significant bit first.
    pub fn byte(&mut self, value: u8) {
        self.bits(u32::from(value), 8);
    }

    /// Bytes one after the other: the LEDs of a frame, each LED's bytes in wire order.
    pub fn bytes(&mut self, bytes: impl IntoIterator<Item = u8>) {
        for byte in bytes {
            self.byte(byte);
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

#[cfg(test)]
mod tests {
    use super::*;

    fn levels(block: &LogicBlock, channel: u8) -> String {
        (block.start().0..block.end().0)
            .map(|tick| {
                if block.level(channel, Tick(tick)).unwrap_or(false) {
                    '1'
                } else {
                    '0'
                }
            })
            .collect()
    }

    #[test]
    fn typical_timing_follows_the_datasheets() {
        assert_eq!(
            BitTiming::typical(20_000_000),
            BitTiming {
                zero_high: 8,
                zero_low: 17,
                one_high: 16,
                one_low: 9
            }
        );
        // 4 MHz: 1.6, 3.4, 3.2 and 1.8 samples, to the nearest.
        assert_eq!(
            BitTiming::typical(4_000_000),
            BitTiming {
                zero_high: 2,
                zero_low: 3,
                one_high: 3,
                one_low: 2
            }
        );
        assert_eq!(BitTiming::typical(1).zero_high, 1);
    }

    #[test]
    fn bits_are_highs_of_two_lengths() {
        let config = Ws281xConfig {
            data: 1,
            ..Ws281xConfig::default()
        };
        let timing = BitTiming {
            zero_high: 1,
            zero_low: 3,
            one_high: 3,
            one_low: 1,
        };
        let mut waveform = Ws281xWaveform::new(&config, 2, timing, Tick(10));
        waveform.low(2);
        waveform.bits(0b101, 3);
        waveform.low(1);
        assert_eq!(waveform.position(), Tick(25));
        let block = waveform.finish();
        assert_eq!(block.start(), Tick(10));
        assert_eq!(levels(&block, 1), "001110100011100");
        assert_eq!(levels(&block, 0), "0".repeat(15));
        // A byte goes out most significant bit first.
        let mut waveform = Ws281xWaveform::new(&config, 2, timing, Tick::ZERO);
        waveform.byte(0x81);
        assert_eq!(
            levels(&waveform.finish(), 1),
            format!("1110{}1110", "1000".repeat(6))
        );
    }
}
