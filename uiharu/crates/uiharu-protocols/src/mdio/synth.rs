// SPDX-License-Identifier: Apache-2.0

//! Making MDIO waveforms: a station clocking management frames on MDC with their bits on MDIO,
//! for tests, tools and benchmarks.

use uiharu_core::{Levels, LogicBlock, LogicBlockBuilder, Tick};

use super::decoder::MdioConfig;

/// The fields of a management frame as the codes it sends, two bits for ST, OP and the
/// turnaround, the first bit in bit 1; any codes may be given, those of no frame included.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Frame {
    /// ST: `01` for clause 22, `00` for clause 45.
    pub st: u8,
    /// OP.
    pub op: u8,
    /// PHYAD or PRTAD, five bits.
    pub phy: u8,
    /// REGAD or DEVAD, five bits.
    pub reg: u8,
    /// The two turnaround bits as they will be on the line.
    pub turnaround: u8,
    /// The sixteen data bits.
    pub data: u16,
}

impl Frame {
    /// The usual turnaround: the station's `10` on a write; on a read the station releases
    /// the line, which the pull-up holds high, and the device drives `0`.
    const TURNAROUND: u8 = 0b10;

    const fn new(st: u8, op: u8, phy: u8, reg: u8, data: u16) -> Self {
        Self {
            st,
            op,
            phy,
            reg,
            turnaround: Self::TURNAROUND,
            data,
        }
    }

    /// A clause 22 read of register `reg` of PHY `phy`, which answers `data`.
    pub const fn c22_read(phy: u8, reg: u8, data: u16) -> Self {
        Self::new(0b01, 0b10, phy, reg, data)
    }

    /// A clause 22 write of `data` to register `reg` of PHY `phy`.
    pub const fn c22_write(phy: u8, reg: u8, data: u16) -> Self {
        Self::new(0b01, 0b01, phy, reg, data)
    }

    /// A clause 45 address frame: device `devad` of port `prtad` takes `address` as its
    /// register address.
    pub const fn c45_address(prtad: u8, devad: u8, address: u16) -> Self {
        Self::new(0b00, 0b00, prtad, devad, address)
    }

    /// A clause 45 write of `data` to the addressed register of device `devad` of port
    /// `prtad`.
    pub const fn c45_write(prtad: u8, devad: u8, data: u16) -> Self {
        Self::new(0b00, 0b01, prtad, devad, data)
    }

    /// A clause 45 read of the addressed register, which answers `data`.
    pub const fn c45_read(prtad: u8, devad: u8, data: u16) -> Self {
        Self::new(0b00, 0b11, prtad, devad, data)
    }

    /// A clause 45 post-read-increment-address read, which answers `data`.
    pub const fn c45_read_increment(prtad: u8, devad: u8, data: u16) -> Self {
        Self::new(0b00, 0b10, prtad, devad, data)
    }

    /// The same frame with the turnaround bits `turnaround`.
    pub const fn with_turnaround(self, turnaround: u8) -> Self {
        Self { turnaround, ..self }
    }

    /// The frame's 32 bits in the order they are sent, each field most significant bit
    /// first.
    pub fn bits(&self) -> [bool; 32] {
        let word = u32::from(self.st & 0b11) << 30
            | u32::from(self.op & 0b11) << 28
            | u32::from(self.phy & 0x1f) << 23
            | u32::from(self.reg & 0x1f) << 18
            | u32::from(self.turnaround & 0b11) << 16
            | u32::from(self.data);
        std::array::from_fn(|i| word >> (31 - i) & 1 != 0)
    }
}

/// Builds the samples of a management interface one bit at a time.
///
/// A bit is half a period with MDC low, MDIO taking the bit's level as it falls, then half a
/// period with MDC high: MDIO is stable at the rising edge, which reads it. Between bits MDC
/// may stop low; MDIO idles high, released to its pull-up.
#[derive(Debug)]
pub struct MdioWaveform {
    config: MdioConfig,
    builder: LogicBlockBuilder,
    /// Samples per half MDC period.
    half_period: u64,
    levels: Levels,
}

