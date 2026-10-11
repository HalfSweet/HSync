// SPDX-License-Identifier: Apache-2.0

//! Making RC-5 waveforms, the demodulated output of an infrared receiver: for tests, for tools
//! and for benchmarks.
//!
//! A waveform is built from elements, stretches of mark or space given in nanoseconds. Each
//! element begins on the first sample at or after its exact time from the waveform's first
//! sample, as a receiver's output looks when sampled at the chosen rate, so rounding never
//! accumulates. A frame is given as its 28 half bits, one element each, for a caller that wants
//! to stretch, shrink or drop some of them before sending them; two half bits at the same level
//! simply make a longer mark or space on the line.

use uiharu_core::{Levels, LogicBlock, LogicBlockBuilder, Tick};

use super::decoder::{Polarity, Rc5Config};

/// A half bit: 32 periods of the 36 kHz carrier, 888.9 µs, rounded down to the nanosecond.
pub const HALF_BIT_NS: u64 = 888_888;
/// How often a remote control sends a frame while a key is held, 64 bit times (113.8 ms),
/// counted from the start of one frame to the start of the next.
pub const REPEAT_PERIOD_NS: u64 = 128 * HALF_BIT_NS;

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

/// The 14 bits of a frame in the order they are sent: the start bits, the toggle bit, five
/// address bits and six command bits, each most significant bit first. A command of 64 or more
/// is sent as RC-5X does, its bit 6 inverted in the second start bit; only the low five bits of
/// `address` and seven of `command` are sent.
pub fn frame_bits(toggle: bool, address: u8, command: u8) -> [bool; 14] {
    let mut bits = [false; 14];
    bits[0] = true;
    bits[1] = command & 0x40 == 0;
    bits[2] = toggle;
    for i in 0..5 {
        bits[3 + i] = address >> (4 - i) & 1 == 1;
    }
    for i in 0..6 {
        bits[8 + i] = command >> (5 - i) & 1 == 1;
    }
    bits
}

/// The half bits of `bits`, two elements a bit: a 1 is a space then a mark, a 0 a mark then a
/// space. Element `2 n` is the first half of bit `n`, element `2 n + 1` its second half.
pub fn half_bits(bits: &[bool]) -> Vec<Element> {
    let mut elements = Vec::with_capacity(2 * bits.len());
    for &one in bits {
        elements.push(Element {
            mark: !one,
            ns: HALF_BIT_NS,
        });
        elements.push(Element {
            mark: one,
            ns: HALF_BIT_NS,
        });
    }
    elements
}

/// The 28 half bits of a frame.
pub fn frame_elements(toggle: bool, address: u8, command: u8) -> Vec<Element> {
    half_bits(&frame_bits(toggle, address, command))
}

/// Builds the output of an infrared receiver one element at a time; the other channels stay
/// low.
#[derive(Debug)]
pub struct Rc5Waveform {
    channel: u8,
    polarity: Polarity,
    rate: u64,
    builder: LogicBlockBuilder,
    /// Nanoseconds from the first sample to the end of the last element.
    elapsed: u64,
}

impl Rc5Waveform {
    /// A waveform on `channels` channels for the receiver of `config`, sampled at `rate_hz`
    /// samples per second from `start` on.
    pub fn new(config: &Rc5Config, channels: u8, rate_hz: u64, start: Tick) -> Self {
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

    /// Sends a frame: the start bits, `toggle`, `address` and `command`, a command of 64 or
    /// more as RC-5X sends it.
    pub fn frame(&mut self, toggle: bool, address: u8, command: u8) {
        self.send(&frame_elements(toggle, address, command));
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

    /// The runs of half bits written as `m` (mark) and `s` (space), `samples` each.
    fn runs_of(halves: &str, samples: u64) -> Vec<(bool, u64)> {
        let mut runs: Vec<(bool, u64)> = Vec::new();
        for half in halves.chars() {
            let mark = half == 'm';
            match runs.last_mut() {
                Some((level, count)) if *level == mark => *count += samples,
                _ => runs.push((mark, samples)),
            }
        }
        runs
    }

    #[test]
    fn bits_follow_the_frame_format() {
        // Address 5, command 12: start bits 1 1, toggle, 00101, 001100.
        let bits = frame_bits(true, 5, 12);
        let text: String = bits
            .iter()
            .map(|&one| if one { '1' } else { '0' })
            .collect();
        assert_eq!(text, "11100101001100");
        // Command 64 and up clears the second start bit.
        let text: String = frame_bits(false, 31, 64 + 0b101010)
            .iter()
            .map(|&one| if one { '1' } else { '0' })
            .collect();
        assert_eq!(text, "10011111101010");
    }

    #[test]
    fn frames_are_bi_phase_coded() {
        for polarity in [Polarity::ActiveLow, Polarity::ActiveHigh] {
            let config = Rc5Config {
                channel: 1,
                polarity,
                ..Rc5Config::default()
            };
            // 36 kHz: a half bit is 32 samples.
            let mut waveform = Rc5Waveform::new(&config, 2, 36_000, Tick::ZERO);
            waveform.idle(4 * HALF_BIT_NS);
            waveform.frame(false, 0, 12);
            waveform.idle(4 * HALF_BIT_NS);
            let block = waveform.finish();
            // Bits 1 1 0 00000 001100, two half bits each, a 1 being a space then a mark;
            // four half bits of idle line before and after.
            let halves =
                "ssss".to_owned() + "smsm" + "ms" + &"ms".repeat(5) + "msmssmsmmsms" + "ssss";
            assert_eq!(
                runs(&block, 1, polarity),
                runs_of(&halves, 32),
                "{polarity:?}"
            );
        }
    }

    #[test]
    fn elements_start_on_the_first_sample_at_or_after_their_time() {
        let mut waveform = Rc5Waveform::new(&Rc5Config::default(), 1, 1_000_000, Tick::ZERO);
        let half = Element::mark(HALF_BIT_NS);
        waveform.send(&[half, half.scaled(100), half]);
        // 2666.664 µs.
        assert_eq!(waveform.position(), Tick(2667));
        assert_eq!(waveform.elapsed_ns(), 2_666_664);
        waveform.idle_until(3_000_000);
        assert_eq!(waveform.position(), Tick(3000));
        assert_eq!(REPEAT_PERIOD_NS, 113_777_664);
        assert!(
            Rc5Waveform::new(&Rc5Config::default(), 1, 1_000_000, Tick::ZERO)
                .try_finish()
                .is_none()
        );
    }
}
