// SPDX-License-Identifier: Apache-2.0

//! Making SWD waveforms: a debug host sending packets, idle cycles, line resets and select
//! sequences, and a target answering them, for tests, tools and benchmarks.

use uiharu_core::{Levels, LogicBlock, LogicBlockBuilder, Tick};

use super::decoder::{JTAG_TO_SWD, SWD_TO_JTAG, SwdConfig};
use super::events::{Ack, Interface};

/// One packet as the host sends it and the target answers it, with knobs for the errors a
/// decoder must notice.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SwdPacket {
    /// APnDP: an access port request rather than a debug port one.
    pub apndp: bool,
    /// RnW: a read rather than a write.
    pub rnw: bool,
    /// A\[3:2\] as a byte address: `0x0`, `0x4`, `0x8` or `0xC`; other bits are ignored.
    pub addr: u8,
    /// The acknowledge the target sends. After an acknowledge other than OK, WAIT and FAULT
    /// the waveform stops where the packet broke, as a host that saw it would.
    pub ack: Ack,
    /// The word the target reads out or the host writes.
    pub data: u32,
    /// Sends the request's parity bit inverted.
    pub bad_request_parity: bool,
    /// The Stop bit as sent; 0 in a well-formed request.
    pub stop: bool,
    /// The Park bit as sent; 1 in a well-formed request.
    pub park: bool,
    /// Sends the data's parity bit inverted.
    pub bad_data_parity: bool,
}

impl SwdPacket {
    /// A well-formed read of `addr` that the target acknowledges OK and answers with `data`.
    pub fn read(apndp: bool, addr: u8, data: u32) -> Self {
        Self {
            apndp,
            rnw: true,
            addr,
            ack: Ack::Ok,
            data,
            bad_request_parity: false,
            stop: false,
            park: true,
            bad_data_parity: false,
        }
    }

    /// A well-formed write of `data` to `addr` that the target acknowledges OK.
    pub fn write(apndp: bool, addr: u8, data: u32) -> Self {
        Self {
            rnw: false,
            ..Self::read(apndp, addr, data)
        }
    }

    /// The same packet answered with `ack`.
    pub fn with_ack(self, ack: Ack) -> Self {
        Self { ack, ..self }
    }

    /// The request's eight bits as sent, the Start bit first.
    pub fn request_bits(&self) -> u8 {
        let a2 = self.addr >> 2 & 1 != 0;
        let a3 = self.addr >> 3 & 1 != 0;
        let parity = (self.apndp ^ self.rnw ^ a2 ^ a3) != self.bad_request_parity;
        [
            true, self.apndp, self.rnw, a2, a3, parity, self.stop, self.park,
        ]
        .iter()
        .enumerate()
        .fold(0, |bits, (i, bit)| bits | u8::from(*bit) << i)
    }
}

/// Builds the samples of an SWD port one clock at a time.
///
/// A clock is half a period with SWCLK low, then half a period with it high, so that SWCLK
/// rests high between clocks. Whoever drives SWDIO changes it on the falling edge that starts
/// the low half — the host for requests, idle cycles and written data, the target for the
/// acknowledge and read data — so it is stable at the rising edge. In a turnaround nobody
/// drives SWDIO and its pull-up holds it high.
#[derive(Debug)]
pub struct SwdWaveform {
    config: SwdConfig,
    builder: LogicBlockBuilder,
    /// Samples per half SWCLK period.
    half_period: u64,
    levels: Levels,
}

impl SwdWaveform {
    /// A waveform on `channels` channels for a port wired as `config`, clocked at
    /// `half_period` samples per half period, starting at `start` with SWCLK high and SWDIO
    /// low.
    pub fn new(config: SwdConfig, channels: u8, half_period: u64, start: Tick) -> Self {
        let mut waveform = Self {
            builder: LogicBlockBuilder::new(start, channels),
            half_period: half_period.max(1),
            levels: Levels(0),
            config,
        };
        waveform.set(waveform.config.swclk, true);
        waveform
    }

    /// The tick the next sample will have.
    pub fn position(&self) -> Tick {
        self.builder.next_tick()
    }

