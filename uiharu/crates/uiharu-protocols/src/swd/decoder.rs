// SPDX-License-Identifier: Apache-2.0

//! The SWD decoder: Arm Serial Wire Debug packets, line resets and the select sequences of a
//! Serial Wire/JTAG debug port, from logic samples.
//!
//! Written from the *Arm Debug Interface Architecture Specification, ADIv5.0 to ADIv5.2*
//! (ARM IHI 0031): its Serial Wire Debug protocol and the switching sequences of the SWJ-DP.
//! The rules:
//!
//! - every bit belongs to a rising edge of SWCLK. The host's bits are read there, where the
//!   target samples them; the target changes SWDIO on the rising edge (B4.3.1), so its bits —
//!   the acknowledge, a read's data and parity — are read at the falling edge before, where
//!   they have settled, whatever the target's output delay;
//! - a line reset is at least 50 clocks in a row with SWDIO high; it is reported when SWDIO is
//!   read low again, or where the samples stop, spanning the whole run;
//! - right after a line reset, the select sequences JTAG-to-SWD (`0xE79E`), SWD-to-JTAG
//!   (`0xE73C`) and SWD-to-dormant (`0xE3BC`), sixteen bits sent least significant first, are
//!   reported as a `switch`; while the bits after a line reset may still be one, nothing is
//!   reported for them, and once they cannot, they are decoded as idle cycles and packets;
//! - the Selection Alert that wakes a dormant target (B5.3.4: 128 bits, then four clocks the
//!   target ignores, then an activation code) is watched for on every bit of the wire. The
//!   Arm CoreSight SW-DP code switches to SWD, the Arm CoreSight JTAG-DP and the JTAG-Serial
//!   codes to JTAG, as a `switch` spanning alert and code; a code the decoder does not know
//!   leaves the port where it was. A packet under way when the alert completes was none: it
//!   ends with `protocol-error`;
//! - packets are decoded only where it is known where they start: from a line reset on, until
//!   a protocol error, a gap or a switch to JTAG or dormant. At the first sample, after a gap,
//!   after a protocol error and after waking to SWD — which leaves the target in the protocol
//!   error state until a line reset — the decoder waits for a line reset rather than for idle
//!   cycles, which the bits of a data phase can imitate; after a switch to JTAG or dormant it
//!   decodes no packet until a line reset and a switch back. A line reset that is not followed
//!   by a select sequence is taken to leave the port in SWD, unless it was switched to JTAG or
//!   dormant;
//! - between packets, a low bit is an idle cycle and a high bit a Start bit — unless the run of
//!   high bits it starts grows into a line reset. A packet is therefore reported from the
//!   first low bit of its request (a well-formed request has one among its first six bits),
//!   and its `begin` is settled there; a run of 8 to 49 high bits is a request of all ones,
//!   which is malformed;
//! - a packet is a request (Start, APnDP, RnW, A\[2\], A\[3\], Parity, Stop, Park), a
//!   turnaround, and the acknowledge, three bits least significant first. After OK, a read
//!   goes on with 32 data bits, least significant first, and a parity bit from the target,
//!   then a turnaround; a write with a turnaround, then 32 data bits and a parity bit from the
//!   host. After WAIT or FAULT comes a turnaround and no data. A turnaround lasts the clocks
//!   the configuration says, 1 to 4 (DLCR.TURNROUND), one unless the debugger changed it;
//! - a packet ends `complete` on its last bit — the read's final turnaround, the write's parity
//!   bit — and `wait` or `fault` on the turnaround after the acknowledge;
//! - a write to debug port address `0xC` is a TARGETSEL write (B4.3.4, SWD protocol version
//!   2): the target does not drive the acknowledge, and the host goes on with the data phase
//!   whatever the line showed. The decoder reports the three bits as read and decodes the
//!   write's data phase after them;
//! - a protocol error ends the packet with `protocol-error` on the bit that shows it: a request
//!   whose parity is wrong, whose Stop bit is not 0 or whose Park bit is not 1; an acknowledge
//!   other than OK, WAIT and FAULT, except after a TARGETSEL write; data whose parity is wrong.
//!   High bits at the end of the broken packet count toward the line reset that brings the
//!   decoder back;
//! - a gap or the end of the samples ends the packet under way `aborted`, and a run of high
//!   bits long enough is a line reset there;
//! - progress is passed on, held back to the first bit read and not reported yet: of a run of
//!   high bits, which may still be a line reset or a request; of the bits after a line reset
//!   that may be a select sequence; of the request, acknowledge or data being read;
//! - a bit decides at most a field of the packet and then its end, in that order.
//!
//! Limits: the turnaround is the configuration's, not followed from writes to DLCR; with
//! overrun detection (CTRL/STAT.ORUNDETECT) WAIT and FAULT carry a data phase the decoder does
//! not expect; the JTAG-to-dormant sequence, defined by the TAP controller's zero-bit scans, is
//! not recognised on SWDIO. No register is named: what `addr` selects depends on APnDP and on
//! the debug and access ports, which is the next layer's business.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use uiharu_core::{
    Change, ChannelMask, InputRef, InputSpec, Io, LogicBlock, MAX_CHANNELS, Node, NodeFuture,
    NodeId, NodeSpec, Outbox, OutputRef, OutputSpec, Props, Provenance, Requires, SignalCursor,
    Tick, TickRange,
};

