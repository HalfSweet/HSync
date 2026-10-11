// SPDX-License-Identifier: Apache-2.0

//! Making I²S waveforms: for tests, for tools and for benchmarks.

use uiharu_core::{Levels, LogicBlock, LogicBlockBuilder, Tick};

use super::decoder::{Format, I2sConfig, MAX_WORD_BITS, SampleEdge};
use super::events::Channel;

/// Builds the samples of an I²S bus half-period by half-period, as a transmitter drives it:
/// WS and SD change with the clock edge the receiver does not sample on, half a clock period
/// before the sampling edge that reads them.
#[derive(Debug)]
pub struct I2sWaveform {
    config: I2sConfig,
    builder: LogicBlockBuilder,
    /// Samples per half clock period.
    half_period: u64,
    levels: Levels,
    /// The channel WS was last set for.
    channel: Channel,
    /// In the I²S format, the bit the next clock carries: the previous clock's bit of the
    /// word, one clock late.
    carry: bool,
}

impl I2sWaveform {
    /// A waveform on `channels` channels for a bus wired as `config`, clocked at
    /// `half_period` samples per half clock period, starting at `start` with the clock
    /// stopped after a sampling edge and WS on the right channel, so that a left half-period
    /// begins with a WS change.
    pub fn new(config: I2sConfig, channels: u8, half_period: u64, start: Tick) -> Self {
        let mut waveform = Self {
            builder: LogicBlockBuilder::new(start, channels),
            half_period: half_period.max(1),
            levels: Levels(0),
            channel: Channel::Right,
            carry: false,
            config,
        };
        let sampled = waveform.sampled();
        waveform.set(waveform.config.sck, sampled);
        waveform.set(waveform.config.ws, waveform.config.ws_high(Channel::Right));
        waveform
    }

    /// The tick the next sample will have.
    pub fn position(&self) -> Tick {
        self.builder.next_tick()
    }

    /// The clock's level after a sampling edge.
    fn sampled(&self) -> bool {
        self.config.sample_edge == SampleEdge::Rising
    }

    fn set(&mut self, channel: u8, high: bool) {
        if high {
            self.levels.0 |= 1 << channel;
        } else {
            self.levels.0 &= !(1 << channel);
        }
    }

    fn hold(&mut self, samples: u64) {
        self.builder.push_repeated(self.levels, samples);
    }

    /// Holds every line as it is for `samples` samples, the clock stopped.
    pub fn idle(&mut self, samples: u64) {
        self.hold(samples);
    }

    /// One clock period: WS set for `channel` and `bit` put on SD with the edge the receiver
    /// does not sample on, then the sampling edge half a period later. The bit goes on the
    /// line as it is, whatever the format.
    pub fn clock(&mut self, channel: Channel, bit: bool) {
        let sampled = self.sampled();
        self.set(self.config.sck, !sampled);
        self.set(self.config.ws, self.config.ws_high(channel));
        self.set(self.config.sd, bit);
        self.hold(self.half_period);
        self.set(self.config.sck, sampled);
        self.hold(self.half_period);
        self.channel = channel;
    }

    /// One half-period of `slots` clocks for `channel`, carrying the low `bits` bits of
    /// `value` (at most 32), MSB first, where the configured format puts them: left-justified
    /// from the first clock on; in the I²S format one clock later, so that the last bit of a
    /// word that fills its half-period goes out on the first clock of the next one;
    /// right-justified ending with the last clock. A word longer than the half-period loses
    /// its last bits (right-justified, its first ones); the other clocks carry zeros.
    pub fn half(&mut self, channel: Channel, value: u32, bits: u8, slots: u32) {
        let bits = u32::from(bits.min(MAX_WORD_BITS));
        for slot in 0..slots {
            // The bit of the word on this clock, counted from the least significant.
            let index = match self.config.format {
                Format::I2s | Format::LeftJustified => bits.checked_sub(slot + 1),
                Format::RightJustified => Some(slots - 1 - slot).filter(|index| *index < bits),
            };
            let bit = index.is_some_and(|index| value >> index & 1 != 0);
            if self.config.format == Format::I2s {
                let late = std::mem::replace(&mut self.carry, bit);
                self.clock(channel, late);
            } else {
                self.clock(channel, bit);
            }
        }
    }

    /// A WS period: the left half-period, then the right one, each of `slots` clocks
    /// carrying a word of `bits` bits.
    pub fn frame(&mut self, left: u32, right: u32, bits: u8, slots: u32) {
        self.half(Channel::Left, left, bits, slots);
        self.half(Channel::Right, right, bits, slots);
    }

    /// One clock of the next half-period, WS changed: it ends the last word. In the I²S
    /// format it carries that word's last bit when the word fills its half-period; a
    /// right-justified word, and one whose width WS decides, are decided by the change.
    pub fn close(&mut self) {
        let next = match self.channel {
            Channel::Left => Channel::Right,
            Channel::Right => Channel::Left,
        };
        let bit = std::mem::take(&mut self.carry);
        self.clock(next, bit);
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
