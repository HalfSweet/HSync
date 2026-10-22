// SPDX-License-Identifier: Apache-2.0

//! Native SD SDR waveforms, with explicit raw bits for malformed token tests.

use super::{SdcardSdConfig, crc7_bit, crc16_bit};
use uiharu_core::{Levels, LogicBlock, LogicBlockBuilder, Tick};

/// A native SD waveform builder. No decoder state is used to construct checksums.
#[derive(Debug)]
pub struct SdcardSdWaveform {
    config: SdcardSdConfig,
    builder: LogicBlockBuilder,
    half: u64,
    levels: Levels,
}
impl SdcardSdWaveform {
    /// Starts with CLK low and CMD/DAT high.
    pub fn new(config: SdcardSdConfig, channels: u8, half_period: u64, start: Tick) -> Self {
        let mut w = Self {
            config,
            builder: LogicBlockBuilder::new(start, channels),
            half: half_period.max(1),
            levels: Levels(u64::MAX),
        };
        w.line(w.config.clk, false);
        w
    }
    fn line(&mut self, c: u8, high: bool) {
        if high {
            self.levels.0 |= 1 << c;
        } else {
            self.levels.0 &= !(1 << c);
        }
    }
    /// Position of the next sample.
    pub fn position(&self) -> Tick {
        self.builder.next_tick()
    }
    /// Holds current levels for samples (without clock edges).
    pub fn idle(&mut self, samples: u64) {
        self.builder.push_repeated(self.levels, samples);
    }
    /// One rising-edge symbol. `data` packs DAT3..DAT0 in its low bits.
    pub fn symbol(&mut self, cmd: bool, data: u8) {
        self.line(self.config.clk, false);
        self.line(self.config.cmd, cmd);
        for i in 0..self.config.data.len() {
            self.line(self.config.data[i], data & (1 << i) != 0);
        }
        self.idle(self.half);
        self.line(self.config.clk, true);
        self.idle(self.half);
    }
    /// Clock idle bits with all CMD/DAT lines high.
    pub fn clocks(&mut self, count: usize) {
        for _ in 0..count {
            self.symbol(true, 15);
        }
    }
    /// Arbitrary CMD bits, leaving DAT high.
    pub fn bits(&mut self, bits: impl IntoIterator<Item = bool>) {
        for bit in bits {
            self.symbol(bit, 15);
        }
    }
    /// A 48-bit token with its CRC7, optionally replaced by reserved R3 bits.
    pub fn token(&mut self, host: bool, index: u8, argument: u32, no_crc: bool) {
        let mut bits = vec![false, host];
        for bit in (0..6).rev() {
            bits.push(index & (1 << bit) != 0);
        }
        for bit in (0..32).rev() {
            bits.push(argument & (1 << bit) != 0);
        }
        let crc = if no_crc {
            127
        } else {
            bits.iter().fold(0, |crc, &bit| crc7_bit(crc, bit))
        };
        for bit in (0..7).rev() {
            bits.push(crc & (1 << bit) != 0);
        }
        bits.push(true);
        self.bits(bits);
    }
    /// A 136-bit R2 response, including 120 payload bits and internal CRC7.
    pub fn r2(&mut self, payload: [u8; 15]) {
        self.bits([false, false, true, true, true, true, true, true]);
        let mut crc = 0;
        for byte in payload {
            for bit in (0..8).rev() {
                let value = byte & (1 << bit) != 0;
                crc = crc7_bit(crc, value);
                self.symbol(value, 15);
            }
        }
        for bit in (0..7).rev() {
            self.symbol(crc & (1 << bit) != 0, 15);
        }
        self.symbol(true, 15);
    }
    /// A read data block. The caller arms decoding with CMD17/18 first.
    pub fn read_block(&mut self, bytes: &[u8]) {
        let width = self.config.data.len();
        assert!(matches!(width, 1 | 4));
        let mut crcs = [0u16; 4];
        self.symbol(true, 0);
        for &byte in bytes {
            for shift in (0..8 / width).rev() {
                let symbol = (byte >> (shift * width)) & ((1 << width) - 1);
                for (lane, crc) in crcs.iter_mut().enumerate().take(width) {
                    *crc = crc16_bit(*crc, symbol & (1 << lane) != 0);
                }
                self.symbol(true, symbol);
            }
        }
        for bit in (0..16).rev() {
            let symbol = crcs
                .iter()
                .enumerate()
                .take(width)
                .fold(0, |v, (lane, crc)| {
                    v | (u8::from(crc & (1 << bit) != 0) << lane)
                });
            self.symbol(true, symbol);
        }
        self.symbol(true, 15);
    }
    /// Finishes the waveform; at least one sample must have been generated.
    pub fn finish(self) -> LogicBlock {
        self.builder.finish().expect("waveform contains samples")
    }
}