use super::events::{Ack, EndReason, Interface, SwdEvent, TxId};

/// Clocks in a row with SWDIO high that make a line reset.
pub const LINE_RESET_CLOCKS: u32 = 50;

/// The JTAG-to-SWD select sequence, sent least significant bit first.
pub const JTAG_TO_SWD: u16 = 0xE79E;

/// The SWD-to-JTAG select sequence, sent least significant bit first.
pub const SWD_TO_JTAG: u16 = 0xE73C;

/// The SWD-to-dormant select sequence, sent least significant bit first.
pub const SWD_TO_DS: u16 = 0xE3BC;

/// The Selection Alert that wakes a dormant target: 128 bits, sent least significant bit
/// first.
pub const SELECTION_ALERT: u128 = 0x19BC_0EA2_E3DD_AFE9_8685_2D95_6209_F392;

/// The activation code of an Arm CoreSight SW-DP, as read least significant bit first: the
/// code `0b0101_1000` of ADIv6 Table B5-2, which is sent most significant bit first.
pub const ACTIVATE_SWD: u8 = 0x1A;

/// The activation code of an Arm CoreSight JTAG-DP, as read least significant bit first: the
/// code `0b0101_0000` of ADIv6 Table B5-2, which is sent most significant bit first.
pub const ACTIVATE_JTAG_DP: u8 = 0x0A;

/// Bits in the JTAG-Serial activation code, which are all zero.
pub const JTAG_SERIAL_BITS: u8 = 12;

/// Bits in a select sequence.
const SELECT_BITS: usize = 16;

/// Bits in the Selection Alert.
const ALERT_BITS: usize = 128;

/// Clocks after the Selection Alert whose SWDIO the target ignores, before the activation
/// code.
const ACTIVATION_GAP: u8 = 4;

/// Bits in a packet request.
const REQUEST_BITS: usize = 8;

/// The longest turnaround DLCR.TURNROUND can ask for, in clocks.
pub const MAX_TURNAROUND: u8 = 4;

/// The most bits waiting to be decoded again: the bits after a line reset that turned out to
/// be no select sequence. A request found among them is put back in their place, so it never
/// adds to them; one found on the wire is at most a request and the low bit after it.
const REPLAY_BITS: usize = SELECT_BITS;

/// How the port is wired and set up.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SwdConfig {
    /// The clock channel.
    pub swclk: u8,
    /// The data channel.
    pub swdio: u8,
    /// The turnaround period in clocks, 1 to 4: what the debugger wrote to DLCR.TURNROUND,
    /// one after a reset.
    pub turnaround: u8,
}

impl Default for SwdConfig {
    fn default() -> Self {
        Self {
            swclk: 0,
            swdio: 1,
            turnaround: 1,
        }
    }
}

impl SwdConfig {
    /// The channels the decoder reads.
    pub fn channels(&self) -> impl Iterator<Item = u8> {
        [self.swclk, self.swdio].into_iter()
    }

    /// Checks the configuration.
    pub fn validate(&self) -> Result<(), SwdConfigError> {
        for channel in self.channels() {
            if channel >= MAX_CHANNELS {
                return Err(SwdConfigError::Channel(channel));
            }
        }
        if self.swclk == self.swdio {
            return Err(SwdConfigError::DuplicateChannel(self.swclk));
        }
        if !(1..=MAX_TURNAROUND).contains(&self.turnaround) {
            return Err(SwdConfigError::Turnaround(self.turnaround));
        }
        Ok(())
    }

    /// What the decoder's output port declares: the turnaround it assumes, in clocks.
    pub fn props(&self) -> Props {
        Props::new().with("turnaround", i64::from(self.turnaround))
    }
}

/// Why a configuration is rejected.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum SwdConfigError {
    /// SWCLK and SWDIO are on the same channel.
    #[error("channel {0} is used for two lines")]
    DuplicateChannel(u8),
    /// A channel number is out of range.
    #[error("channel {0} is out of range; channels are 0 to {last}", last = MAX_CHANNELS - 1)]
    Channel(u8),
    /// The turnaround is not 1 to 4 clocks.
    #[error("a turnaround of {0} clocks is out of range; it is 1 to {MAX_TURNAROUND}")]
    Turnaround(u8),
}

/// The SWD decoder node: logic samples in, `swd.events` out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Swd {
    config: SwdConfig,
}

impl Swd {
    /// A decoder for `config`.
    pub fn new(config: SwdConfig) -> Result<Self, SwdConfigError> {
        config.validate()?;
        Ok(Self { config })
    }

    /// The configuration.
    pub fn config(&self) -> &SwdConfig {
        &self.config
    }

    /// The logic input of the decoder added as `node`.
    pub fn input(node: NodeId) -> InputRef<Arc<LogicBlock>> {
        InputRef::of_node(node, 0)
    }

    /// The event output of the decoder added as `node`.
    pub fn events(node: NodeId) -> OutputRef<SwdEvent> {
        OutputRef::of_node(node, 0)
    }
}

