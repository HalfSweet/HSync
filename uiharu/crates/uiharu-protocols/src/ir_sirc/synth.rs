// SPDX-License-Identifier: Apache-2.0

//! Making SIRC waveforms, the demodulated output of an infrared receiver: for tests, for tools
//! and for benchmarks.
//!
//! A waveform is built from elements, stretches of mark or space given in nanoseconds. Each
//! element begins on the first sample at or after its exact time from the waveform's first
//! sample, as a receiver's output looks when sampled at the chosen rate, so rounding never
//! accumulates. A frame is given as its elements, for a caller that wants to stretch, shrink or
//! drop some of them before sending them.

use uiharu_core::{Levels, LogicBlock, LogicBlockBuilder, Tick};

use super::decoder::{Polarity, SircConfig};

/// The unit `T`, 600 µs: a 0's mark and every space.
pub const UNIT_NS: u64 = 600_000;
/// The start mark, 4 `T`.
pub const START_MARK_NS: u64 = 4 * UNIT_NS;
/// A 1's mark, 2 `T`.
pub const ONE_MARK_NS: u64 = 2 * UNIT_NS;
/// How often a remote control sends a frame while a key is held, 45 ms, counted from the start
/// of one frame to the start of the next.
pub const REPEAT_PERIOD_NS: u64 = 45_000_000;

/// A stretch of the receiver's output at one level.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Element {
    /// Whether the carrier is received: the line at mark.
    pub mark: bool,
    /// How long it lasts, in nanoseconds.
    pub ns: u64,
}

impl Element {
    /// A mark of `ns` nanoseconds.
    pub const fn mark(ns: u64) -> Self {
        Self { mark: true, ns }
    }

    /// A space of `ns` nanoseconds.
    pub const fn space(ns: u64) -> Self {
        Self { mark: false, ns }
    }

    /// The element lasting `percent` percent of its length: stretched above 100, shrunk below.
    pub const fn scaled(self, percent: u64) -> Self {
        Self {
            mark: self.mark,
            ns: self.ns * percent / 100,
        }
    }
}

/// The bits of a frame in the order they are sent, each field least significant bit first: the
/// 7-bit command, then for `length` 12 a 5-bit address, for 15 an 8-bit address, for 20 a 5-bit
/// address and the 8-bit `extended` field. Another length gives that many bits of the same
/// fields, cut short or padded with zeros. Bits of a field beyond its width are not sent.
pub fn frame_bits(length: u8, command: u8, address: u8, extended: u8) -> Vec<bool> {
    let mut value = u32::from(command & 0x7f);
    value |= if length == 15 {
        u32::from(address) << 7
    } else {
        u32::from(address & 0x1f) << 7 | u32::from(extended) << 12
    };
    (0..length).map(|bit| value >> bit & 1 == 1).collect()
}

/// The elements of a frame of `bits`: the start mark and its space, then for each bit a mark of
/// `T` (0) or 2 `T` (1) and a space of `T`. Element `2 + 2 n` is the mark of bit `n`, element
/// `3 + 2 n` its space; the last bit's space runs into the line's silence.
pub fn elements(bits: &[bool]) -> Vec<Element> {
    let mut elements = Vec::with_capacity(2 + 2 * bits.len());
    elements.push(Element::mark(START_MARK_NS));
    elements.push(Element::space(UNIT_NS));
    for &one in bits {
        elements.push(Element::mark(if one { ONE_MARK_NS } else { UNIT_NS }));
        elements.push(Element::space(UNIT_NS));
    }
    elements
}

/// The elements of a frame of `length` bits: 12, 15 or 20.
pub fn frame_elements(length: u8, command: u8, address: u8, extended: u8) -> Vec<Element> {
    elements(&frame_bits(length, command, address, extended))
}

/// Builds the output of an infrared receiver one element at a time; the other channels stay
/// low.
#[derive(Debug)]
pub struct SircWaveform {
    channel: u8,
    polarity: Polarity,
    rate: u64,
    builder: LogicBlockBuilder,
    /// Nanoseconds from the first sample to the end of the last element.
    elapsed: u64,
}

impl SircWaveform {
    /// A waveform on `channels` channels for the receiver of `config`, sampled at `rate_hz`
    /// samples per second from `start` on.
    pub fn new(config: &SircConfig, channels: u8, rate_hz: u64, start: Tick) -> Self {
        Self {
            channel: config.channel,
            polarity: config.polarity,
            rate: rate_hz.max(1),
            builder: LogicBlockBuilder::new(start, channels),
            elapsed: 0,
        }
    }

    /// The tick the next sample will have.
    pub fn position(&self) -> Tick {
        self.builder.next_tick()
    }

    /// Nanoseconds from the first sample to the end of what was sent.
    pub fn elapsed_ns(&self) -> u64 {
        self.elapsed
    }

