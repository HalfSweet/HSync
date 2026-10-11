// SPDX-License-Identifier: Apache-2.0

//! Making AM230x waveforms: for tests, for tools and for benchmarks.
//!
//! [`Am230xWaveform`] lays out the exchange of a reading on the data line at a chosen sample
//! rate, with the pulse lengths of a [`Timing`] — by default the datasheets' typical values for
//! the configured model — and lets tests stretch any of them.

use uiharu_core::{Levels, LogicBlock, LogicBlockBuilder, Tick, TimeBase};

use super::decoder::{Am230xConfig, Model};

/// The pulse lengths a waveform uses, in nanoseconds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Timing {
    /// The host's start signal.
    pub start: u64,
    /// The host's release before the sensor answers.
    pub release: u64,
    /// The sensor's response low.
    pub response_low: u64,
    /// The sensor's response high.
    pub response_high: u64,
    /// The low before each bit.
    pub bit_low: u64,
    /// The high of a 0.
    pub zero_high: u64,
    /// The high of a 1.
    pub one_high: u64,
    /// The sensor's final low.
    pub end_low: u64,
}

impl Timing {
    /// The typical times of the datasheet of `model`.
    pub const fn of(model: Model) -> Timing {
        Timing {
            start: match model {
                Model::Am2302 => 1_000_000,
                Model::Dht11 => 18_000_000,
            },
            release: 30_000,
            response_low: 80_000,
            response_high: 80_000,
            bit_low: 50_000,
            zero_high: 27_000,
            one_high: 70_000,
            end_low: 50_000,
        }
    }
}

/// The five bytes of a reading: the four data bytes and their sum modulo 256.
pub fn with_checksum(data: [u8; 4]) -> [u8; 5] {
    let sum = data.iter().fold(0u8, |sum, byte| sum.wrapping_add(*byte));
    [data[0], data[1], data[2], data[3], sum]
}

/// Builds the samples of a sensor's data line; the line idles high and every other channel
/// stays low.
#[derive(Debug)]
pub struct Am230xWaveform {
    builder: LogicBlockBuilder,
    data: u8,
    rate_hz: u64,
    timing: Timing,
}

impl Am230xWaveform {
    /// A waveform on `channels` channels for a line configured as `config`, sampled on
    /// `time_base`, starting at `start` with the line high, with the typical times of the
    /// configured model.
    pub fn new(config: Am230xConfig, channels: u8, time_base: TimeBase, start: Tick) -> Self {
        Self {
            builder: LogicBlockBuilder::new(start, channels),
            data: config.data,
            rate_hz: time_base.rate_hz,
            timing: Timing::of(config.model),
        }
    }

    /// The pulse lengths in use.
    pub fn timing(&self) -> Timing {
        self.timing
    }

    /// Uses `timing` from here on.
    pub fn set_timing(&mut self, timing: Timing) {
        self.timing = timing;
    }

    /// The tick the next sample will have.
    pub fn position(&self) -> Tick {
        self.builder.next_tick()
    }

    /// `ns` nanoseconds in samples, rounded down.
    pub fn ticks(&self, ns: u64) -> u64 {
        let ticks = u128::from(ns) * u128::from(self.rate_hz) / 1_000_000_000;
        u64::try_from(ticks).unwrap_or(u64::MAX)
    }

    /// Holds the line low for `ns` nanoseconds.
    pub fn low(&mut self, ns: u64) {
        let samples = self.ticks(ns);
        self.builder.push_repeated(Levels(0), samples);
    }

    /// Holds the line high for `ns` nanoseconds.
    pub fn high(&mut self, ns: u64) {
        let samples = self.ticks(ns);
        self.builder.push_repeated(Levels(1 << self.data), samples);
    }

    /// Holds the line high for `us` microseconds.
    pub fn idle_us(&mut self, us: u64) {
        self.high(us.saturating_mul(1_000));
    }

    /// The host's start signal and release.
    pub fn start_signal(&mut self) {
        self.low(self.timing.start);
        self.high(self.timing.release);
    }

    /// The sensor's response: low, then high.
    pub fn response(&mut self) {
        self.low(self.timing.response_low);
        self.high(self.timing.response_high);
    }

    /// One bit: the low, then the high of a 0 or a 1.
    pub fn bit(&mut self, bit: bool) {
        self.low(self.timing.bit_low);
        let high = if bit {
            self.timing.one_high
        } else {
            self.timing.zero_high
        };
        self.high(high);
    }

    /// The bits of `bytes`, most significant bit first.
    pub fn bytes(&mut self, bytes: &[u8]) {
        for byte in bytes {
            for bit in (0..8).rev() {
                self.bit(byte >> bit & 1 != 0);
            }
        }
    }

    /// The sensor's final low; the line is released after it.
    pub fn end(&mut self) {
        self.low(self.timing.end_low);
    }

    /// A whole exchange: start signal, response, the five bytes and the final low.
    pub fn reading(&mut self, bytes: [u8; 5]) {
        self.start_signal();
        self.response();
        self.bytes(&bytes);
        self.end();
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
    fn checksums_wrap_around() {
        assert_eq!(
            with_checksum([0x02, 0x8c, 0x01, 0x5f]),
            [2, 140, 1, 95, 0xee]
        );
        assert_eq!(
            with_checksum([0xff, 0xff, 0x00, 0x03]),
            [255, 255, 0, 3, 0x01]
        );
    }

    #[test]
    fn a_reading_takes_its_typical_time() {
        let config = Am230xConfig::default();
        let mut waveform = Am230xWaveform::new(config, 1, TimeBase::new(1_000_000), Tick(0));
        waveform.reading([0, 0, 0, 0, 0]);
        // 1 ms start, 30 µs release, 80 + 80 µs response, forty 0s of 77 µs, 50 µs end.
        assert_eq!(waveform.position(), Tick(1_000 + 30 + 160 + 40 * 77 + 50));
        let block = waveform.finish();
        assert_eq!(block.level(0, Tick(999)), Some(false));
        assert_eq!(block.level(0, Tick(1_000)), Some(true));
        assert_eq!(block.level(0, Tick(1_030)), Some(false));
    }
}