impl Node for Swd {
    fn spec(&self) -> NodeSpec {
        let channels = self
            .config
            .channels()
            .max()
            .map_or(0, |max| i64::from(max) + 1);
        NodeSpec {
            name: "swd".into(),
            inputs: vec![InputSpec::new::<Arc<LogicBlock>>(
                "logic",
                Requires::new().at_least("channels", channels),
            )],
            outputs: vec![OutputSpec::new::<SwdEvent>("events", self.config.props())],
        }
    }

    fn start(self: Box<Self>, mut io: Io) -> NodeFuture {
        Box::pin(run(io.signal(0), io.output(0), self.config))
    }
}

/// SWDIO around a rising edge of SWCLK: at the edge, where the target samples what the host
/// drives, and at the falling edge before it, where the host samples what the target drives.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Bit {
    tick: Tick,
    /// SWDIO at the rising edge: the host's bit.
    high: bool,
    /// SWDIO at the falling edge before it: the target's bit. Where no falling edge came
    /// before, since the samples began, the level at the rising edge.
    target: bool,
}

/// What the decoder knows about where packets start.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    /// Nothing: at the first sample, after a gap and after a protocol error.
    Unaligned,
    /// Between packets of an SWD port.
    Swd,
    /// The port was switched to JTAG.
    Jtag,
    /// The port was put into the dormant state, which a Selection Alert and an activation
    /// code leave.
    Dormant,
}

/// High bits in a row, outside packets or at the end of one that broke.
#[derive(Clone, Copy, Debug, Default)]
struct Run {
    count: u32,
    /// The ticks of the first bits: a request's worth, should the run turn out to start one.
    first: [Tick; REQUEST_BITS],
    last: Tick,
    /// Whether the run began between packets of an SWD port: with a Start bit, unless it grows
    /// into a line reset.
    start: bool,
}

impl Run {
    /// Counts a high bit in, `start` saying whether it would be a Start bit; a low bit ends
    /// the run.
    fn track(&mut self, bit: Bit, start: bool) {
        if !bit.high {
            *self = Run::default();
            return;
        }
        if self.count == 0 {
            self.start = start;
        }
        if let Some(slot) = self.first.get_mut(self.count as usize) {
            *slot = bit.tick;
        }
        self.count = self.count.saturating_add(1);
        self.last = bit.tick;
    }

    fn span(&self) -> TickRange {
        TickRange::new(self.first[0], self.last.next())
    }
}

/// The bits after a line reset, while they may be a select sequence.
#[derive(Clone, Copy, Debug, Default)]
struct Select {
    bits: [Bit; SELECT_BITS],
    len: usize,
}

impl Select {
    /// Whether the bits so far are the start of `sequence`, sent least significant bit first.
    fn matches(&self, sequence: u16) -> bool {
        self.bits[..self.len]
            .iter()
            .enumerate()
            .all(|(i, bit)| bit.high == (sequence >> i & 1 != 0))
    }
}

/// The bit at `index` of the Selection Alert.
fn alert_bit(index: usize) -> bool {
    SELECTION_ALERT >> index & 1 != 0
}

/// The activation code being read after a Selection Alert.
#[derive(Clone, Copy, Debug)]
struct Activation {
    /// The alert's first bit.
    start: Tick,
    /// Bits taken since the alert: the ignored clocks, then the code's.
    taken: u8,
    /// The code's bits, bit `i` as the `i`-th on the wire.
    bits: u16,
}

/// What a bit of the wire did to the wake sequence.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Woke {
    /// Nothing yet: the bit is the wire's to decode.
    Nothing,
    /// The last 128 bits were the Selection Alert.
    Alert,
    /// The bit is one the target ignores or one of the activation code, which is not complete.
    Code,
    /// The activation code selected an interface; the sequence began at the tick.
    Switch(Interface, Tick),
    /// The activation code is none the decoder knows: the port stays where it was.
    Unrecognized,
}

/// The Selection Alert and the activation code after it, watched for on every bit of the wire.
struct Wake {
    /// How many bits of the alert the latest bits match, 0 to 128.
    matched: usize,
    /// For every prefix of the alert, the length of the longest shorter prefix that is also
    /// its suffix: where a match goes on from after a bit that breaks it.
    fail: [u8; ALERT_BITS],
    /// The ticks of the last 128 bits read; `pos` counts them.
    ticks: [Tick; ALERT_BITS],
    pos: usize,
    /// The activation code being read, after an alert.
    code: Option<Activation>,
}

impl Wake {
    fn new() -> Self {
        let mut fail = [0u8; ALERT_BITS];
        let mut k = 0usize;
        for i in 1..ALERT_BITS {
            while k > 0 && alert_bit(i) != alert_bit(k) {
                k = usize::from(fail[k - 1]);
            }
            if alert_bit(i) == alert_bit(k) {
                k += 1;
            }
            fail[i] = k as u8;
        }
        Self {
            matched: 0,
            fail,
            ticks: [Tick::ZERO; ALERT_BITS],
            pos: 0,
            code: None,
        }
    }

