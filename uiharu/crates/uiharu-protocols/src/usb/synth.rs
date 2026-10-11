// SPDX-License-Identifier: Apache-2.0

//! Making USB waveforms — low- and full-speed packets and the bus signalling between them — for
//! tests, the command line's generator and benchmarks.
//!
//! The CRCs are computed by polynomial long division over the bits of a packet, a reference
//! written apart from the shift register the decoder checks them with.

use uiharu_core::{Levels, LogicBlock, LogicBlockBuilder, Tick};

use super::decoder::{LineState, UsbConfig};
use super::events::Pid;

/// The low-speed bit rate, which the EOP of resume signalling keeps at every speed.
const LOW_SPEED: u64 = 1_500_000;

/// Builds the samples of a low- or full-speed bus one packet or bus state at a time.
#[derive(Debug)]
pub struct UsbWaveform {
    config: UsbConfig,
    builder: LogicBlockBuilder,
    start: Tick,
    /// Samples per second.
    rate: u64,
    /// The transmitter's bits per second.
    bit_rate: u64,
    /// Where the next cell begins, in 1 / `bit_rate` samples from `start`.
    time: u128,
    state: LineState,
    /// 1 bits in a row, for bit stuffing.
    ones: u8,
    /// Samples by which the falling line leads the rising one at a J/K transition.
    skew: u64,
}

impl UsbWaveform {
    /// A waveform on `channels` channels for a bus wired as `config`, sampled at `rate_hz`,
    /// starting at `start` with the bus idle; the transmitter keeps the speed's bit rate.
    pub fn new(config: UsbConfig, channels: u8, rate_hz: u64, start: Tick) -> Self {
        let bit_rate = config.speed.bit_rate_hz();
        Self {
            config,
            builder: LogicBlockBuilder::new(start, channels),
            start,
            rate: rate_hz.max(1),
            bit_rate,
            time: 0,
            state: LineState::J,
            ones: 0,
            skew: 0,
        }
    }

    /// The same waveform with a transmitter running at `bit_rate_hz` instead, to test how far
    /// a clock may be off.
    pub fn with_bit_rate(mut self, bit_rate_hz: u64) -> Self {
        self.bit_rate = bit_rate_hz.max(1);
        self
    }

    /// The same waveform with J/K transitions whose falling line leads the rising one by
    /// `samples`, passing through SE0.
    pub fn with_skew(mut self, samples: u64) -> Self {
        self.skew = samples;
        self
    }

    /// The tick the next sample will have.
    pub fn position(&self) -> Tick {
        self.builder.next_tick()
    }

    fn levels(&self, state: LineState) -> Levels {
        let (dp, dm) = self.config.lines(state);
        Levels((u64::from(dp) << self.config.dp) | (u64::from(dm) << self.config.dm))
    }

    /// Holds `state` until `time`, in 1 / `bit_rate` samples from the start.
    fn hold_until(&mut self, state: LineState, time: u128) {
        let ticks = u64::try_from(time / u128::from(self.bit_rate)).unwrap_or(u64::MAX);
        let mut count = self.builder.next_tick().until(self.start.offset(ticks));
        let differential = |state: LineState| matches!(state, LineState::J | LineState::K);
        if self.skew > 0 && state != self.state && differential(state) && differential(self.state) {
            let transient = self.skew.min(count);
            self.builder
                .push_repeated(self.levels(LineState::Se0), transient);
            count -= transient;
        }
        self.builder.push_repeated(self.levels(state), count);
        self.time = time;
        self.state = state;
    }

    /// One bit time of `state`.
    fn cell(&mut self, state: LineState) {
        self.hold_until(state, self.time + u128::from(self.rate));
    }

    /// Holds `state` for `samples` samples.
    pub fn hold(&mut self, state: LineState, samples: u64) {
        let time = self.time + u128::from(samples) * u128::from(self.bit_rate);
        self.hold_until(state, time);
    }

