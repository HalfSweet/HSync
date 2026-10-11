// SPDX-License-Identifier: Apache-2.0

//! Making UART waveforms: for tests, for the command line's generator and for benchmarks.

use uiharu_core::{Levels, LogicBlock, LogicBlockBuilder, Tick};

use super::decoder::{BitOrder, UartConfig};

/// Errors [`UartWaveform::frame`] builds into a character.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Faults {
    /// Send the wrong parity bit; a line without a parity bit is not affected.
    pub parity: bool,
    /// Send the stop bits at space. The line then stays at space until [`UartWaveform::idle`]
    /// returns it to mark.
    pub framing: bool,
}

impl Faults {
    /// A character without errors.
    pub const NONE: Faults = Faults {
        parity: false,
        framing: false,
    };

    /// A character with the wrong parity bit.
    pub const PARITY: Faults = Faults {
        parity: true,
        framing: false,
    };

    /// A character whose stop bits are at space.
    pub const FRAMING: Faults = Faults {
        parity: false,
        framing: true,
    };
}

/// Builds the samples of a UART line one character at a time.
#[derive(Debug)]
pub struct UartWaveform {
    config: UartConfig,
    builder: LogicBlockBuilder,
    /// Samples per bit, as the fraction `per_bit.0 / per_bit.1`.
    per_bit: (u64, u64),
    /// Whether the line is at mark.
    mark: bool,
}

impl UartWaveform {
    /// A waveform on `channels` channels for a line configured as `config`, a bit lasting
    /// `bit_samples` samples, starting at `start` with the line idle.
    pub fn new(config: UartConfig, channels: u8, bit_samples: u64, start: Tick) -> Self {
        Self::with_bit_time(config, channels, (bit_samples.max(1), 1), start)
    }

    /// A waveform sampled at `rate_hz` samples per second for a line at `config.baud`: a bit lasts
    /// `rate_hz / baud` samples, which need not be a whole number. Each bit of a character
    /// begins on the first sample at or after its exact time from the character's first sample,
    /// as a transmitter at that baud rate looks when sampled.
    pub fn at_rate(config: UartConfig, channels: u8, rate_hz: u64, start: Tick) -> Self {
        let baud = u64::from(config.baud).max(1);
        Self::with_bit_time(config, channels, (rate_hz.max(1), baud), start)
    }

    fn with_bit_time(config: UartConfig, channels: u8, per_bit: (u64, u64), start: Tick) -> Self {
        Self {
            config,
            builder: LogicBlockBuilder::new(start, channels),
            per_bit,
            mark: true,
        }
    }

    /// The tick the next sample will have.
    pub fn position(&self) -> Tick {
        self.builder.next_tick()
    }

    /// The samples the first `half_bits` half bits of a character take.
    fn samples(&self, half_bits: u64) -> u64 {
        let (samples, bits) = self.per_bit;
        let total = (u128::from(samples) * u128::from(half_bits)).div_ceil(2 * u128::from(bits));
        u64::try_from(total).unwrap_or(u64::MAX)
    }

    fn hold(&mut self, samples: u64) {
        let high = self.mark != self.config.inverted;
        let levels = Levels(if high { 1 << self.config.line } else { 0 });
        self.builder.push_repeated(levels, samples);
    }

    /// Holds the line at mark, idle, for `samples` samples.
    pub fn idle(&mut self, samples: u64) {
        self.mark = true;
        self.hold(samples);
    }

    /// Sends a character carrying the configured number of `value`'s low bits, with `faults`.
    /// Characters sent one after the other follow each other without idle time.
    pub fn frame(&mut self, value: u16, faults: Faults) {
        let config = self.config;
        let bits = u32::from(config.data_bits);
        let value = value & u16::MAX.checked_shr(16 - bits.min(16)).unwrap_or(0);
        // Each cell is a level held for a number of half bits.
        let mut cells = Vec::with_capacity(13);
        cells.push((false, 2));
        for i in 0..bits {
            let shift = match config.bit_order {
                BitOrder::LsbFirst => i,
                BitOrder::MsbFirst => bits - 1 - i,
            };
            cells.push(((value >> shift) & 1 != 0, 2));
        }
        if let Some(bit) = config.parity.bit(value) {
            cells.push((bit != faults.parity, 2));
        }
        cells.push((!faults.framing, u64::from(config.stop_bits.half_bits())));
        let mut at = 0;
        for (mark, half_bits) in cells {
            let samples = self.samples(at + half_bits) - self.samples(at);
            self.mark = mark;
            self.hold(samples);
            at += half_bits;
        }
    }