    /// Takes a bit of the wire in.
    fn feed(&mut self, bit: Bit) -> Woke {
        if let Some(code) = self.code.as_mut() {
            code.taken += 1;
            if code.taken <= ACTIVATION_GAP {
                return Woke::Code;
            }
            let index = code.taken - ACTIVATION_GAP - 1;
            code.bits |= u16::from(bit.high) << index;
            let (start, bits) = (code.start, code.bits);
            let woke = match (index + 1, bits) {
                (8, bits) if bits == u16::from(ACTIVATE_SWD) => Woke::Switch(Interface::Swd, start),
                (8, bits) if bits == u16::from(ACTIVATE_JTAG_DP) => {
                    Woke::Switch(Interface::Jtag, start)
                }
                // Eight zero bits may go on into the JTAG-Serial code.
                (8, 0) => return Woke::Code,
                (8, _) => Woke::Unrecognized,
                (JTAG_SERIAL_BITS, 0) => Woke::Switch(Interface::Jtag, start),
                (JTAG_SERIAL_BITS, _) => Woke::Unrecognized,
                _ => return Woke::Code,
            };
            self.code = None;
            return woke;
        }
        self.ticks[self.pos % ALERT_BITS] = bit.tick;
        self.pos = self.pos.wrapping_add(1);
        while self.matched > 0 && bit.high != alert_bit(self.matched) {
            self.matched = usize::from(self.fail[self.matched - 1]);
        }
        if bit.high == alert_bit(self.matched) {
            self.matched += 1;
        }
        if self.matched < ALERT_BITS {
            return Woke::Nothing;
        }
        self.matched = 0;
        self.code = Some(Activation {
            start: self.start_of(ALERT_BITS),
            taken: 0,
            bits: 0,
        });
        Woke::Alert
    }

    /// The tick of the first of the last `count` bits read.
    fn start_of(&self, count: usize) -> Tick {
        self.ticks[self.pos.wrapping_sub(count) % ALERT_BITS]
    }

    /// The first bit that may still be part of a switch: of the alert and code under way, or
    /// of the bits that match the start of the alert.
    fn in_flight(&self) -> Option<Tick> {
        if let Some(code) = &self.code {
            return Some(code.start);
        }
        (self.matched > 0).then(|| self.start_of(self.matched))
    }

    /// Forgets the bits: the samples stopped.
    fn reset(&mut self) {
        self.matched = 0;
        self.code = None;
    }
}

/// Bits waiting to be decoded again, in order.
#[derive(Clone, Copy, Debug, Default)]
struct Replay {
    bits: [Bit; REPLAY_BITS],
    head: usize,
    len: usize,
}

impl Replay {
    fn pop(&mut self) -> Option<Bit> {
        if self.len == 0 {
            return None;
        }
        let bit = self.bits[self.head];
        self.head = (self.head + 1) % REPLAY_BITS;
        self.len -= 1;
        Some(bit)
    }

    /// Puts `bits` before those waiting.
    fn put_back(&mut self, bits: &[Bit]) {
        debug_assert!(self.len + bits.len() <= REPLAY_BITS, "the replay overflows");
        for bit in bits.iter().rev().take(REPLAY_BITS - self.len) {
            self.head = (self.head + REPLAY_BITS - 1) % REPLAY_BITS;
            self.bits[self.head] = *bit;
            self.len += 1;
        }
    }
}

/// The packet under way. Its bits are numbered from the Start bit: the request is bits 0 to
/// 7, a turnaround of `trn` clocks follows, then the three acknowledge bits; a read's data
/// phase comes right after them and ends with a turnaround, a write's begins with one.
#[derive(Clone, Copy, Debug)]
struct Packet {
    tx: TxId,
    /// Clocks in a turnaround.
    trn: u8,
    /// Bits taken, from the Start bit.
    taken: u8,
    start: Tick,
    /// The request's bits, bit `i` as the `i`-th on the wire.
    request: u8,
    ack: u8,
    ack_first: Tick,
    data: u32,
    data_first: Tick,
}

impl Packet {
    fn new(tx: TxId, trn: u8) -> Self {
        Self {
            tx,
            trn,
            taken: 0,
            start: Tick::ZERO,
            request: 0,
            ack: 0,
            ack_first: Tick::ZERO,
            data: 0,
            data_first: Tick::ZERO,
        }
    }

    /// The index of the first acknowledge bit.
    fn ack_first(&self) -> u8 {
        8 + self.trn
    }

    /// The index of the last acknowledge bit.
    fn ack_last(&self) -> u8 {
        10 + self.trn
    }

    /// The index of the last bit of a packet with a data phase: a read's final turnaround, a
    /// write's parity bit.
    fn last(&self) -> u8 {
        43 + 2 * self.trn
    }

    /// The index of the last bit of a packet without a data phase: the turnaround after WAIT
    /// or FAULT.
    fn last_without_data(&self) -> u8 {
        10 + 2 * self.trn
    }

    fn rnw(&self) -> bool {
        self.request >> 2 & 1 != 0
    }

    /// Whether the request is a write to debug port address `0xC`: TARGETSEL, whose
    /// acknowledge the target does not drive and whose data phase follows whatever it showed.
    fn targetsel(&self) -> bool {
        // APnDP 0, RnW 0, A[2] 1, A[3] 1.
        self.request & 0b1_1110 == 0b1_1000
    }