    /// The idle bus, J, for `samples` samples.
    pub fn idle(&mut self, samples: u64) {
        self.hold(LineState::J, samples);
    }

    /// The idle bus for `bits` bit times.
    pub fn idle_bits(&mut self, bits: u64) {
        let time = self.time + u128::from(bits) * u128::from(self.rate);
        self.hold_until(LineState::J, time);
    }

    /// One NRZI bit: a 0 changes the differential state, a 1 keeps it.
    fn send(&mut self, one: bool) {
        let next = if one {
            self.state
        } else {
            self.state.toggled()
        };
        self.cell(next);
    }

    /// Sends one bit of a packet, with a stuffed 0 after six 1s in a row.
    pub fn bit(&mut self, one: bool) {
        self.send(one);
        if !one {
            self.ones = 0;
            return;
        }
        self.ones += 1;
        if self.ones == 6 {
            self.send(false);
            self.ones = 0;
        }
    }

    /// Sends the `count` low bits of `value`, the least significant first.
    pub fn bits(&mut self, value: u64, count: u32) {
        for i in 0..count {
            self.bit((value >> i) & 1 != 0);
        }
    }

    /// Sends a byte, the least significant bit first.
    pub fn byte(&mut self, byte: u8) {
        self.bits(u64::from(byte), 8);
    }

    /// Sends `count` 1 bits without stuffing: a bit stuff error once more than six are in a
    /// row.
    pub fn unstuffed_ones(&mut self, count: u32) {
        for _ in 0..count {
            self.send(true);
        }
        self.ones = 0;
    }

    /// The SYNC pattern, `KJKJKJKK`, from the idle bus.
    pub fn sync(&mut self) {
        self.sync_with(7);
    }

    /// A SYNC of `zeros` alternating cells, from K, before the final repeat: seven make the
    /// pattern in full; an even count ends in two J, which is no SYNC.
    pub fn sync_with(&mut self, zeros: u32) {
        if self.state != LineState::J {
            self.idle_bits(1);
        }
        for _ in 0..zeros {
            self.send(false);
        }
        self.send(true);
        self.ones = 1;
    }

    /// The PID byte of `pid`: its type and the type's complement.
    pub fn pid(&mut self, pid: Pid) {
        self.byte(pid.byte());
    }

    /// End of packet: SE0 for two bit times, then J for one.
    pub fn eop(&mut self) {
        self.cell(LineState::Se0);
        self.cell(LineState::Se0);
        self.cell(LineState::J);
        self.ones = 0;
    }

    /// A token packet — OUT, IN, SETUP or PING — with its CRC5.
    pub fn token(&mut self, pid: Pid, address: u8, endpoint: u8) {
        let crc = crc5(token_fields(address, endpoint), 11);
        self.token_with_crc(pid, address, endpoint, crc);
    }

    /// A token packet with the CRC5 field given, as it is sent.
    pub fn token_with_crc(&mut self, pid: Pid, address: u8, endpoint: u8, crc5: u8) {
        self.sync();
        self.pid(pid);
        self.bits(u64::from(token_fields(address, endpoint)), 11);
        self.bits(u64::from(crc5), 5);
        self.eop();
    }

    /// A start-of-frame packet with its CRC5.
    pub fn sof(&mut self, frame: u16) {
        let crc = crc5(u32::from(frame & 0x7ff), 11);
        self.sof_with_crc(frame, crc);
    }

    /// A start-of-frame packet with the CRC5 field given.
    pub fn sof_with_crc(&mut self, frame: u16, crc5: u8) {
        self.sync();
        self.pid(Pid::Sof);
        self.bits(u64::from(frame & 0x7ff), 11);
        self.bits(u64::from(crc5), 5);
        self.eop();
    }

    /// A data packet with its CRC16.
    pub fn data(&mut self, pid: Pid, payload: &[u8]) {
        self.data_with_crc(pid, payload, crc16(payload));
    }