    /// The samples from the first one to the first at or after `ns` nanoseconds.
    fn samples(&self, ns: u64) -> u64 {
        let samples = (u128::from(ns) * u128::from(self.rate)).div_ceil(1_000_000_000);
        u64::try_from(samples).unwrap_or(u64::MAX)
    }

    /// Sends `elements` one after the other.
    pub fn send(&mut self, elements: &[Element]) {
        for element in elements {
            let from = self.samples(self.elapsed);
            self.elapsed = self.elapsed.saturating_add(element.ns);
            let to = self.samples(self.elapsed);
            let high = element.mark == matches!(self.polarity, Polarity::ActiveHigh);
            self.builder
                .push_repeated(Levels(u64::from(high) << self.channel), to - from);
        }
    }

    /// Holds the line idle, at space, for `ns` nanoseconds.
    pub fn idle(&mut self, ns: u64) {
        self.send(&[Element::space(ns)]);
    }

    /// Holds the line idle until `ns` nanoseconds after the first sample, if that is later.
    pub fn idle_until(&mut self, ns: u64) {
        let rest = ns.saturating_sub(self.elapsed);
        if rest > 0 {
            self.idle(rest);
        }
    }

    /// Sends a frame of `length` bits, 12, 15 or 20, as [`frame_bits`] lays them out.
    pub fn frame(&mut self, length: u8, command: u8, address: u8, extended: u8) {
        self.send(&frame_elements(length, command, address, extended));
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

    /// The levels of `channel` as runs: whether at mark, for how many samples.
    fn runs(block: &LogicBlock, channel: u8, polarity: Polarity) -> Vec<(bool, u64)> {
        let mut runs: Vec<(bool, u64)> = Vec::new();
        let mut tick = block.start();
        while tick < block.end() {
            let high = block.level(channel, tick).expect("in the block");
            let mark = polarity.is_mark(high);
            match runs.last_mut() {
                Some((level, count)) if *level == mark => *count += 1,
                _ => runs.push((mark, 1)),
            }
            tick = tick.next();
        }
        runs
    }

    fn text(bits: &[bool]) -> String {
        bits.iter()
            .map(|&one| if one { '1' } else { '0' })
            .collect()
    }

    #[test]
    fn bits_follow_the_three_lengths() {
        // Command 21 (1010100 from its least significant bit), address 1 (10000).
        assert_eq!(text(&frame_bits(12, 21, 1, 0)), "101010010000");
        // An 8-bit address 0x97 (11101001).
        assert_eq!(text(&frame_bits(15, 0, 0x97, 0)), "000000011101001");
        // A 5-bit address 0x1a (01011) and the extended field 0xe2 (01000111).
        assert_eq!(
            text(&frame_bits(20, 0x7f, 0x1a, 0xe2)),
            "11111110101101000111"
        );
        assert_eq!(text(&frame_bits(13, 0, 0, 0xff)), "0000000000001");
    }

    #[test]
    fn frames_are_pulse_width_coded() {
        for polarity in [Polarity::ActiveLow, Polarity::ActiveHigh] {
            let config = SircConfig {
                channel: 1,
                polarity,
                ..SircConfig::default()
            };
            // 40 kHz: the unit is 24 samples.
            let mut waveform = SircWaveform::new(&config, 2, 40_000, Tick::ZERO);
            waveform.idle(UNIT_NS);
            waveform.frame(12, 21, 1, 0);
            waveform.idle(4 * UNIT_NS);
            let block = waveform.finish();
            let mut wanted = vec![(false, 24), (true, 96), (false, 24)];
            for one in [1, 0, 1, 0, 1, 0, 0, 1, 0, 0, 0, 0] {
                wanted.push((true, 24 + 24 * one));
                wanted.push((false, 24));
            }
            // The last bit's space runs into the silence after the frame.
            if let Some((_, samples)) = wanted.last_mut() {
                *samples += 4 * 24;
            }
            assert_eq!(runs(&block, 1, polarity), wanted, "{polarity:?}");
        }
    }

    #[test]
    fn elements_start_on_the_first_sample_at_or_after_their_time() {
        // 33 kHz: 19.8 samples to the unit.
        let mut waveform = SircWaveform::new(&SircConfig::default(), 1, 33_000, Tick::ZERO);
        let unit = Element::mark(UNIT_NS);
        waveform.send(&[unit, unit.scaled(100), unit]);
        assert_eq!(waveform.position(), Tick(60));
        assert_eq!(waveform.elapsed_ns(), 1_800_000);
        waveform.idle_until(2_000_000);
        assert_eq!(waveform.position(), Tick(66));
        assert_eq!(frame_elements(20, 0, 0, 0).len(), 42);
        assert!(
            SircWaveform::new(&SircConfig::default(), 1, 1_000_000, Tick::ZERO)
                .try_finish()
                .is_none()
        );
    }
}
