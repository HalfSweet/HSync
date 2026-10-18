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
//! - right after a line reset, the select sequences JTAG-to-SWD (`0xE79E`) and SWD-to-JTAG
//!   (`0xE73C`), sixteen bits sent least significant first, are reported as a `switch`; while
//!   the bits after a line reset may still be one, nothing is reported for them, and once they
//!   cannot, they are decoded as idle cycles and packets;
//! - packets are decoded only where it is known where they start: from a line reset on, until
//!   a protocol error, a gap or a switch to JTAG. At the first sample, after a gap and after a
//!   protocol error the decoder waits for a line reset rather than for idle cycles, which the
//!   bits of a data phase can imitate; after a switch to JTAG it decodes no packet until a
//!   line reset and a switch back. A line reset that is not followed by a select sequence is
//!   taken to leave the port in SWD, unless it was switched to JTAG;
//! - between packets, a low bit is an idle cycle and a high bit a Start bit — unless the run of
//!   high bits it starts grows into a line reset. A packet is therefore reported from the
//!   first low bit of its request (a well-formed request has one among its first six bits),
//!   and its `begin` is settled there; a run of 8 to 49 high bits is a request of all ones,
//!   which is malformed;
//! - a packet is a request (Start, APnDP, RnW, A\[2\], A\[3\], Parity, Stop, Park), one
//!   turnaround, and the acknowledge, three bits least significant first. After OK, a read
//!   goes on with 32 data bits, least significant first, and a parity bit from the target,
//!   then a turnaround; a write with a turnaround, then 32 data bits and a parity bit from the
//!   host. After WAIT or FAULT comes a turnaround and no data;
//! - a packet ends `complete` on its last bit — the read's final turnaround, the write's parity
//!   bit — and `wait` or `fault` on the turnaround after the acknowledge;
//! - a protocol error ends the packet with `protocol-error` on the bit that shows it: a request
//!   whose parity is wrong, whose Stop bit is not 0 or whose Park bit is not 1; an acknowledge
//!   other than OK, WAIT and FAULT; data whose parity is wrong. High bits at the end of the
//!   broken packet count toward the line reset that brings the decoder back;
//! - a gap or the end of the samples ends the packet under way `aborted`, and a run of high
//!   bits long enough is a line reset there;
//! - progress is passed on, held back to the first bit read and not reported yet: of a run of
//!   high bits, which may still be a line reset or a request; of the bits after a line reset
//!   that may be a select sequence; of the request, acknowledge or data being read;
//! - a bit decides at most a field of the packet and then its end, in that order.
//!
//! Limits: the turnaround lasts one clock, the reset value of DLCR.TURNROUND; with overrun
//! detection (CTRL/STAT.ORUNDETECT) WAIT and FAULT carry a data phase the decoder does not
//! expect; the TARGETSEL write of multi-drop SWD, whose acknowledge no target drives, is
//! reported as a protocol error; the dormant state of ADIv6 and its selection sequences are out
//! of scope. No register is named: what `addr` selects depends on APnDP and on the debug and
//! access ports, which is the next layer's business.

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

/// Bits in a select sequence.
const SELECT_BITS: usize = 16;

/// Bits in a packet request.
const REQUEST_BITS: usize = 8;

/// The most bits waiting to be decoded again: the bits after a line reset that turned out to
/// be no select sequence. A request found among them is put back in their place, so it never
/// adds to them; one found on the wire is at most a request and the low bit after it.
const REPLAY_BITS: usize = SELECT_BITS;

/// How the port is wired.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SwdConfig {
    /// The clock channel.
    pub swclk: u8,
    /// The data channel.
    pub swdio: u8,
}

impl Default for SwdConfig {
    fn default() -> Self {
        Self { swclk: 0, swdio: 1 }
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
        Ok(())
    }