    /// A data packet with the CRC16 field given, as the packet's last two bytes read
    /// little-endian.
    pub fn data_with_crc(&mut self, pid: Pid, payload: &[u8], crc16: u16) {
        self.sync();
        self.pid(pid);
        for byte in payload {
            self.byte(*byte);
        }
        for byte in crc16.to_le_bytes() {
            self.byte(byte);
        }
        self.eop();
    }

    /// A handshake packet: ACK, NAK, STALL or NYET.
    pub fn handshake(&mut self, pid: Pid) {
        self.sync();
        self.pid(pid);
        self.eop();
    }

    /// A SPLIT packet carrying the 24 bits of `raw`, first bit in bit 0.
    pub fn split(&mut self, raw: u32) {
        self.sync();
        self.pid(Pid::Split);
        self.bits(u64::from(raw), 24);
        self.eop();
    }

    /// A preamble: SYNC and the PRE PID, with no EOP.
    pub fn preamble(&mut self) {
        self.sync();
        self.pid(Pid::Pre);
    }

    /// SE0 for `samples` samples, then J for a bit time: a reset when it lasts 2.5 µs or
    /// more.
    pub fn reset(&mut self, samples: u64) {
        self.hold(LineState::Se0, samples);
        self.idle_bits(1);
    }

    /// A low-speed keep-alive: an EOP on the idle bus.
    pub fn keep_alive(&mut self) {
        self.eop();
    }

    /// Resume signalling: K for `samples` samples, then a low-speed EOP — SE0 for two
    /// low-speed bit times, then J for one — at any speed.
    pub fn resume(&mut self, samples: u64) {
        self.hold(LineState::K, samples);
        let low_bit = u128::from(self.rate) * u128::from(self.bit_rate) / u128::from(LOW_SPEED);
        self.hold_until(LineState::Se0, self.time + 2 * low_bit);
        self.hold_until(LineState::J, self.time + low_bit);
        self.ones = 0;
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

/// The 11 bits of a token after its PID: the address, then the endpoint.
pub fn token_fields(address: u8, endpoint: u8) -> u32 {
    u32::from(address & 0x7f) | (u32::from(endpoint & 0x0f) << 7)
}

/// The CRC5 of the `count` low bits of `value`, sent least significant first, as the CRC5
/// field is sent and read least significant bit first.
pub fn crc5(value: u32, count: u32) -> u8 {
    let message: Vec<bool> = (0..count).map(|i| (value >> i) & 1 != 0).collect();
    field(&reference_crc(&message, 0b10_0101, 5)) as u8
}

/// The CRC16 of `bytes`, each sent least significant bit first, as the CRC16 field is read:
/// the packet's last two bytes, little-endian.
pub fn crc16(bytes: &[u8]) -> u16 {
    let message: Vec<bool> = bytes
        .iter()
        .flat_map(|byte| (0..8).map(move |i| (byte >> i) & 1 != 0))
        .collect();
    field(&reference_crc(&message, 0x1_8005, 16)) as u16
}

/// The CRC bits of `message`, in the order they are sent: the remainder of
/// `M(x)·x^width + (x^(width−1) + … + 1)·x^len` — the message, its first bit the highest
/// power, with the effect of the generator's all-ones seed — divided by `generator`, of
/// degree `width`, and inverted.
fn reference_crc(message: &[bool], generator: u32, width: usize) -> Vec<bool> {
    let len = message.len();
    let mut dividend = vec![false; len + width];
    dividend[..len].copy_from_slice(message);
    for coefficient in dividend.iter_mut().take(width) {
        *coefficient = !*coefficient;
    }
    for i in 0..len {
        if dividend[i] {
            for k in 0..=width {
                dividend[i + k] ^= (generator >> (width - k)) & 1 != 0;
            }
        }
    }
    dividend[len..].iter().map(|bit| !bit).collect()
}

/// Bits in the order they are sent as a number, the first in bit 0.
fn field(bits: &[bool]) -> u32 {
    bits.iter()
        .enumerate()
        .map(|(i, bit)| u32::from(*bit) << i)
        .sum()
}
