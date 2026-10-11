// SPDX-License-Identifier: Apache-2.0

//! Making JTAG waveforms: a test host driving TCK, TMS, TDI and TRST, and a device whose TAP
//! controller follows them and answers on TDO, for tests, tools and benchmarks.

use uiharu_core::{Levels, LogicBlock, LogicBlockBuilder, Tick};

use super::decoder::{JtagConfig, RESET_CLOCKS};
use super::events::{Register, TapState};

/// Builds the samples of a test access port one clock at a time.
///
/// A clock is half a period with TCK low, then half a period with TCK high. The host changes
/// TMS and TDI, and the device TDO, on the falling edge that starts the low half, so every
/// line is stable at the rising edge. The device's controller starts in Test-Logic-Reset, as
/// after power-up, and follows TMS like the decoder will; in a shift state it shifts out the
/// bits given to [`JtagWaveform::load_tdo`], and TDO idles high (its pull-up) elsewhere.
#[derive(Debug)]
pub struct JtagWaveform {
    config: JtagConfig,
    builder: LogicBlockBuilder,
    /// Samples per half TCK period.
    half_period: u64,
    levels: Levels,
    /// The device's controller.
    state: TapState,
    /// Whether TRST is asserted.
    trst: bool,
    /// What the device shifts out, least significant bit first, and how far it got.
    tdo: Vec<u8>,
    tdo_bits: u32,
    tdo_next: u32,
}

impl JtagWaveform {
    /// A waveform on `channels` channels for lines wired as `config`, clocked at `half_period`
    /// samples per half period, starting at `start` with TCK low, TMS, TDI and TDO high (their
    /// pull-ups) and TRST released.
    pub fn new(config: JtagConfig, channels: u8, half_period: u64, start: Tick) -> Self {
        let mut waveform = Self {
            config,
            builder: LogicBlockBuilder::new(start, channels),
            half_period: half_period.max(1),
            levels: Levels(0),
            state: TapState::TestLogicReset,
            trst: false,
            tdo: Vec::new(),
            tdo_bits: 0,
            tdo_next: 0,
        };
        let (tms, tdi, tdo, trst) = (
            waveform.config.tms,
            waveform.config.tdi,
            waveform.config.tdo,
            waveform.config.trst,
        );
        waveform.set(Some(tms), true);
        waveform.set(tdi, true);
        waveform.set(tdo, true);
        waveform.set(trst, true);
        waveform
    }

    /// The tick the next sample will have.
    pub fn position(&self) -> Tick {
        self.builder.next_tick()
    }

