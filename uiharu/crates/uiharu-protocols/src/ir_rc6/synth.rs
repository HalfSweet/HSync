// SPDX-License-Identifier: Apache-2.0

//! Making RC-6 waveforms, the demodulated output of an infrared receiver: for tests, for tools
//! and for benchmarks.
//!
//! A waveform is built from elements, stretches of mark or space given in nanoseconds. Each
//! element begins on the first sample at or after its exact time from the waveform's first
//! sample, as a receiver's output looks when sampled at the chosen rate, so rounding never
//! accumulates. A frame is given as its leader mark and space and its half bits, one element
//! each, for a caller that wants to stretch, shrink or drop some of them before sending them;
//! two halves at the same level simply make a longer mark or space on the line.

use uiharu_core::{Levels, LogicBlock, LogicBlockBuilder, Tick};

use super::decoder::{Polarity, Rc6Config};

/// The unit `t`: 16 periods of the 36 kHz carrier, 444.4 µs, rounded down to the nanosecond.
pub const UNIT_NS: u64 = 444_444;
/// The leader mark, 6 `t`.
pub const LEADER_MARK_NS: u64 = 6 * UNIT_NS;
/// The leader space, 2 `t`.
pub const LEADER_SPACE_NS: u64 = 2 * UNIT_NS;
/// The least signal-free time after a frame, 6 `t`.
pub const SIGNAL_FREE_NS: u64 = 6 * UNIT_NS;

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

/// The 21 bits of a frame in the order they are sent: the start bit, the three mode bits, the
/// trailer bit, eight control and eight information bits, each field most significant bit
/// first. Only the low three bits of `mode` are sent.
pub fn frame_bits(mode: u8, toggle: bool, address: u8, command: u8) -> [bool; 21] {
    let mut bits = [false; 21];
    bits[0] = true;
    for i in 0..3 {
        bits[1 + i] = mode >> (2 - i) & 1 == 1;
    }
    bits[4] = toggle;
    for i in 0..8 {
        bits[5 + i] = address >> (7 - i) & 1 == 1;
        bits[13 + i] = command >> (7 - i) & 1 == 1;
    }
    bits
}

/// The elements of a frame of `bits`: the leader mark and space, then two halves a bit, a 1
/// being a mark then a space and a 0 a space then a mark, the halves of bit 4, the trailer bit,
/// twice as long. Element `2 + 2 n` is the first half of bit `n`, element `3 + 2 n` its second
/// half.
pub fn elements(bits: &[bool]) -> Vec<Element> {
    let mut elements = Vec::with_capacity(2 + 2 * bits.len());
    elements.push(Element::mark(LEADER_MARK_NS));
    elements.push(Element::space(LEADER_SPACE_NS));
    for (n, &one) in bits.iter().enumerate() {
        let ns = if n == 4 { 2 * UNIT_NS } else { UNIT_NS };
        elements.push(Element { mark: one, ns });
        elements.push(Element { mark: !one, ns });
    }
    elements
}

/// The 44 elements of a frame.
pub fn frame_elements(mode: u8, toggle: bool, address: u8, command: u8) -> Vec<Element> {
    elements(&frame_bits(mode, toggle, address, command))
}

/// Builds the output of an infrared receiver one element at a time; the other channels stay
/// low.
#[derive(Debug)]
pub struct Rc6Waveform {
    channel: u8,
    polarity: Polarity,
    rate: u64,
    builder: LogicBlockBuilder,
    /// Nanoseconds from the first sample to the end of the last element.
    elapsed: u64,
}

impl Rc6Waveform {
    /// A waveform on `channels` channels for the receiver of `config`, sampled at `rate_hz`
    /// samples per second from `start` on.
    pub fn new(config: &Rc6Config, channels: u8, rate_hz: u64, start: Tick) -> Self {
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

    /// Sends a frame of mode 0: `toggle` in the trailer bit, `address` and `command`.
    pub fn frame(&mut self, toggle: bool, address: u8, command: u8) {
        self.send(&frame_elements(0, toggle, address, command));
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

    /// The runs of units written as `m` (mark) and `s` (space), `samples` each.
    fn runs_of(units: &str, samples: u64) -> Vec<(bool, u64)> {
        let mut runs: Vec<(bool, u64)> = Vec::new();
        for unit in units.chars() {
            let mark = unit == 'm';
            match runs.last_mut() {
                Some((level, count)) if *level == mark => *count += samples,
                _ => runs.push((mark, samples)),
            }
        }
        runs
    }

    #[test]
    fn bits_follow_the_frame_format() {
        let text = |bits: [bool; 21]| -> String {
            bits.iter()
                .map(|&one| if one { '1' } else { '0' })
                .collect()
        };
        // Start, mode 000, trailer 1, address 0x04, command 0x5c.
        assert_eq!(
            text(frame_bits(0, true, 0x04, 0x5c)),
            "100010000010001011100"
        );
        assert_eq!(text(frame_bits(6, false, 0, 0xff)), "111000000000011111111");
    }

    #[test]
    fn frames_are_bi_phase_coded_with_a_double_trailer_bit() {
        for polarity in [Polarity::ActiveLow, Polarity::ActiveHigh] {
            let config = Rc6Config {
                channel: 1,
                polarity,
                ..Rc6Config::default()
            };
            // 72 kHz: a unit is 32 samples.
            let mut waveform = Rc6Waveform::new(&config, 2, 72_000, Tick::ZERO);
            waveform.idle(2 * UNIT_NS);
            waveform.frame(true, 0x04, 0x5c);
            waveform.idle(2 * UNIT_NS);
            let block = waveform.finish();
            // The leader, then bits 1 000 1 00000100 01011100, a 1 a mark then a space; the
            // trailer bit's halves two units each.
            let units = "ss".to_owned()
                + "mmmmmmss"
                + "ms"
                + "smsmsm"
                + "mmss"
                + "smsmsmsmsmmssmsm"
                + "smmssmmsmsmssmsm"
                + "ss";
            assert_eq!(
                runs(&block, 1, polarity),
                runs_of(&units, 32),
                "{polarity:?}"
            );
        }
    }

    #[test]
    fn elements_start_on_the_first_sample_at_or_after_their_time() {
        let mut waveform = Rc6Waveform::new(&Rc6Config::default(), 1, 1_000_000, Tick::ZERO);
        let unit = Element::mark(UNIT_NS);
        waveform.send(&[unit, unit.scaled(100), unit]);
        // 1333.332 µs.
        assert_eq!(waveform.position(), Tick(1334));
        assert_eq!(waveform.elapsed_ns(), 1_333_332);
        waveform.idle_until(2_000_000);
        assert_eq!(waveform.position(), Tick(2000));
        assert_eq!(frame_elements(0, false, 0, 0).len(), 44);
        assert_eq!(SIGNAL_FREE_NS, LEADER_MARK_NS);
        assert!(
            Rc6Waveform::new(&Rc6Config::default(), 1, 1_000_000, Tick::ZERO)
                .try_finish()
                .is_none()
        );
    }
}
