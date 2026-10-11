// SPDX-License-Identifier: Apache-2.0

//! Making NEC waveforms, the demodulated output of an infrared receiver: for tests, for tools
//! and for benchmarks.
//!
//! A waveform is built from elements, stretches of mark or space given in nanoseconds. Each
//! element begins on the first sample at or after its exact time from the waveform's first
//! sample, as a receiver's output looks when sampled at the chosen rate, so rounding never
//! accumulates. The frame and repeat code builders give their elements to a caller that wants to
//! stretch, shrink or drop some of them before sending them.

use uiharu_core::{Levels, LogicBlock, LogicBlockBuilder, Tick};

use super::decoder::{NecConfig, Polarity};

/// The unit of NEC durations, 562.5 µs: a data bit's mark, a 0's space and the final mark.
pub const UNIT_NS: u64 = 562_500;
/// The leader mark, 9 ms.
pub const LEADER_MARK_NS: u64 = 9_000_000;
/// The leader space of a frame, 4.5 ms.
pub const FRAME_SPACE_NS: u64 = 4_500_000;
/// The leader space of a repeat code, 2.25 ms.
pub const REPEAT_SPACE_NS: u64 = 2_250_000;
/// A 1's space, 1687.5 µs.
pub const ONE_SPACE_NS: u64 = 1_687_500;
/// How often a remote control sends a frame or a repeat code while a key is held, 108 ms,
/// counted from the start of one to the start of the next.
pub const REPEAT_PERIOD_NS: u64 = 108_000_000;

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

/// The elements of a frame carrying `bytes` in order, each least significant bit first: the
/// leader mark and space, a mark and a space for each of the 32 data bits, and the final mark.
/// Element `2 + 2 n` is the mark of data bit `n`, element `3 + 2 n` its space.
pub fn frame_elements(bytes: [u8; 4]) -> Vec<Element> {
    let mut elements = Vec::with_capacity(67);
    elements.push(Element::mark(LEADER_MARK_NS));
    elements.push(Element::space(FRAME_SPACE_NS));
    for byte in bytes {
        for bit in 0..8 {
            let space = if byte >> bit & 1 == 1 {
                ONE_SPACE_NS
            } else {
                UNIT_NS
            };
            elements.push(Element::mark(UNIT_NS));
            elements.push(Element::space(space));
        }
    }
    elements.push(Element::mark(UNIT_NS));
    elements
}

/// The elements of a repeat code: the leader mark, the short leader space and the final mark.
pub fn repeat_elements() -> [Element; 3] {
    [
        Element::mark(LEADER_MARK_NS),
        Element::space(REPEAT_SPACE_NS),
        Element::mark(UNIT_NS),
    ]
}

/// Builds the output of an infrared receiver one element at a time; the other channels stay
/// low.
#[derive(Debug)]
pub struct NecWaveform {
    channel: u8,
    polarity: Polarity,
    rate: u64,
    builder: LogicBlockBuilder,
    /// Nanoseconds from the first sample to the end of the last element.
    elapsed: u64,
}

impl NecWaveform {
    /// A waveform on `channels` channels for the receiver of `config`, sampled at `rate_hz`
    /// samples per second from `start` on.
    pub fn new(config: &NecConfig, channels: u8, rate_hz: u64, start: Tick) -> Self {
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

    /// Sends a frame of `address`, its inverse, `command` and its inverse.
    pub fn frame(&mut self, address: u8, command: u8) {
        self.bytes([address, !address, command, !command]);
    }

    /// Sends an extended frame: the 16-bit `address`, low byte first, then `command` and its
    /// inverse.
    pub fn extended(&mut self, address: u16, command: u8) {
        let [low, high] = address.to_le_bytes();
        self.bytes([low, high, command, !command]);
    }

    /// Sends a frame of four bytes as they are given.
    pub fn bytes(&mut self, bytes: [u8; 4]) {
        self.send(&frame_elements(bytes));
    }

    /// Sends a repeat code.
    pub fn repeat(&mut self) {
        self.send(&repeat_elements());
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

    /// The runs of a frame of `bytes` at nine samples a unit.
    fn expected(bytes: [u8; 4]) -> Vec<(bool, u64)> {
        let mut runs = vec![(true, 144), (false, 72)];
        for byte in bytes {
            for bit in 0..8 {
                runs.push((true, 9));
                runs.push((false, if byte >> bit & 1 == 1 { 27 } else { 9 }));
            }
        }
        runs.push((true, 9));
        runs
    }

    #[test]
    fn frames_follow_the_format() {
        for polarity in [Polarity::ActiveLow, Polarity::ActiveHigh] {
            let config = NecConfig {
                channel: 1,
                polarity,
                ..NecConfig::default()
            };
            // 16 kHz: a unit is nine samples.
            let mut waveform = NecWaveform::new(&config, 2, 16_000, Tick(5));
            waveform.idle(UNIT_NS);
            waveform.frame(0x00, 0x45);
            waveform.idle(UNIT_NS);
            let block = waveform.finish();
            assert_eq!(block.start(), Tick(5));
            let mut wanted = vec![(false, 9)];
            // Address 0x00 and its inverse, command 0x45 (bits 1 0 1 0 0 0 1 0 from the
            // first) and its inverse 0xba.
            wanted.extend(expected([0x00, 0xff, 0x45, 0xba]));
            wanted.push((false, 9));
            assert_eq!(runs(&block, 1, polarity), wanted, "{polarity:?}");
            // The other channel stays low.
            assert!((5..block.end().0).all(|tick| block.level(0, Tick(tick)) == Some(false)));
        }
    }

    #[test]
    fn extended_frames_and_repeat_codes() {
        let config = NecConfig::default();
        let mut waveform = NecWaveform::new(&config, 1, 16_000, Tick::ZERO);
        waveform.idle(UNIT_NS);
        waveform.extended(0x1234, 0x56);
        waveform.idle(UNIT_NS);
        waveform.repeat();
        let block = waveform.finish();
        let mut wanted = vec![(false, 9)];
        wanted.extend(expected([0x34, 0x12, 0x56, 0xa9]));
        wanted.extend([(false, 9), (true, 144), (false, 36), (true, 9)]);
        assert_eq!(runs(&block, 0, config.polarity), wanted);
    }

    #[test]
    fn elements_start_on_the_first_sample_at_or_after_their_time() {
        // 1 MHz: 562.5 samples a unit, so units alternate between 563 and 562 samples.
        let mut waveform = NecWaveform::new(&NecConfig::default(), 1, 1_000_000, Tick::ZERO);
        let unit = Element::mark(UNIT_NS);
        waveform.send(&[unit, unit.scaled(100), unit]);
        assert_eq!(waveform.position(), Tick(1688));
        assert_eq!(waveform.elapsed_ns(), 1_687_500);
        waveform.idle_until(2_000_000);
        assert_eq!(waveform.position(), Tick(2000));
        waveform.idle_until(1_000_000);
        assert_eq!(waveform.position(), Tick(2000));
        assert_eq!(unit.scaled(130).ns, 731_250);
        assert_eq!(Element::space(1000).scaled(50), Element::space(500));
        assert!(
            NecWaveform::new(&NecConfig::default(), 1, 1_000_000, Tick::ZERO)
                .try_finish()
                .is_none()
        );
    }
}