    /// Samples per clock.
    pub fn period(&self) -> u64 {
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

    /// Holds the lines as they are for `samples` samples, the clock stopped.
    pub fn idle(&mut self, samples: u64) {
        self.hold(samples);
    }

    /// One clock with SWDIO at `high`.
    pub fn clock(&mut self, high: bool) {
        let (swclk, swdio) = (self.config.swclk, self.config.swdio);
        self.set(swclk, false);
        self.set(swdio, high);
        self.hold(self.half_period);
        self.set(swclk, true);
        self.hold(self.half_period);
    }

    /// Clocks the `count` low bits of `bits`, least significant first.
    pub fn bits(&mut self, bits: u64, count: u32) {
        for i in 0..count.min(64) {
            self.clock(bits >> i & 1 != 0);
        }
    }

    /// `cycles` idle cycles: clocks with SWDIO low.
    pub fn idle_cycles(&mut self, cycles: u32) {
        for _ in 0..cycles {
            self.clock(false);
        }
    }

    /// `cycles` clocks with SWDIO high: a line reset when there are at least 50.
    pub fn line_reset(&mut self, cycles: u32) {
        for _ in 0..cycles {
            self.clock(true);
        }
    }

    /// The sixteen-bit sequence that selects `to`, least significant bit first.
    pub fn select(&mut self, to: Interface) {
        let sequence = match to {
            Interface::Swd => JTAG_TO_SWD,
            Interface::Jtag => SWD_TO_JTAG,
        };
        self.bits(u64::from(sequence), 16);
    }

    /// What a debugger sends to take over a port that may be in JTAG: a line reset, the
    /// JTAG-to-SWD sequence, another line reset and two idle cycles.
    pub fn connect(&mut self) {
        self.line_reset(56);
        self.select(Interface::Swd);
        self.line_reset(56);
        self.idle_cycles(2);
    }

    /// One packet: the request, a turnaround and the acknowledge; after OK, the read data,
    /// its parity and a turnaround, or a turnaround, the written data and its parity; after
    /// WAIT and FAULT, a turnaround; after anything else, nothing.
    pub fn packet(&mut self, packet: &SwdPacket) {
        self.bits(u64::from(packet.request_bits()), 8);
        self.clock(true);
        self.bits(u64::from(packet.ack.bits()), 3);
        let parity = (packet.data.count_ones() & 1 == 1) != packet.bad_data_parity;
        match packet.ack {
            Ack::Ok if packet.rnw => {
                self.bits(u64::from(packet.data), 32);
                self.clock(parity);
                self.clock(true);
            }
            Ack::Ok => {
                self.clock(true);
                self.bits(u64::from(packet.data), 32);
                self.clock(parity);
            }
            Ack::Wait | Ack::Fault => self.clock(true),
            Ack::Other(_) => {}
        }
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

    #[test]
    fn requests_carry_even_parity() {
        // A debug port read of 0x0 (DPIDR on most ports): Start, 0, 1, 0, 0, parity 1, 0, 1.
        assert_eq!(SwdPacket::read(false, 0x0, 0).request_bits(), 0b1010_0101);
        // An access port write of 0xC: Start, 1, 0, 1, 1, parity 1, 0, 1.
        assert_eq!(SwdPacket::write(true, 0xc, 0).request_bits(), 0b1011_1011);
        let bad = SwdPacket {
            bad_request_parity: true,
            ..SwdPacket::read(false, 0x0, 0)
        };
        assert_eq!(bad.request_bits(), 0b1000_0101);
    }

    #[test]
    fn packets_take_their_length_in_clocks() {
        let config = SwdConfig::default();
        let clocks = |packet: SwdPacket| {
            let mut waveform = SwdWaveform::new(config.clone(), 2, 1, Tick::ZERO);
            waveform.packet(&packet);
            waveform.position().get() / waveform.period()
        };
        assert_eq!(clocks(SwdPacket::read(false, 0, 1)), 46);
        assert_eq!(clocks(SwdPacket::write(true, 4, 1)), 46);
        assert_eq!(clocks(SwdPacket::read(false, 0, 1).with_ack(Ack::Wait)), 13);
        assert_eq!(
            clocks(SwdPacket::read(false, 0, 1).with_ack(Ack::Other(7))),
            12
        );
    }
}
