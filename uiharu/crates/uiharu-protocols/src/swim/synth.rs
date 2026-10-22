// SPDX-License-Identifier: Apache-2.0

//! Nominal UM0470 SWIM waveforms, independent of the decoder's pulse classification.

use super::{Direction, Speed, SwimConfig};
use uiharu_core::{Levels, LogicBlock, LogicBlockBuilder, Tick, TimeBase};

/// A waveform with bounded operations per bit; other channels remain low.
#[derive(Debug)]
pub struct SwimWaveform {
    builder: LogicBlockBuilder,
    config: SwimConfig,
    rate: u64,
    speed: Speed,
}
impl SwimWaveform {
    /// Starts a waveform; callers add an initial high idle before a packet.
    pub fn new(config: SwimConfig, channels: u8, time_base: TimeBase, start: Tick) -> Self {
        Self {
            builder: LogicBlockBuilder::new(start, channels),
            config,
            rate: time_base.rate_hz,
            speed: config.speed,
        }
    }
    /// Tick of the next sample.
    pub fn position(&self) -> Tick {
        self.builder.next_tick()
    }
    /// Uses this speed for subsequent bits.
    pub fn set_speed(&mut self, speed: Speed) {
        self.speed = speed;
    }
    /// A high interval in samples.
    pub fn idle(&mut self, ticks: u64) {
        self.builder
            .push_repeated(Levels(1u64 << self.config.data), ticks);
    }
    /// A low interval in samples; no automatic release.
    pub fn low(&mut self, ticks: u64) {
        self.builder.push_repeated(Levels(0), ticks);
    }
    /// SWIM clock periods converted to samples, rounded down.
    pub fn clock_ticks(&self, clocks: u64) -> u64 {
        u64::try_from(
            u128::from(clocks) * u128::from(self.rate) / u128::from(self.config.clock_hz.max(1)),
        )
        .unwrap_or(u64::MAX)
    }
    /// A bit using the documented 2/20 or 2/8 pulse durations.
    pub fn bit(&mut self, value: bool) {
        let long = u64::from(self.speed.clocks()) - 2;
        let low = self.clock_ticks(if value { 2 } else { long });
        let high = self.clock_ticks(if value { long } else { 2 });
        self.low(low.max(1));
        self.idle(high.max(1));
    }
    /// Bits already in wire order, useful for specification-derived vectors and faults.
    pub fn bits(&mut self, bits: impl IntoIterator<Item = bool>) {
        for bit in bits {
            self.bit(bit);
        }
    }
    /// One physical packet with explicit payload length, parity fault and receiver ACK.
    pub fn packet(&mut self, target: bool, value: u8, bits: u8, bad_parity: bool, ack: bool) {
        self.bit(target);
        for bit in (0..bits).rev() {
            self.bit(value & (1 << bit) != 0);
        }
        self.bit((!value.count_ones().is_multiple_of(2)) ^ bad_parity);
        self.bit(ack);
    }
    /// A three-bit host command.
    pub fn command(&mut self, value: u8, ack: bool) {
        self.packet(false, value & 7, 3, false, ack);
    }
    /// An eight-bit payload packet.
    pub fn byte(&mut self, direction: Direction, value: u8, ack: bool) {
        self.packet(direction == Direction::TargetToHost, value, 8, false, ack);
    }
    /// A nominal communication reset/synchronization pulse; subsequent traffic is low speed.
    pub fn reset(&mut self) {
        self.low(self.clock_ticks(128));
        self.idle(self.clock_ticks(4));
        self.speed = Speed::Low;
    }
    /// Entry prelude and four 1 kHz followed by four 2 kHz pulses; ends high.
    pub fn entry(&mut self) {
        let rate = self.rate;
        let us = |n: u64| {
            u64::try_from(u128::from(n) * u128::from(rate) / 1_000_000).unwrap_or(u64::MAX)
        };
        self.low(us(16));
        self.idle(us(500));
        for half in [500, 500, 500, 500, 250, 250, 250, 250] {
            self.low(us(half));
            self.idle(us(half));
        }
        self.speed = Speed::Low;
    }
    /// Finishes the captured samples.
    pub fn finish(self) -> LogicBlock {
        self.builder.finish().expect("valid SWIM waveform")
    }
}