    /// The state of the device's controller.
    pub fn state(&self) -> TapState {
        self.state
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

    /// Holds the lines as they are for `samples` samples, the clock stopped.
    pub fn idle(&mut self, samples: u64) {
        self.hold(samples);
    }

    /// Gives the device `bits` bits to shift out on TDO, least significant bit of `bytes`
    /// first, from the next clock in a shift state on; past them, TDO reads high.
    pub fn load_tdo(&mut self, bytes: &[u8], bits: u32) {
        self.tdo = bytes.to_vec();
        self.tdo_bits = bits;
        self.tdo_next = 0;
    }

    fn tdo_level(&self) -> bool {
        if !self.state.is_shift() || self.tdo_next >= self.tdo_bits {
            return true;
        }
        bit(&self.tdo, self.tdo_next)
    }

    /// One TCK period with TMS and TDI as given: the device's controller moves on, unless
    /// TRST holds it.
    pub fn clock(&mut self, tms: bool, tdi: bool) {
        let tck = Some(self.config.tck);
        self.set(Some(self.config.tms), tms);
        self.set(self.config.tdi, tdi);
        self.set(tck, false);
        self.hold(self.half_period);
        self.set(tck, true);
        self.hold(self.half_period);
        if !self.trst {
            if self.state.is_shift() {
                self.tdo_next = self.tdo_next.saturating_add(1);
            }
            self.state = self.state.next(tms);
        }
        // The falling edge, at the next sample: the device drives its next bit.
        self.set(tck, false);
        let tdo = self.tdo_level();
        self.set(self.config.tdo, tdo);
    }

    /// Clocks the TMS values of `path` in order, with TDI high.
    pub fn clocks(&mut self, path: &[bool]) {
        for tms in path {
            self.clock(*tms, true);
        }
    }

    /// Five clocks with TMS high: Test-Logic-Reset from any state.
    pub fn reset(&mut self) {
        for _ in 0..RESET_CLOCKS {
            self.clock(true, true);
        }
    }

    /// Clocks the shortest TMS path from the device's state to `state`, with TDI high.
    pub fn go_to(&mut self, state: TapState) {
        let path = tms_path(self.state, state);
        self.clocks(&path);
    }

    /// In a shift state, clocks `bits` bits of `tdi`, least significant bit first, with TMS
    /// low; with `exit`, the last one with TMS high, leaving for Exit1.
    pub fn shift(&mut self, tdi: &[u8], bits: u32, exit: bool) {
        for i in 0..bits {
            self.clock(exit && i + 1 == bits, bit(tdi, i));
        }
    }

    /// A whole scan from the device's state: to the shift state of `register`, `bits` bits in
    /// from `tdi` and out from `tdo`, then through Exit1 and Update to Run-Test/Idle. A scan of
    /// no bits goes from Capture straight to Exit1.
    pub fn scan(&mut self, register: Register, tdi: &[u8], tdo: &[u8], bits: u32) {
        self.load_tdo(tdo, bits);
        if bits == 0 {
            self.go_to(match register {
                Register::Ir => TapState::CaptureIr,
                Register::Dr => TapState::CaptureDr,
            });
            self.clock(true, true);
        } else {
            self.go_to(TapState::shift(register));
            self.shift(tdi, bits, true);
        }
        self.go_to(match register {
            Register::Ir => TapState::UpdateIr,
            Register::Dr => TapState::UpdateDr,
        });
        self.go_to(TapState::RunTestIdle);
    }

    /// Asserts TRST (drives it low) for half a period: the device's controller resets and
    /// stays in Test-Logic-Reset until [`JtagWaveform::release_trst`].
    pub fn assert_trst(&mut self) {
        self.set(self.config.trst, false);
        self.trst = true;
        self.state = TapState::TestLogicReset;
        let tdo = self.tdo_level();
        self.set(self.config.tdo, tdo);
        self.hold(self.half_period);
    }

    /// Releases TRST, then waits half a period.
    pub fn release_trst(&mut self) {
        self.set(self.config.trst, true);
        self.trst = false;
        self.hold(self.half_period);
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

/// Bit `index` of `bytes`, least significant bit first; past the end, low.
fn bit(bytes: &[u8], index: u32) -> bool {
    let byte = bytes.get(index as usize / 8).copied().unwrap_or(0);
    byte >> (index % 8) & 1 != 0
}

/// The shortest TMS sequence that takes the controller from `from` to `to`; among equally
/// short ones, the one that keeps TMS low the longest at the start.
pub fn tms_path(from: TapState, to: TapState) -> Vec<bool> {
    let mut previous: [Option<(TapState, bool)>; 16] = [None; 16];
    let mut seen = [false; 16];
    let mut queue = [from; 16];
    let (mut head, mut tail) = (0, 1);
    seen[from.index()] = true;
    while head < tail && !seen[to.index()] {
        let state = queue[head];
        head += 1;
        for tms in [false, true] {
            let next = state.next(tms);
            if !seen[next.index()] {
                seen[next.index()] = true;
                previous[next.index()] = Some((state, tms));
                queue[tail] = next;
                tail += 1;
            }
        }
    }
    let mut path = Vec::new();
    let mut at = to;
    while at != from {
        let Some((before, tms)) = previous[at.index()] else {
            break;
        };
        path.push(tms);
        at = before;
    }
    path.reverse();
    path
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_are_the_shortest() {
        assert_eq!(
            tms_path(TapState::RunTestIdle, TapState::ShiftDr),
            vec![true, false, false]
        );
        assert_eq!(
            tms_path(TapState::RunTestIdle, TapState::ShiftIr),
            vec![true, true, false, false]
        );
        assert_eq!(
            tms_path(TapState::Exit1Dr, TapState::ShiftDr),
            vec![false, true, false]
        );
        assert_eq!(
            tms_path(TapState::RunTestIdle, TapState::TestLogicReset),
            vec![true, true, true]
        );
        assert!(tms_path(TapState::PauseIr, TapState::PauseIr).is_empty());
        let walk = |from: TapState, len: usize, code: u32| {
            (0..len).fold(from, |state, i| state.next(code >> i & 1 != 0))
        };
        for from in TapState::ALL {
            for to in TapState::ALL {
                let path = tms_path(from, to);
                let end = path.iter().fold(from, |state, tms| state.next(*tms));
                assert_eq!(end, to, "{from:?} to {to:?}");
                // No shorter sequence gets there.
                for len in 0..path.len() {
                    for code in 0..1u32 << len {
                        assert_ne!(walk(from, len, code), to, "{from:?} to {to:?}");
                    }
                }
            }
        }
    }

    #[test]
    fn the_device_follows_tms() {
        let mut waveform = JtagWaveform::new(JtagConfig::default(), 4, 2, Tick::ZERO);
        waveform.clock(false, true);
        assert_eq!(waveform.state(), TapState::RunTestIdle);
        waveform.scan(Register::Ir, &[0b1010], &[0b0001], 4);
        assert_eq!(waveform.state(), TapState::RunTestIdle);
        waveform.go_to(TapState::PauseDr);
        assert_eq!(waveform.state(), TapState::PauseDr);
        waveform.reset();
        assert_eq!(waveform.state(), TapState::TestLogicReset);
        // One clock, four samples, per call.
        let before = waveform.position();
        waveform.clock(false, false);
        assert_eq!(waveform.position(), before.offset(4));
    }
}