    /// Whether a data phase follows the acknowledge: after OK, and after any acknowledge of
    /// a TARGETSEL write.
    fn has_data_phase(&self) -> bool {
        Ack::from_bits(self.ack) == Ack::Ok || self.targetsel()
    }

    /// Whether the target drives the bit at `index`: the acknowledge, and a read's data and
    /// parity after OK.
    fn target_drives(&self, index: u8) -> bool {
        (self.ack_first()..=self.ack_last()).contains(&index)
            || (self.rnw() && index > self.ack_last() && Ack::from_bits(self.ack) == Ack::Ok)
    }

    /// The indices of the first data bit and of the parity bit: a read's data follows the
    /// acknowledge, a write's comes after a turnaround.
    fn data_bits(&self) -> (u8, u8) {
        let after_ack = self.ack_last() + 1;
        if self.rnw() {
            (after_ack, after_ack + 32)
        } else {
            (after_ack + self.trn, after_ack + self.trn + 32)
        }
    }

    /// The first bit of the field being read — the request, the acknowledge or the data — which
    /// its event will span from; `None` between fields.
    fn field_start(&self) -> Option<Tick> {
        match self.taken {
            1..=7 => Some(self.start),
            taken if taken > self.ack_first() && taken <= self.ack_last() => Some(self.ack_first),
            taken if self.has_data_phase() => {
                let (first, parity) = self.data_bits();
                (first < taken && taken <= parity).then_some(self.data_first)
            }
            _ => None,
        }
    }
}

/// Settles `at` no earlier than `now`: a bit decoded again is decided by a later one.
fn settle(at: Provenance, now: Tick) -> Provenance {
    Provenance {
        span: at.span,
        settled: at.settled.max(now),
    }
}

struct Decoder {
    /// Clocks in a turnaround.
    trn: u8,
    mode: Mode,
    run: Run,
    select: Option<Select>,
    packet: Option<Packet>,
    replay: Replay,
    wake: Wake,
    next_tx: u64,
    /// The tick of the last bit read from the wire.
    now: Tick,
}

impl Decoder {
    fn new(config: &SwdConfig) -> Self {
        Self {
            trn: config.turnaround,
            mode: Mode::Unaligned,
            run: Run::default(),
            select: None,
            packet: None,
            replay: Replay::default(),
            wake: Wake::new(),
            next_tx: 1,
            now: Tick::ZERO,
        }
    }

    /// The first bit read and not reported yet, which an event may still span from: of a run
    /// of high bits, which may turn into a line reset or be decoded again as a request; of the
    /// bits after a line reset that may be a select sequence; of the packet's field being
    /// read; of the bits that may be a Selection Alert and its activation code. Bits waiting
    /// in the replay need nothing: the wire is read only once they are decoded.
    fn in_flight(&self) -> Option<Tick> {
        let run = (self.run.count > 0).then_some(self.run.first[0]);
        let select = self.select.as_ref().map(|select| select.bits[0].tick);
        let field = self.packet.as_ref().and_then(Packet::field_start);
        let wake = self.wake.in_flight();
        [run, select, field, wake].into_iter().flatten().min()
    }

    /// Progress to `tick`, held back to what is in flight, which promises that nothing
    /// starting earlier follows.
    fn hold(&self, tick: Tick) -> Tick {
        self.in_flight().map_or(tick, |start| start.min(tick))
    }

    /// A bit read from the wire, not one decoded again: watched for the wake sequence, then
    /// decoded.
    async fn wire_bit(&mut self, bit: Bit, out: &mut Outbox<SwdEvent>) {
        match self.wake.feed(bit) {
            Woke::Nothing => self.bit(bit, out).await,
            Woke::Alert => {
                // The last 128 bits were the Selection Alert, whatever they were taken for: a
                // packet under way was none, and they were no line reset or select sequence.
                if self.packet.is_some() {
                    self.end(EndReason::ProtocolError, bit, out).await;
                }
                self.run = Run::default();
                self.select = None;
            }
            Woke::Code | Woke::Unrecognized => {}
            Woke::Switch(to, start) => {
                self.mode = match to {
                    // Woken to SWD, the target is in the protocol error state until a line
                    // reset.
                    Interface::Swd => Mode::Unaligned,
                    Interface::Jtag => Mode::Jtag,
                    Interface::Dormant => Mode::Dormant,
                };
                let span = TickRange::new(start, bit.tick.next());
                out.emit(Provenance::over(span, bit.tick), SwdEvent::Switch { to })
                    .await;
            }
        }
    }

