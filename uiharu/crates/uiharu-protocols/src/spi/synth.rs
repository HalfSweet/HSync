// SPDX-License-Identifier: Apache-2.0

//! Making SPI waveforms: for tests, for the command line's generator and for benchmarks.

use uiharu_core::{Levels, LogicBlock, LogicBlockBuilder, Tick};

use super::{BitOrder, SpiConfig, cpha, cpol};

/// Builds the samples of an SPI bus one transaction at a time.
#[derive(Debug)]
pub struct SpiWaveform {
    config: SpiConfig,
    builder: LogicBlockBuilder,
    /// Samples per half clock period.
    half_period: u64,
    levels: Levels,
    selected: bool,
}

impl SpiWaveform {
    /// A waveform on `channels` channels for a bus wired as `config`, clocked at
    /// `half_period` samples per half period, starting at `start` with the bus idle.
    pub fn new(config: SpiConfig, channels: u8, half_period: u64, start: Tick) -> Self {
        let mut levels = Levels(0);
        if cpol(&config) {
            levels.0 |= 1 << config.clk;
        }
        if let Some(cs) = config.cs
            && config.cs_active_low
        {
            levels.0 |= 1 << cs;
        }
        Self {
            config,
            builder: LogicBlockBuilder::new(start, channels),
            half_period: half_period.max(1),
            levels,
            selected: false,
        }
    }

    /// The tick the next sample will have.
    pub fn position(&self) -> Tick {
        self.builder.next_tick()
    }

    /// Whether chip select is asserted.
    pub fn is_selected(&self) -> bool {
        self.selected
    }

    fn set(&mut self, channel: Option<u8>, high: bool) {
        if let Some(channel) = channel {
            if high {
                self.levels.0 |= 1 << channel;
            } else {
                self.levels.0 &= !(1 << channel);
            }
        }
    }

    fn hold(&mut self, samples: u64) {
        self.builder.push_repeated(self.levels, samples);
    }

    /// Holds the bus as it is for `samples` samples.
    pub fn idle(&mut self, samples: u64) {
        self.hold(samples);
    }

    /// Asserts chip select, then waits half a period before the first clock edge.
    pub fn select(&mut self) {
        self.set(self.config.cs, !self.config.cs_active_low);
        self.selected = true;
        self.hold(self.half_period);
    }

    /// Releases chip select after half a period of idle clock.
    pub fn release(&mut self) {
        self.hold(self.half_period);
        self.set(self.config.cs, self.config.cs_active_low);
        self.selected = false;
    }

    /// Clocks `bits` bits of a word on both lines, the most or least significant first as
    /// configured, with the clock edges the mode calls for.
    pub fn partial_word(&mut self, mosi: u8, miso: u8, bits: u8) {
        let idle = cpol(&self.config);
        let (clk, cpha) = (self.config.clk, cpha(&self.config));
        let word_bits = self.config.word_bits;
        for i in 0..bits.min(word_bits) {
            let shift = match self.config.bit_order {
                BitOrder::MsbFirst => word_bits - 1 - i,
                BitOrder::LsbFirst => i,
            };
            let (mosi_bit, miso_bit) = (mosi >> shift & 1 != 0, miso >> shift & 1 != 0);
            if cpha {
                // Data changes on the leading edge and is taken on the trailing one.
                self.set(Some(clk), !idle);
                self.set(self.config.mosi, mosi_bit);
                self.set(self.config.miso, miso_bit);
                self.hold(self.half_period);
                self.set(Some(clk), idle);
                self.hold(self.half_period);
            } else {
                // Data is set up before the leading edge, which takes it.
                self.set(Some(clk), idle);
                self.set(self.config.mosi, mosi_bit);
                self.set(self.config.miso, miso_bit);
                self.hold(self.half_period);
                self.set(Some(clk), !idle);
                self.hold(self.half_period);
            }
        }
        // Leave the clock idle after the last bit.
        self.set(Some(clk), idle);
    }

    /// Clocks one whole word.
    pub fn word(&mut self, mosi: u8, miso: u8) {
        self.partial_word(mosi, miso, self.config.word_bits);
    }

    /// Clocks `words`, MOSI and MISO values pairwise.
    pub fn words(&mut self, words: impl IntoIterator<Item = (u8, u8)>) {
        for (mosi, miso) in words {
            self.word(mosi, miso);
        }
    }

    /// A whole transaction: select, the words, release.
    pub fn transaction(&mut self, words: impl IntoIterator<Item = (u8, u8)>) {
        self.select();
        self.words(words);
        self.release();
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

/// The MOSI and MISO words of an SPI NOR `READ` command: the opcode `0x03`, three address
/// bytes, then as many words as `data` has, with the data on MISO.
pub fn nor_read_words(address: u32, data: &[u8]) -> Vec<(u8, u8)> {
    let [_, a2, a1, a0] = address.to_be_bytes();
    let mut words = vec![(0x03, 0xff), (a2, 0xff), (a1, 0xff), (a0, 0xff)];
    words.extend(data.iter().map(|byte| (0x00, *byte)));
    words
}
