// SPDX-License-Identifier: Apache-2.0

//! Making PS/2 waveforms: for tests, for tools and for benchmarks.
//!
//! [`Ps2Waveform`] lays out the frames of both directions on the clock and data lines at a
//! chosen sample rate, with the times of a [`Timing`] — by default a 12.5 kHz clock, well inside
//! the 10 to 16.7 kHz the interface allows. [`Faults`] builds wrong start, parity and stop bits
//! into a frame.

use uiharu_core::{Levels, LogicBlock, LogicBlockBuilder, Tick, TimeBase};

use super::decoder::Ps2Config;

/// The times a waveform uses, in nanoseconds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Timing {
    /// Half a period of the device's clock: how long it stays high, and low.
    pub half_period: u64,
    /// How long the data line is set before the clock edge that is not the one reading it:
    /// before the fall when the device sends, after the fall when the host does.
    pub setup: u64,
    /// How long the host holds the clock low before it pulls the data line low.
    pub inhibit: u64,
    /// How long the host holds both low before it releases the clock.
    pub request: u64,
    /// From the host releasing the clock to the device's first clock pulse.
    pub first_clock: u64,
}

impl Timing {
    /// A 12.5 kHz clock, a 110 µs inhibit and the device answering 100 µs after the request.
    pub const NOMINAL: Timing = Timing {
        half_period: 40_000,
        setup: 20_000,
        inhibit: 110_000,
        request: 10_000,
        first_clock: 100_000,
    };
}

impl Default for Timing {
    fn default() -> Self {
        Timing::NOMINAL
    }
}

/// Wrong bits a frame is built with.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Faults {
    /// A start bit of 1 (the device's frames only: the host's start bit is its request).
    pub start: bool,
    /// The wrong parity bit.
    pub parity: bool,
    /// A stop bit of 0.
    pub stop: bool,
}

impl Faults {
    /// A frame without errors.
    pub const NONE: Faults = Faults {
        start: false,
        parity: false,
        stop: false,
    };
}

/// The ten bits after a frame's start bit, first first: the data bits least significant first,
/// the odd parity bit and the stop bit, with `faults` built in.
pub fn frame_bits(value: u8, faults: Faults) -> [bool; 10] {
    let mut bits = [false; 10];
    for (bit, slot) in bits.iter_mut().take(8).enumerate() {
        *slot = value >> bit & 1 != 0;
    }
    bits[8] = value.count_ones().is_multiple_of(2) != faults.parity;
    bits[9] = !faults.stop;
    bits
}

/// Builds the samples of a PS/2 bus one frame at a time; both lines idle high and every other
/// channel stays low.
#[derive(Debug)]
pub struct Ps2Waveform {
    builder: LogicBlockBuilder,
    clk: u8,
    data: u8,
    rate_hz: u64,
    timing: Timing,
    levels: Levels,
}

impl Ps2Waveform {
    /// A waveform on `channels` channels for a bus wired as `config`, sampled on `time_base`,
    /// starting at `start` with the bus idle, with the nominal timing.
    pub fn new(config: Ps2Config, channels: u8, time_base: TimeBase, start: Tick) -> Self {
        Self {
            builder: LogicBlockBuilder::new(start, channels),
            clk: config.clk,
            data: config.data,
            rate_hz: time_base.rate_hz,
            timing: Timing::NOMINAL,
            levels: Levels((1 << config.clk) | (1 << config.data)),
        }
    }

    /// The times in use.
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

    /// Sets the clock line's level.
    pub fn set_clock(&mut self, high: bool) {
        self.set(self.clk, high);
    }

    /// Sets the data line's level.
    pub fn set_data(&mut self, high: bool) {
        self.set(self.data, high);
    }

    fn set(&mut self, channel: u8, high: bool) {
        if high {
            self.levels.0 |= 1 << channel;
        } else {
            self.levels.0 &= !(1 << channel);
        }
    }

    /// Holds both lines as they are for `samples` samples.
    pub fn hold(&mut self, samples: u64) {
        self.builder.push_repeated(self.levels, samples);
    }

    fn hold_ns(&mut self, ns: u64) {
        let samples = self.ticks(ns);
        self.hold(samples);
    }

    /// Releases both lines and holds them high for `us` microseconds.
    pub fn idle_us(&mut self, us: u64) {
        self.set_clock(true);
        self.set_data(true);
        self.hold_ns(us.saturating_mul(1_000));
    }

