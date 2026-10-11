// SPDX-License-Identifier: Apache-2.0

//! Making I²C waveforms: for tests, for tools and for benchmarks.
//!
//! An SCL period is four quarters: SDA takes its next level one quarter into the low phase,
//! SCL rises after the second quarter and stays high for two. A START or STOP condition
//! changes SDA a quarter after SCL rose and holds the new level for a quarter.

use uiharu_core::{Levels, LogicBlock, LogicBlockBuilder, Tick};

use super::decoder::I2cConfig;

/// What the bus carries between a START and a STOP.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Symbol {
    /// A byte, most significant bit first, and the acknowledge bit after it.
    Byte {
        /// The byte.
        value: u8,
        /// Whether the receiver acknowledges it (SDA low on the ninth clock).
        ack: bool,
    },
    /// A repeated START.
    Restart,
}

/// Builds the samples of an I²C bus one condition and one byte at a time.
#[derive(Debug)]
pub struct I2cWaveform {
    config: I2cConfig,
    builder: LogicBlockBuilder,
    /// Samples per quarter of the SCL period.
    quarter: u64,
    levels: Levels,
}

impl I2cWaveform {
    /// A waveform on `channels` channels for a bus wired as `config`, with `quarter` samples per
    /// quarter of the SCL period, starting at `start` with the bus free: both lines high.
    pub fn new(config: I2cConfig, channels: u8, quarter: u64, start: Tick) -> Self {
        Self {
            config,
            builder: LogicBlockBuilder::new(start, channels),
            quarter: quarter.max(1),
            levels: Levels((1 << config.scl) | (1 << config.sda)),
        }
    }

    /// The tick the next sample will have.
    pub fn position(&self) -> Tick {
        self.builder.next_tick()
    }

    fn scl_high(&self) -> bool {
        self.levels.high(self.config.scl)
    }

    fn sda_high(&self) -> bool {
        self.levels.high(self.config.sda)
    }

    fn set(&mut self, channel: u8, high: bool) {
        if high {
            self.levels.0 |= 1 << channel;
        } else {
            self.levels.0 &= !(1 << channel);
        }
    }

    fn set_scl(&mut self, high: bool) {
        self.set(self.config.scl, high);
    }

    fn set_sda(&mut self, high: bool) {
        self.set(self.config.sda, high);
    }

    fn hold(&mut self, samples: u64) {
        self.builder.push_repeated(self.levels, samples);
    }

    /// Holds both lines as they are for `samples` samples. With SCL low, between bytes or
    /// conditions, this is a target stretching the clock.
    pub fn idle(&mut self, samples: u64) {
        self.hold(samples);
    }

    /// A START condition, or a repeated START when SCL is low: SDA falls while SCL is high,
    /// then SCL falls.
    pub fn start(&mut self) {
        let q = self.quarter;
        if self.scl_high() && !self.sda_high() {
            // Raising SDA now would be a STOP.
            self.set_scl(false);
            self.hold(q);
        }
        if !self.scl_high() {
            self.set_sda(true);
            self.hold(q);
            self.set_scl(true);
            self.hold(q);
        }
        self.set_sda(false);
        self.hold(q);
        self.set_scl(false);
        self.hold(q);
    }

    /// A repeated START: the same as [`I2cWaveform::start`] from where a byte left the bus.
    pub fn restart(&mut self) {
        self.start();
    }

    /// A STOP condition: SDA low while SCL is low, SCL rises, then SDA rises while SCL is
    /// high. From a free bus this clocks SCL once first.
    pub fn stop(&mut self) {
        let q = self.quarter;
        if self.scl_high() {
            self.set_scl(false);
            self.hold(q);
        }
        self.set_sda(false);
        self.hold(q);
        self.set_scl(true);
        self.hold(q);
        self.set_sda(true);
        self.hold(q);
    }

    /// One clock: SDA set while SCL is low, SCL high for half a period, then low again.
    fn bit(&mut self, high: bool) {
        let q = self.quarter;
        if self.scl_high() {
            self.set_scl(false);
            self.hold(q);
        }
        self.set_sda(high);
        self.hold(q);
        self.set_scl(true);
        self.hold(2 * q);
        self.set_scl(false);
        self.hold(q);
    }

    /// The first `count` bits of `value`, most significant first, with no acknowledge bit: a
    /// byte cut short.
    pub fn bits(&mut self, value: u8, count: u8) {
        for i in 0..count.min(8) {
            self.bit(value >> (7 - i) & 1 != 0);
        }
    }

    /// A byte and its acknowledge bit (`ack`: SDA low on the ninth clock).
    pub fn byte(&mut self, value: u8, ack: bool) {
        self.byte_stretched(value, ack, 0);
    }

    /// A byte whose receiver holds SCL low for `stretch` more samples before the acknowledge
    /// clock, as a target does that stretches the clock.
    pub fn byte_stretched(&mut self, value: u8, ack: bool, stretch: u64) {
        self.bits(value, 8);
        self.hold(stretch);
        self.bit(!ack);
    }