    async fn bit(&mut self, bit: Bit, out: &mut Outbox<SwdEvent>) {
        if self.select.is_some() {
            self.select_bit(bit, out).await;
            return;
        }
        if self.packet.is_some() {
            self.run.track(bit, false);
            self.packet_bit(bit, out).await;
            return;
        }
        if bit.high {
            let start = self.mode == Mode::Swd;
            self.run.track(bit, start);
            return;
        }
        let run = std::mem::take(&mut self.run);
        if run.count >= LINE_RESET_CLOCKS {
            let mut select = Select::default();
            select.bits[0] = bit;
            select.len = 1;
            self.select = Some(select);
            let at = settle(Provenance::over(run.span(), bit.tick), self.now);
            out.emit(at, SwdEvent::LineReset).await;
            return;
        }
        if run.start && self.mode == Mode::Swd {
            // The run began with a Start bit: decode it again as a packet, with this bit when
            // it belongs to the request.
            let tx = TxId(self.next_tx);
            self.next_tx += 1;
            self.packet = Some(Packet::new(tx, self.trn));
            let ones = (run.count as usize).min(REQUEST_BITS);
            let mut bits = [bit; REQUEST_BITS + 1];
            for (slot, tick) in bits.iter_mut().zip(&run.first[..ones]) {
                *slot = Bit {
                    tick: *tick,
                    high: true,
                    target: true,
                };
            }
            self.replay.put_back(&bits[..=ones]);
        }
    }

    /// A bit after a line reset, while the bits may still be a select sequence.
    async fn select_bit(&mut self, bit: Bit, out: &mut Outbox<SwdEvent>) {
        let Some(mut select) = self.select.take() else {
            return;
        };
        select.bits[select.len] = bit;
        select.len += 1;
        let to = [
            (JTAG_TO_SWD, Interface::Swd),
            (SWD_TO_JTAG, Interface::Jtag),
            (SWD_TO_DS, Interface::Dormant),
        ]
        .into_iter()
        .find(|(sequence, _)| select.matches(*sequence))
        .map(|(_, to)| to);
        match to {
            Some(to) if select.len == SELECT_BITS => {
                self.mode = match to {
                    Interface::Swd => Mode::Swd,
                    Interface::Jtag => Mode::Jtag,
                    Interface::Dormant => Mode::Dormant,
                };
                let span = TickRange::new(select.bits[0].tick, bit.tick.next());
                let at = settle(Provenance::over(span, bit.tick), self.now);
                out.emit(at, SwdEvent::Switch { to }).await;
            }
            Some(_) => self.select = Some(select),
            None => {
                // No select sequence: idle cycles and packets, or more JTAG, or a target
                // still dormant.
                if !matches!(self.mode, Mode::Jtag | Mode::Dormant) {
                    self.mode = Mode::Swd;
                }
                self.replay.put_back(&select.bits[..select.len]);
            }
        }
    }

    async fn packet_bit(&mut self, bit: Bit, out: &mut Outbox<SwdEvent>) {
        let now = self.now;
        let Some(packet) = self.packet.as_mut() else {
            return;
        };
        let index = packet.taken;
        packet.taken = packet.taken.saturating_add(1);
        let tx = packet.tx;
        let high = if packet.target_drives(index) {
            bit.target
        } else {
            bit.high
        };
        let level = u8::from(high);
        match index {
            0 => {
                packet.start = bit.tick;
                out.emit(
                    settle(Provenance::one(bit.tick), now),
                    SwdEvent::Begin { tx },
                )
                .await;
            }
            1..=7 => {
                packet.request |= level << index;
                if index < 7 {
                    return;
                }
                let request = packet.request;
                let field = |n: u8| request >> n & 1 != 0;
                let (apndp, rnw, a2, a3) = (field(1), field(2), field(3), field(4));
                let parity_ok = field(5) == (apndp ^ rnw ^ a2 ^ a3);
                let span = TickRange::new(packet.start, bit.tick.next());
                let event = SwdEvent::Request {
                    tx,
                    apndp,
                    rnw,
                    addr: u8::from(a2) << 2 | u8::from(a3) << 3,
                    parity_ok,
                };
                out.emit(settle(Provenance::over(span, bit.tick), now), event)
                    .await;
                // Stop is 0 and Park is 1 in a well-formed request.
                if !parity_ok || field(6) || !field(7) {
                    self.end(EndReason::ProtocolError, bit, out).await;
                }
            }
            // The turnaround to the target.
            index if index < packet.ack_first() => {}
            index if index <= packet.ack_last() => {
                if index == packet.ack_first() {
                    packet.ack_first = bit.tick;
                }
                packet.ack |= level << (index - packet.ack_first());
                if index < packet.ack_last() {
                    return;
                }
                let value = Ack::from_bits(packet.ack);
                let span = TickRange::new(packet.ack_first, bit.tick.next());
                out.emit(
                    settle(Provenance::over(span, bit.tick), now),
                    SwdEvent::Ack { tx, value },
                )
                .await;
                if let Ack::Other(_) = value
                    && !packet.targetsel()
                {
                    self.end(EndReason::ProtocolError, bit, out).await;
                }
            }
            // A TARGETSEL write's data phase follows whatever the acknowledge bits showed.
            _ if packet.targetsel() => self.data_bit(index, bit, high, out).await,
            _ => match Ack::from_bits(packet.ack) {
                // The turnaround back to the host ends the packet on its last clock.
                Ack::Wait | Ack::Fault if index < packet.last_without_data() => {}
                Ack::Wait => self.end(EndReason::Wait, bit, out).await,
                Ack::Fault => self.end(EndReason::Fault, bit, out).await,
                Ack::Ok => self.data_bit(index, bit, high, out).await,
                // The packet ended with the acknowledge.
                Ack::Other(_) => {}
            },
        }
    }