    /// The device clocks out `bits`, each set on the data line while the clock is high and read
    /// on its fall: a clock period per bit, the clock high again after the last.
    pub fn device_bits(&mut self, bits: impl IntoIterator<Item = bool>) {
        let Timing {
            half_period, setup, ..
        } = self.timing;
        for bit in bits {
            self.set_data(bit);
            self.hold_ns(setup);
            self.set_clock(false);
            self.hold_ns(half_period);
            self.set_clock(true);
            self.hold_ns(half_period.saturating_sub(setup));
        }
    }

    /// A frame the device sends: start bit, `value`, parity and stop bit, then the data line
    /// released.
    pub fn device_byte(&mut self, value: u8) {
        self.device_byte_with(value, Faults::NONE);
    }

    /// A frame the device sends, with `faults` built in.
    pub fn device_byte_with(&mut self, value: u8, faults: Faults) {
        let mut bits = vec![faults.start];
        bits.extend(frame_bits(value, faults));
        self.device_bits(bits);
        self.set_data(true);
    }

    /// The host holds the clock low for `us` microseconds, then releases it.
    pub fn inhibit_us(&mut self, us: u64) {
        self.set_clock(false);
        self.hold_ns(us.saturating_mul(1_000));
        self.set_clock(true);
    }

    /// The host's request to send: the clock low for the inhibit time, the data line low, the
    /// clock released, then the wait for the device's first pulse.
    pub fn request_to_send(&mut self) {
        let Timing {
            inhibit,
            request,
            first_clock,
            ..
        } = self.timing;
        self.set_clock(false);
        self.hold_ns(inhibit);
        self.set_data(false);
        self.hold_ns(request);
        self.set_clock(true);
        self.hold_ns(first_clock);
    }

    /// The device clocks `bits` in from the host: the host sets each one after a fall, the
    /// device reads it on the rise.
    pub fn host_bits(&mut self, bits: impl IntoIterator<Item = bool>) {
        let Timing {
            half_period, setup, ..
        } = self.timing;
        for bit in bits {
            self.set_clock(false);
            self.hold_ns(setup);
            self.set_data(bit);
            self.hold_ns(half_period.saturating_sub(setup));
            self.set_clock(true);
            self.hold_ns(half_period);
        }
    }

    /// The device's last clock pulse after the stop bit, the data line pulled low before it when
    /// the device acknowledges, then both lines released.
    pub fn acknowledge(&mut self, ack: bool) {
        let Timing {
            half_period, setup, ..
        } = self.timing;
        self.set_data(!ack);
        self.hold_ns(setup);
        self.set_clock(false);
        self.hold_ns(half_period);
        self.set_clock(true);
        self.hold_ns(half_period / 2);
        self.set_data(true);
        self.hold_ns(half_period / 2);
    }

    /// A frame the host sends, acknowledged.
    pub fn host_byte(&mut self, value: u8) {
        self.host_byte_with(value, Faults::NONE, true);
    }

    /// A frame the host sends, with `faults` built in, acknowledged or not.
    pub fn host_byte_with(&mut self, value: u8, faults: Faults, ack: bool) {
        self.request_to_send();
        self.host_bits(frame_bits(value, faults));
        self.acknowledge(ack);
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
    fn frames_carry_odd_parity_and_a_stop_bit() {
        // 0x1c, 0001 1100: three ones, so the parity bit is 0.
        assert_eq!(
            frame_bits(0x1c, Faults::NONE),
            [
                false, false, true, true, true, false, false, false, false, true
            ]
        );
        let bad = frame_bits(
            0x1c,
            Faults {
                parity: true,
                stop: true,
                ..Faults::NONE
            },
        );
        assert!(bad[8] && !bad[9]);
        assert!(frame_bits(0, Faults::NONE)[8]);
    }

    #[test]
    fn the_device_sets_data_while_the_clock_is_high() {
        let config = Ps2Config::default();
        let mut waveform = Ps2Waveform::new(config, 2, TimeBase::new(1_000_000), Tick(0));
        waveform.device_bits([false, true]);
        let block = waveform.finish();
        assert_eq!(block.len(), 160);
        // Data low, then the clock falls 20 µs later and rises 40 µs after that.
        assert_eq!(block.levels(Tick(0)), Some(Levels(0b01)));
        assert_eq!(block.levels(Tick(20)), Some(Levels(0b00)));
        assert_eq!(block.levels(Tick(60)), Some(Levels(0b01)));
        assert_eq!(block.levels(Tick(80)), Some(Levels(0b11)));
        assert_eq!(block.levels(Tick(100)), Some(Levels(0b10)));
    }
}