    /// The bytes and repeated STARTs of `symbols`.
    pub fn symbols(&mut self, symbols: impl IntoIterator<Item = Symbol>) {
        for symbol in symbols {
            match symbol {
                Symbol::Byte { value, ack } => self.byte(value, ack),
                Symbol::Restart => self.restart(),
            }
        }
    }

    /// A whole transaction: START, `symbols`, STOP.
    pub fn transaction(&mut self, symbols: impl IntoIterator<Item = Symbol>) {
        self.start();
        self.symbols(symbols);
        self.stop();
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

/// The first byte of a 10-bit address: `1111 0`, A9, A8 and R/W̄.
fn ten_bit_prefix(address: u16, read: bool) -> u8 {
    0b1111_0000 | ((((address >> 8) & 0b11) as u8) << 1) | u8::from(read)
}

/// What a target sends: `data`, the controller acknowledging every byte but the last.
fn received(data: &[u8]) -> impl Iterator<Item = Symbol> + '_ {
    data.iter().enumerate().map(|(i, value)| Symbol::Byte {
        value: *value,
        ack: i + 1 < data.len(),
    })
}

/// What a controller sends: `data`, every byte acknowledged.
fn sent(data: &[u8]) -> impl Iterator<Item = Symbol> + '_ {
    data.iter().map(|value| Symbol::Byte {
        value: *value,
        ack: true,
    })
}

/// A write of `data` to the 7-bit `address`, every byte acknowledged.
pub fn write_7bit(address: u8, data: &[u8]) -> Vec<Symbol> {
    std::iter::once(Symbol::Byte {
        value: address << 1,
        ack: true,
    })
    .chain(sent(data))
    .collect()
}

/// A read of `data` from the 7-bit `address`: the address acknowledged, then the data with
/// every byte but the last acknowledged by the controller.
pub fn read_7bit(address: u8, data: &[u8]) -> Vec<Symbol> {
    std::iter::once(Symbol::Byte {
        value: (address << 1) | 1,
        ack: true,
    })
    .chain(received(data))
    .collect()
}

/// A write of `data` to the 10-bit `address`: `1111 0 A9 A8 0`, A7–A0, then the data, every
/// byte acknowledged.
pub fn write_10bit(address: u16, data: &[u8]) -> Vec<Symbol> {
    [
        Symbol::Byte {
            value: ten_bit_prefix(address, false),
            ack: true,
        },
        Symbol::Byte {
            value: (address & 0xff) as u8,
            ack: true,
        },
    ]
    .into_iter()
    .chain(sent(data))
    .collect()
}

/// A read of `data` from the 10-bit `address`: the write address in two bytes, a repeated
/// START, `1111 0 A9 A8 1`, then the data with every byte but the last acknowledged.
pub fn read_10bit(address: u16, data: &[u8]) -> Vec<Symbol> {
    let mut symbols = write_10bit(address, &[]);
    symbols.push(Symbol::Restart);
    symbols.push(Symbol::Byte {
        value: ten_bit_prefix(address, true),
        ack: true,
    });
    symbols.extend(received(data));
    symbols
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn byte_lists_follow_the_addressing() {
        assert_eq!(
            write_7bit(0x50, &[0x12]),
            vec![
                Symbol::Byte {
                    value: 0xa0,
                    ack: true
                },
                Symbol::Byte {
                    value: 0x12,
                    ack: true
                },
            ]
        );
        assert_eq!(
            read_7bit(0x50, &[1, 2]),
            vec![
                Symbol::Byte {
                    value: 0xa1,
                    ack: true
                },
                Symbol::Byte {
                    value: 1,
                    ack: true
                },
                Symbol::Byte {
                    value: 2,
                    ack: false
                },
            ]
        );
        assert_eq!(
            read_10bit(0x2a5, &[9]),
            vec![
                Symbol::Byte {
                    value: 0xf4,
                    ack: true
                },
                Symbol::Byte {
                    value: 0xa5,
                    ack: true
                },
                Symbol::Restart,
                Symbol::Byte {
                    value: 0xf5,
                    ack: true
                },
                Symbol::Byte {
                    value: 9,
                    ack: false
                },
            ]
        );
    }

    #[test]
    fn a_byte_takes_nine_clocks_of_four_quarters() {
        let mut waveform = I2cWaveform::new(I2cConfig::default(), 2, 3, Tick(100));
        waveform.start();
        assert_eq!(waveform.position(), Tick(106));
        waveform.byte(0xff, true);
        assert_eq!(waveform.position(), Tick(106 + 9 * 12));
        waveform.stop();
        let block = waveform.finish();
        // SDA fell at 100 with SCL high; SCL rose a quarter into the first bit.
        assert_eq!(block.levels(Tick(100)), Some(Levels(0b01)));
        assert_eq!(block.levels(Tick(108)), Some(Levels(0b10)));
        assert_eq!(block.levels(Tick(109)), Some(Levels(0b11)));
        // The acknowledge bit holds SDA low, and the STOP ends with both lines high.
        assert_eq!(block.levels(Tick(106 + 8 * 12 + 3)), Some(Levels(0b01)));
        assert_eq!(block.levels(Tick(block.end().0 - 1)), Some(Levels(0b11)));
    }
}