    /// What the decoder's output port declares: the turnaround it assumes, in clocks.
    pub fn props(&self) -> Props {
        Props::new().with("turnaround", 1)
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

/// The packet under way.
#[derive(Clone, Copy, Debug)]
struct Packet {
    tx: TxId,
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
    fn new(tx: TxId) -> Self {
        Self {
            tx,
            taken: 0,
            start: Tick::ZERO,
            request: 0,
            ack: 0,
            ack_first: Tick::ZERO,
            data: 0,
            data_first: Tick::ZERO,
        }
    }

    fn rnw(&self) -> bool {
        self.request >> 2 & 1 != 0
    }

    /// Whether the target drives the bit at `index`: the acknowledge, and a read's data and
    /// parity after OK.
    fn target_drives(&self, index: u8) -> bool {
        (9..=11).contains(&index)
            || (self.rnw() && index >= 12 && Ack::from_bits(self.ack) == Ack::Ok)
    }

    /// The indices of the first data bit and of the parity bit after OK: a read's data follows
    /// the acknowledge, a write's comes after a turnaround.
    fn data_bits(&self) -> (u8, u8) {
        if self.rnw() { (12, 44) } else { (13, 45) }
    }

    /// The first bit of the field being read — the request, the acknowledge or the data — which
    /// its event will span from; `None` between fields.
    fn field_start(&self) -> Option<Tick> {
        match self.taken {
            1..=7 => Some(self.start),
            10..=11 => Some(self.ack_first),
            taken if Ack::from_bits(self.ack) == Ack::Ok => {
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
    mode: Mode,
    run: Run,
    select: Option<Select>,
    packet: Option<Packet>,
    replay: Replay,
    next_tx: u64,
    /// The tick of the last bit read from the wire.
    now: Tick,
}

impl Decoder {
    fn new() -> Self {
        Self {
            mode: Mode::Unaligned,
            run: Run::default(),
            select: None,
            packet: None,
            replay: Replay::default(),
            next_tx: 1,
            now: Tick::ZERO,
        }
    }

    /// The first bit read and not reported yet, which an event may still span from: of a run
    /// of high bits, which may turn into a line reset or be decoded again as a request; of the
    /// bits after a line reset that may be a select sequence; of the packet's field being
    /// read. Bits waiting in the replay need nothing: the wire is read only once they are
    /// decoded.
    fn in_flight(&self) -> Option<Tick> {
        let run = (self.run.count > 0).then_some(self.run.first[0]);
        let select = self.select.as_ref().map(|select| select.bits[0].tick);
        let field = self.packet.as_ref().and_then(Packet::field_start);
        [run, select, field].into_iter().flatten().min()
    }

    /// Progress to `tick`, held back to what is in flight, which promises that nothing
    /// starting earlier follows.
    fn hold(&self, tick: Tick) -> Tick {
        self.in_flight().map_or(tick, |start| start.min(tick))
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
            self.packet = Some(Packet::new(tx));
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
        ]
        .into_iter()
        .find(|(sequence, _)| select.matches(*sequence))
        .map(|(_, to)| to);
        match to {
            Some(to) if select.len == SELECT_BITS => {
                self.mode = match to {
                    Interface::Swd => Mode::Swd,
                    Interface::Jtag => Mode::Jtag,
                };
                let span = TickRange::new(select.bits[0].tick, bit.tick.next());
                let at = settle(Provenance::over(span, bit.tick), self.now);
                out.emit(at, SwdEvent::Switch { to }).await;
            }
            Some(_) => self.select = Some(select),
            None => {
                // No select sequence: idle cycles and packets, or more JTAG.
                if self.mode != Mode::Jtag {
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
            8 => {}
            9..=11 => {
                if index == 9 {
                    packet.ack_first = bit.tick;
                }
                packet.ack |= level << (index - 9);
                if index < 11 {
                    return;
                }
                let value = Ack::from_bits(packet.ack);
                let span = TickRange::new(packet.ack_first, bit.tick.next());
                out.emit(
                    settle(Provenance::over(span, bit.tick), now),
                    SwdEvent::Ack { tx, value },
                )
                .await;
                if let Ack::Other(_) = value {
                    self.end(EndReason::ProtocolError, bit, out).await;
                }
            }
            _ => match Ack::from_bits(packet.ack) {
                // The turnaround back to the host ends the packet.
                Ack::Wait => self.end(EndReason::Wait, bit, out).await,
                Ack::Fault => self.end(EndReason::Fault, bit, out).await,
                Ack::Ok => self.data_bit(index, bit, high, out).await,
                // The packet ended with the acknowledge.
                Ack::Other(_) => {}
            },
        }
    }

    /// A bit of the data phase after OK: a read's data, parity and turnaround from bit 12, or
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
            self.end(EndReason::Complete, bit, out).await;
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
    let mut decoder = Decoder::new();
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
                    decoder.bit(bit, &mut out).await;
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
            SwdConfig { swclk: 3, swdio: 3 }.validate(),
            Err(SwdConfigError::DuplicateChannel(3))
        );
        assert_eq!(
            SwdConfig {
                swclk: 0,
                swdio: 64
            }
            .validate(),
            Err(SwdConfigError::Channel(64))
        );
    }

    #[test]
    fn props_and_requirements() {
        let swd = Swd::new(SwdConfig { swclk: 9, swdio: 2 }).expect("valid");
        let spec = swd.spec();
        assert_eq!(spec.outputs[0].props.get("turnaround"), Some(&1i64.into()));
        let requires = &spec.inputs[0].requires;
        assert!(requires.unmet(&Props::new().with("channels", 10)).is_none());
        assert!(requires.unmet(&Props::new().with("channels", 9)).is_some());
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