    /// A bit of the data phase after the acknowledge: a read's data, parity and turnaround, or
    /// a write's turnaround, data and parity; `high` is the bit as the side driving it sent it.
    async fn data_bit(&mut self, index: u8, bit: Bit, high: bool, out: &mut Outbox<SwdEvent>) {
        let now = self.now;
        let Some(packet) = self.packet.as_mut() else {
            return;
        };
        let rnw = packet.rnw();
        let (first, parity) = packet.data_bits();
        if index < first {
            return;
        }
        if index < parity {
            if index == first {
                packet.data_first = bit.tick;
            }
            packet.data |= u32::from(high) << (index - first);
            return;
        }
        if index > parity {
            // A read's turnaround back to the host ends the packet on its last clock.
            if index == packet.last() {
                self.end(EndReason::Complete, bit, out).await;
            }
            return;
        }
        let value = packet.data;
        let parity_ok = (value.count_ones() & 1 == 1) == high;
        let span = TickRange::new(packet.data_first, bit.tick.next());
        let event = SwdEvent::Data {
            tx: packet.tx,
            value,
            parity_ok,
            from_target: rnw,
        };
        out.emit(settle(Provenance::over(span, bit.tick), now), event)
            .await;
        if !parity_ok {
            self.end(EndReason::ProtocolError, bit, out).await;
        } else if !rnw {
            self.end(EndReason::Complete, bit, out).await;
        }
    }

    /// Ends the packet under way on `bit`.
    async fn end(&mut self, reason: EndReason, bit: Bit, out: &mut Outbox<SwdEvent>) {
        let Some(packet) = self.packet.take() else {
            return;
        };
        if reason == EndReason::ProtocolError {
            self.mode = Mode::Unaligned;
        } else {
            // The bits of a packet that ended well do not count toward a line reset.
            self.run = Run::default();
        }
        let at = settle(Provenance::one(bit.tick), self.now);
        out.emit(
            at,
            SwdEvent::End {
                tx: packet.tx,
                reason,
            },
        )
        .await;
    }

    /// The samples stop at `tick`, for a gap or for good.
    async fn cut(&mut self, tick: Tick, out: &mut Outbox<SwdEvent>) {
        debug_assert_eq!(
            self.replay.len, 0,
            "the wire is read only once the replay is done"
        );
        self.mode = Mode::Unaligned;
        self.wake.reset();
        let run = std::mem::take(&mut self.run);
        let select = self.select.take();
        if let Some(packet) = self.packet.take() {
            let end = SwdEvent::End {
                tx: packet.tx,
                reason: EndReason::Aborted,
            };
            out.emit(Provenance::at(tick), end).await;
        }
        if select.is_none() && run.count >= LINE_RESET_CLOCKS {
            out.emit(Provenance::over(run.span(), tick), SwdEvent::LineReset)
                .await;
        }
    }
}

