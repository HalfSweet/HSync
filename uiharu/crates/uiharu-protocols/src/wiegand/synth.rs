// SPDX-License-Identifier: Apache-2.0

//! Making Wiegand waveforms: for tests, for tools and for benchmarks.
//!
//! [`WiegandWaveform`] lays out pulses on DATA0 and DATA1 at a chosen sample rate, with the
//! pulse width and spacing of a [`Timing`] — by default the nominal 50 µs pulses 1 ms apart.
//! [`card26`] and [`bits_of`] build the bit lists of frames.

use uiharu_core::{Levels, LogicBlock, LogicBlockBuilder, Tick, TimeBase};

use super::decoder::WiegandConfig;

/// The pulse width and spacing a waveform uses, in nanoseconds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Timing {
    /// How long a line is pulled low for a bit.
    pub pulse: u64,
    /// From one pulse's fall to the next one's.
    pub interval: u64,
}

impl Timing {
    /// The nominal timing: 50 µs pulses, 1 ms apart.
    pub const NOMINAL: Timing = Timing {
        pulse: 50_000,
        interval: 1_000_000,
    };
}

impl Default for Timing {
    fn default() -> Self {
        Timing::NOMINAL
    }
}

/// Builds the samples of a Wiegand interface one pulse at a time; both lines idle high and
/// every other channel stays low.
#[derive(Debug)]
pub struct WiegandWaveform {
    builder: LogicBlockBuilder,
    d0: u8,
    d1: u8,
    rate_hz: u64,
    timing: Timing,
}

impl WiegandWaveform {
    /// A waveform on `channels` channels for lines wired as `config`, sampled on `time_base`,
    /// starting at `start` with both lines high, with the nominal timing.
    pub fn new(config: WiegandConfig, channels: u8, time_base: TimeBase, start: Tick) -> Self {
        Self {
            builder: LogicBlockBuilder::new(start, channels),
            d0: config.d0,
            d1: config.d1,
            rate_hz: time_base.rate_hz,
            timing: Timing::NOMINAL,
        }
    }

    /// The pulse width and spacing in use.
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

    fn levels(&self, d0_low: bool, d1_low: bool) -> Levels {
        let mut levels = 0u64;
        if !d0_low {
            levels |= 1 << self.d0;
        }
        if !d1_low {
            levels |= 1 << self.d1;
        }
        Levels(levels)
    }

    /// Holds both lines high for `samples` samples.
    pub fn idle(&mut self, samples: u64) {
        let high = self.levels(false, false);
        self.builder.push_repeated(high, samples);
    }

    /// Holds both lines high for `us` microseconds.
    pub fn idle_us(&mut self, us: u64) {
        let samples = self.ticks(us.saturating_mul(1_000));
        self.idle(samples);
    }

    /// Pulls DATA0, DATA1 or both low for `samples` samples, then releases them.
    pub fn pulse(&mut self, d0_low: bool, d1_low: bool, samples: u64) {
        let low = self.levels(d0_low, d1_low);
        self.builder.push_repeated(low, samples);
    }

    /// One bit: a pulse on DATA1 for a 1 or DATA0 for a 0, then both lines high until the
    /// next bit's time.
    pub fn bit(&mut self, bit: bool) {
        let pulse = self.ticks(self.timing.pulse);
        let rest = self.ticks(self.timing.interval).saturating_sub(pulse);
        self.pulse(!bit, bit, pulse);
        self.idle(rest);
    }

    /// The bits, first first, a pulse each; the lines stay high after the last one until the
    /// next bit's time, and the frame ends with whatever idle time follows.
    pub fn bits(&mut self, bits: impl IntoIterator<Item = bool>) {
        for bit in bits {
            self.bit(bit);
        }
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

/// The lowest `count` bits of `value`, most significant first: the order a reader sends a
/// number in.
pub fn bits_of(value: u64, count: u32) -> Vec<bool> {
    (0..count.min(64))
        .rev()
        .map(|bit| value >> bit & 1 != 0)
        .collect()
}

/// The 26 bits of the standard format: an even parity bit over the next 12 bits, the 8-bit
/// facility code, the 16-bit card number and an odd parity bit over the 12 bits before it.
pub fn card26(facility: u8, card: u16) -> Vec<bool> {
    let data = (u32::from(facility) << 16) | u32::from(card);
    let first = data >> 12;
    let last = data & 0xfff;
    let even = first.count_ones() % 2 == 1;
    let odd = last.count_ones() % 2 == 0;
    let mut bits = vec![even];
    bits.extend(bits_of(u64::from(data), 24));
    bits.push(odd);
    bits
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_go_out_most_significant_bit_first() {
        assert_eq!(bits_of(0b1011, 4), vec![true, false, true, true]);
        assert_eq!(bits_of(1, 3), vec![false, false, true]);
        assert!(bits_of(5, 0).is_empty());
    }

    #[test]
    fn a_standard_card_carries_its_parities() {
        // Facility 123 (0111 1011), card 12345 (0011 0000 0011 1001): the first twelve bits
        // hold eight ones, so the even parity bit is 0; the last twelve hold four, so the odd
        // parity bit is 1.
        let bits = card26(123, 12_345);
        let text: String = bits
            .iter()
            .map(|bit| if *bit { '1' } else { '0' })
            .collect();
        assert_eq!(text, "00111101100110000001110011");
    }

    #[test]
    fn bits_are_pulses_on_their_lines() {
        let config = WiegandConfig::default();
        let mut waveform = WiegandWaveform::new(config, 2, TimeBase::new(1_000_000), Tick(0));
        waveform.bits([false, true]);
        let block = waveform.finish();
        assert_eq!(block.len(), 2_000);
        // A 0 pulls DATA0 low for 50 µs, a 1 DATA1, 1 ms later.
        assert_eq!(block.levels(Tick(0)), Some(Levels(0b10)));
        assert_eq!(block.levels(Tick(49)), Some(Levels(0b10)));
        assert_eq!(block.levels(Tick(50)), Some(Levels(0b11)));
        assert_eq!(block.levels(Tick(1_000)), Some(Levels(0b01)));
        assert_eq!(block.levels(Tick(1_050)), Some(Levels(0b11)));
    }
}