impl MdioWaveform {
    /// A waveform on `channels` channels for lines wired as `config`, clocked at
    /// `half_period` samples per half period, starting at `start` with MDC low and MDIO
    /// released.
    pub fn new(config: MdioConfig, channels: u8, half_period: u64, start: Tick) -> Self {
        let mut waveform = Self {
            builder: LogicBlockBuilder::new(start, channels),
            half_period: half_period.max(1),
            levels: Levels(0),
            config,
        };
        waveform.set(waveform.config.mdio, true);
        waveform
    }

    /// The tick the next sample will have.
    pub fn position(&self) -> Tick {
        self.builder.next_tick()
    }

    /// Samples per bit: a whole MDC period.
    pub fn bit_period(&self) -> u64 {
        2 * self.half_period
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

    /// Holds the lines as they are for `samples` samples, MDC stopped.
    pub fn idle(&mut self, samples: u64) {
        self.hold(samples);
    }

    /// Releases MDIO: the pull-up takes it high.
    pub fn release(&mut self) {
        self.set(self.config.mdio, true);
    }

    /// One MDC period carrying `high` on MDIO; MDC rises half a period in.
    pub fn bit(&mut self, high: bool) {
        let (mdc, mdio) = (self.config.mdc, self.config.mdio);
        self.set(mdc, false);
        self.set(mdio, high);
        self.hold(self.half_period);
        self.set(mdc, true);
        self.hold(self.half_period);
        // The falling edge comes with the next sample.
        self.set(mdc, false);
    }

    /// One period per bit of `bits`.
    pub fn bits(&mut self, bits: impl IntoIterator<Item = bool>) {
        for bit in bits {
            self.bit(bit);
        }
    }

    /// `count` periods with MDIO high: a preamble, or idle with MDC running.
    pub fn ones(&mut self, count: u32) {
        for _ in 0..count {
            self.bit(true);
        }
    }

    /// A preamble of `preamble` ones, the frame's 32 bits, then MDIO released.
    pub fn frame(&mut self, preamble: u32, frame: &Frame) {
        self.ones(preamble);
        self.bits(frame.bits());
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The bits as `0` and `1` in the order they are sent.
    fn sent(frame: Frame) -> String {
        frame
            .bits()
            .iter()
            .map(|bit| if *bit { '1' } else { '0' })
            .collect()
    }

    #[test]
    fn frames_are_sent_field_by_field_most_significant_bit_first() {
        // ST, OP, PHYAD, REGAD, TA, data.
        assert_eq!(
            sent(Frame::c22_read(1, 2, 0x1234)),
            ["01", "10", "00001", "00010", "10", "0001001000110100"].concat()
        );
        assert_eq!(
            sent(Frame::c22_write(0x1f, 0, 0xffff)),
            ["01", "01", "11111", "00000", "10", "1111111111111111"].concat()
        );
        assert_eq!(
            sent(Frame::c45_address(3, 1, 0x8001)),
            ["00", "00", "00011", "00001", "10", "1000000000000001"].concat()
        );
        assert!(sent(Frame::c45_write(0, 0, 0)).starts_with("0001"));
        assert!(sent(Frame::c45_read(0, 0, 0)).starts_with("0011"));
        assert!(sent(Frame::c45_read_increment(0, 0, 0)).starts_with("0010"));
        let broken = sent(Frame::c22_write(0, 0, 0).with_turnaround(0b01));
        assert_eq!(&broken[14..16], "01");
    }

    #[test]
    fn a_bit_is_one_period_rising_half_way() {
        let config = MdioConfig::default();
        let mut waveform = MdioWaveform::new(config.clone(), 2, 3, Tick::ZERO);
        waveform.idle(2);
        waveform.bit(false);
        waveform.bit(true);
        waveform.idle(1);
        assert_eq!(waveform.bit_period(), 6);
        let block = waveform.finish();
        let level = |channel: u8, tick: u64| block.level(channel, Tick(tick)).expect("a sample");
        // MDIO idles high, then carries 0 from tick 2 and 1 from tick 8.
        assert!(level(config.mdio, 0));
        assert!(!level(config.mdio, 2) && !level(config.mdio, 7));
        assert!(level(config.mdio, 8));
        // MDC rises at 5 and 11, and is low again after each period.
        assert!(!level(config.mdc, 4) && level(config.mdc, 5) && level(config.mdc, 7));
        assert!(!level(config.mdc, 8) && level(config.mdc, 11) && !level(config.mdc, 14));
    }
}