    /// Sends a character without errors.
    pub fn byte(&mut self, value: u16) {
        self.frame(value, Faults::NONE);
    }

    /// Sends `bytes` as characters without errors, one right after the other.
    pub fn bytes(&mut self, bytes: impl IntoIterator<Item = u8>) {
        for byte in bytes {
            self.byte(u16::from(byte));
        }
    }

    /// Holds the line at space for `samples` samples and lets it return to mark: a break when
    /// it lasts longer than a character. Follow it with [`UartWaveform::idle`], or the next
    /// character's start bit continues it.
    pub fn break_(&mut self, samples: u64) {
        self.mark = false;
        self.hold(samples);
        self.mark = true;
    }

    /// A pulse to space of `samples` samples, too short to be a start bit; like
    /// [`UartWaveform::break_`], follow it with idle time.
    pub fn glitch(&mut self, samples: u64) {
        self.break_(samples);
    }

    /// The samples so far; at least one must have been produced.
    pub fn finish(self) -> LogicBlock {
        self.builder.finish().unwrap_or_else(|_| {
            LogicBlockBuilder::new(Tick::ZERO, 1)
                .finish()
                .expect("no samples were produced")
        })
    }

    /// The samples so far, if any.
    pub fn try_finish(self) -> Option<LogicBlock> {
        self.builder.finish().ok()
    }
}

#[cfg(test)]
mod tests {
    use uiharu_core::TickRange;

    use super::super::decoder::{Parity, StopBits};
    use super::*;

    fn levels(block: &LogicBlock, line: u8) -> String {
        let range = TickRange::new(block.start(), block.end());
        (range.start.0..range.end.0)
            .map(|tick| {
                if block.level(line, Tick(tick)).unwrap_or(false) {
                    '1'
                } else {
                    '0'
                }
            })
            .collect()
    }

    #[test]
    fn characters_follow_the_configuration() {
        let config = UartConfig {
            line: 1,
            data_bits: 5,
            parity: Parity::Odd,
            stop_bits: StopBits::Two,
            ..UartConfig::default()
        };
        let mut waveform = UartWaveform::new(config, 2, 1, Tick::ZERO);
        waveform.idle(1);
        // 0b10110: start, bits 0 1 1 0 1 (LSB first), odd parity 0, two stop bits.
        waveform.byte(0b10110);
        waveform.idle(1);
        assert_eq!(
            levels(&waveform.finish(), 1),
            "1001101011 1".replace(' ', "")
        );

        let msb_inverted = UartConfig {
            bit_order: BitOrder::MsbFirst,
            inverted: true,
            data_bits: 6,
            ..UartConfig::default()
        };
        let mut waveform = UartWaveform::new(msb_inverted, 1, 2, Tick::ZERO);
        // Inverted, space is high: start, bits 1 1 0 0 0 1 (MSB first), the stop bit.
        waveform.byte(0b110001);
        assert_eq!(levels(&waveform.finish(), 0), "1100001111110000");
    }

    #[test]
    fn faults_spoil_parity_and_stop_bits() {
        let config = UartConfig {
            data_bits: 5,
            parity: Parity::Even,
            ..UartConfig::default()
        };
        let mut waveform = UartWaveform::new(config, 1, 1, Tick::ZERO);
        // Even parity calls for a 1 after one 1 bit: the first character sends 0, the second
        // sends it right but its stop bit at space.
        waveform.frame(0b00001, Faults::PARITY);
        waveform.frame(0b00001, Faults::FRAMING);
        waveform.idle(1);
        assert_eq!(levels(&waveform.finish(), 0), "01000001010000101");
    }

    #[test]
    fn fractional_bit_times_round_each_edge_up() {
        // 2.5 samples per bit: the bits of a character begin at 0, 3, 5, 8, 10 … samples.
        let config = UartConfig {
            baud: 2,
            data_bits: 5,
            ..UartConfig::default()
        };
        let mut waveform = UartWaveform::at_rate(config, 1, 5, Tick::ZERO);
        waveform.byte(0b10101);
        assert_eq!(waveform.position(), Tick(18));
        assert_eq!(levels(&waveform.finish(), 0), "000110001100011111");
    }

    #[test]
    fn breaks_hold_space_and_return_to_mark() {
        let mut waveform = UartWaveform::new(UartConfig::default(), 1, 1, Tick(5));
        waveform.idle(2);
        waveform.break_(4);
        waveform.idle(2);
        let block = waveform.finish();
        assert_eq!(block.start(), Tick(5));
        assert_eq!(levels(&block, 0), "11000011");
    }
}