async fn run(mut signal: SignalCursor, mut out: Outbox<SwdEvent>, config: SwdConfig) {
    let mut decoder = Decoder::new(&config);
    let clock = ChannelMask::one(config.swclk);
    // SWDIO at the last falling edge of SWCLK, where the target's bit is read.
    let mut fall: Option<bool> = None;
    loop {
        if let Some(bit) = decoder.replay.pop() {
            decoder.bit(bit, &mut out).await;
            continue;
        }
        match signal.next_change(clock).await {
            Change::At { tick, levels, .. } => {
                let swdio = levels.high(config.swdio);
                if levels.high(config.swclk) {
                    decoder.now = tick;
                    let bit = Bit {
                        tick,
                        high: swdio,
                        target: fall.take().unwrap_or(swdio),
                    };
                    decoder.wire_bit(bit, &mut out).await;
                } else {
                    fall = Some(swdio);
                }
            }
            Change::Gap(range) => {
                fall = None;
                decoder.cut(range.start, &mut out).await;
                out.gap(range).await;
            }
            // Without a deadline the wait is never quiet.
            Change::Quiet(_) => {}
            Change::Progress(tick) => out.progress(decoder.hold(tick)).await,
            Change::End(tick) => {
                decoder.cut(tick, &mut out).await;
                out.end(tick).await;
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configurations_are_checked() {
        assert_eq!(SwdConfig::default().validate(), Ok(()));
        assert_eq!(
            SwdConfig {
                swclk: 3,
                swdio: 3,
                ..SwdConfig::default()
            }
            .validate(),
            Err(SwdConfigError::DuplicateChannel(3))
        );
        assert_eq!(
            SwdConfig {
                swdio: 64,
                ..SwdConfig::default()
            }
            .validate(),
            Err(SwdConfigError::Channel(64))
        );
        for turnaround in [0, 5] {
            assert_eq!(
                SwdConfig {
                    turnaround,
                    ..SwdConfig::default()
                }
                .validate(),
                Err(SwdConfigError::Turnaround(turnaround))
            );
        }
        assert_eq!(
            SwdConfig {
                turnaround: 4,
                ..SwdConfig::default()
            }
            .validate(),
            Ok(())
        );
    }

    #[test]
    fn props_and_requirements() {
        let swd = Swd::new(SwdConfig {
            swclk: 9,
            swdio: 2,
            turnaround: 2,
        })
        .expect("valid");
        let spec = swd.spec();
        assert_eq!(spec.outputs[0].props.get("turnaround"), Some(&2i64.into()));
        let requires = &spec.inputs[0].requires;
        assert!(requires.unmet(&Props::new().with("channels", 10)).is_none());
        assert!(requires.unmet(&Props::new().with("channels", 9)).is_some());
    }

    #[test]
    fn the_selection_alert_is_matched_wherever_it_starts() {
        let bit = |tick: u64, high: bool| Bit {
            tick: Tick(tick),
            high,
            target: high,
        };
        let mut wake = Wake::new();
        // It starts 0, 1, 0, 0, 1 (ADIv6 Figure B5-9).
        assert_eq!(
            (0..5).map(alert_bit).collect::<Vec<_>>(),
            [false, true, false, false, true]
        );
        // A run of ones, a false start of twenty bits, then the alert itself.
        let mut tick = 0;
        for _ in 0..8 {
            assert_eq!(wake.feed(bit(tick, true)), Woke::Nothing);
            tick += 1;
        }
        for k in 0..20 {
            assert_eq!(wake.feed(bit(tick, alert_bit(k))), Woke::Nothing);
            tick += 1;
        }
        assert_eq!(wake.in_flight(), Some(Tick(8)));
        let start = tick;
        for k in 0..ALERT_BITS {
            let woke = wake.feed(bit(tick, alert_bit(k)));
            tick += 1;
            if k < ALERT_BITS - 1 {
                assert_eq!(woke, Woke::Nothing, "bit {k}");
            } else {
                assert_eq!(woke, Woke::Alert);
            }
        }
        assert_eq!(wake.in_flight(), Some(Tick(start)));
        // Four clocks the target ignores, then the SW-DP code.
        for high in [true, false, true, true] {
            assert_eq!(wake.feed(bit(tick, high)), Woke::Code);
            tick += 1;
        }
        for i in 0..8 {
            let woke = wake.feed(bit(tick, ACTIVATE_SWD >> i & 1 != 0));
            tick += 1;
            if i < 7 {
                assert_eq!(woke, Woke::Code);
            } else {
                assert_eq!(woke, Woke::Switch(Interface::Swd, Tick(start)));
            }
        }
        assert_eq!(wake.in_flight(), None);
    }

    #[test]
    fn the_activation_codes_are_told_apart() {
        let code = |bits: &[bool]| {
            let mut wake = Wake::new();
            wake.code = Some(Activation {
                start: Tick(1),
                taken: ACTIVATION_GAP,
                bits: 0,
            });
            let mut last = Woke::Nothing;
            for (i, high) in bits.iter().enumerate() {
                last = wake.feed(Bit {
                    tick: Tick(2 + i as u64),
                    high: *high,
                    target: *high,
                });
            }
            last
        };
        let lsb_first = |value: u16, count: usize| -> Vec<bool> {
            (0..count).map(|i| value >> i & 1 != 0).collect()
        };
        assert_eq!(
            code(&lsb_first(u16::from(ACTIVATE_JTAG_DP), 8)),
            Woke::Switch(Interface::Jtag, Tick(1))
        );
        assert_eq!(code(&lsb_first(0, 8)), Woke::Code);
        assert_eq!(
            code(&lsb_first(0, 12)),
            Woke::Switch(Interface::Jtag, Tick(1))
        );
        assert_eq!(
            code(
                &[false; 11]
                    .iter()
                    .chain(&[true])
                    .copied()
                    .collect::<Vec<_>>()
            ),
            Woke::Unrecognized
        );
        assert_eq!(code(&lsb_first(0x5A, 8)), Woke::Unrecognized);
    }

    #[test]
    fn the_select_sequences_part_at_their_second_bit() {
        let mut select = Select::default();
        for (i, high) in [false, true, true, true].into_iter().enumerate() {
            select.bits[i] = Bit {
                tick: Tick(i as u64),
                high,
                target: high,
            };
            select.len = i + 1;
        }
        assert!(select.matches(JTAG_TO_SWD));
        assert!(!select.matches(SWD_TO_JTAG));
    }

    #[test]
    fn bits_put_back_come_first_and_in_order() {
        let bit = |tick: u64| Bit {
            tick: Tick(tick),
            high: tick.is_multiple_of(2),
            target: tick.is_multiple_of(2),
        };
        let mut replay = Replay::default();
        replay.put_back(&[bit(5), bit(6), bit(7)]);
        assert_eq!(replay.pop(), Some(bit(5)));
        replay.put_back(&[bit(1), bit(2)]);
        let order: Vec<Bit> = std::iter::from_fn(|| replay.pop()).collect();
        assert_eq!(order, vec![bit(1), bit(2), bit(6), bit(7)]);
        let all: Vec<Bit> = (0..REPLAY_BITS as u64).map(bit).collect();
        replay.put_back(&all);
        assert_eq!(replay.len, REPLAY_BITS);
        assert_eq!(replay.pop(), Some(bit(0)));
    }
}
