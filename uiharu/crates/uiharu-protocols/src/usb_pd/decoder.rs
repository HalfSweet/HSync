// SPDX-License-Identifier: Apache-2.0

//! The USB Power Delivery decoder: the packets on the CC line — messages, Hard Reset and Cable
//! Reset signalling — read from logic samples of the line's BMC signal.
//!
//! Implemented from the Universal Serial Bus Power Delivery Specification, revision 3.1
//! (usb.org): chapter 5, the physical layer (BMC signalling, the preamble, the 4b5b symbols and
//! K-codes, the ordered sets, the CRC and the end of packet), and chapter 6, the protocol layer
//! (the message header, the extended message header, the message types and the power and
//! request data objects). The rules:
//!
//! - **The line.** One channel carries the CC line as a logic level, through a comparator. BMC
//!   carries its information in transitions alone, so either level may be the idle one and
//!   either polarity decodes.
//! - **BMC.** Every bit begins with a transition; a 1 has a second transition in its middle, a
//!   0 none. A transition that comes before three quarters of the bit period after the bit's
//!   first is its middle transition, one at or after it the next bit's first.
//! - **Bit period.** The preamble's first transition after the line was idle starts bit 0,
//!   which the next transition ends. Bit 1 is timed by bit 0, and every later bit of the
//!   preamble by the bits before it: the period is the time from the first transition to the
//!   start of the bit, divided by the bits since. The 64 bits of the preamble give the period
//!   for the rest of the packet; the bit rate they show must lie within `tolerance` percent of
//!   `bit_rate`, allowing one tick of sampling, or the packet ends `timing`.
//! - **Resynchronisation.** Each bit is timed from the transition that began it: its middle
//!   transition must come half a period after it, the next bit's first transition a period
//!   after it, each within `tolerance` percent of the period plus one tick and the period's
//!   own uncertainty, in integer arithmetic; no bit lasts as long as the line takes to be idle.
//!   A transition elsewhere, or none by the latest time, ends the packet `timing`.
//! - **Idle.** The line is idle once it has held its level for longer than any bit the
//!   decoder accepts: ⌈`rate` · (100 + `tolerance`) / (`bit_rate` · (100 − `tolerance`))⌉ + 2
//!   ticks, about 1.2 bit periods at the default tolerance. After the first sample, after a
//!   gap and after every packet the decoder waits for the line to be idle; the next transition
//!   may start a preamble. A packet under way at the first sample or after a gap is therefore
//!   skipped, and so are the edges a transmitter makes after a packet's final transition.
//! - **Begin.** A preamble starts with 0 then 1. A first transition whose bit 0 is shorter
//!   than the shortest bit the tolerance allows, ⌊`rate` · (100 − `tolerance`) / (`bit_rate`
//!   · (100 + `tolerance`))⌋ − 1 ticks, or lasts until the line is idle, or whose bit 1 is no
//!   1, starts no packet and is not reported. Once these two bits are read the packet is open
//!   and is reported, whatever happens next; `begin` covers the preamble. A preamble whose
//!   bits are outside the tolerance but within these bounds — about twice the tolerance — is
//!   therefore reported, and ends `timing`; one further off is not recognised.
//! - **Symbols.** After the 64 bits of the preamble come five-bit symbols, sent least
//!   significant bit first. The first four must form an ordered set — SOP, SOP', SOP'',
//!   SOP'_Debug, SOP''_Debug, Hard Reset or Cable Reset — with all four K-codes in place or
//!   three of them, the other one corrupted (§5.4); four symbols one K-code away from two sets
//!   end the packet `ordered-set`, as do symbols that make no set.
//! - **Resets.** Hard Reset and Cable Reset signalling are reported as such and end the packet
//!   after their ordered set (`hard-reset`, `cable-reset`).
//! - **Messages.** After SOP*, the 4b5b data symbols carry nibbles, the low nibble of each
//!   byte first and the bytes of each field least significant first: the 16-bit header, then
//!   — as many as the header says — 32-bit data objects, or, in an extended message, the
//!   16-bit extended header and the data bytes; then the 32-bit CRC; then the EOP K-code,
//!   whose last bit the final transition ends.
//! - **Extended messages.** A chunked message carries `4 · objects − 2` bytes after its
//!   extended header, padding included. An unchunked message is read up to its EOP: its last
//!   four bytes are the CRC, the bytes before them its data; each byte is reported once four
//!   more have followed it.
//! - **Companions.** Every object of a Source_Capabilities or Sink_Capabilities message is
//!   also read as a power data object (`pdo`), the first object of a Request as a request data
//!   object (`rdo`); other objects are reported raw only.
//! - **CRC.** The CRC-32 is checked by running it over the header, the data and the CRC field
//!   in the order they are sent, which leaves the residual `0xC704DD7B` when nothing was
//!   damaged; `crc` reports the field and whether it matched.
//! - **Errors.** A five-bit code the table does not have, or a K-code other than EOP among the
//!   data, ends the packet `symbol`; an EOP before the fields the header announces
//!   `early-eop`; a symbol other than EOP after the CRC `missing-eop`. An `error` event says
//!   which, then `end` with `error`.
//! - **Provenance.** A bit spans from the transition that begins it to the one that ends it,
//!   and is settled at that last transition, where it is known to be whole. An event spans the
//!   bits it was read from and settles at its last bit's end; `begin` spans the preamble read,
//!   `pdo` and `rdo` share their object's provenance. An error spans the symbol at fault, or
//!   the bit under way from its start to the tick the timing failed at, where it settles; the
//!   `end` after it shares its provenance. A batch of data settles at its last byte or at the
//!   end that flushed it, and in an unchunked message at the byte that showed its last byte to
//!   be data. `end` with `aborted` is placed, with no span, where the samples stopped.
//! - **Gaps and the end of the samples.** A packet under way ends `aborted`, after the data
//!   bytes already known; the bits of a field not read to its end are dropped, and so are the
//!   bytes of an unchunked message that might be its CRC. The gap or the end is passed on.
//! - **Progress** is passed on, held back to the start of whatever is under way: the
//!   preamble, the field being read, or the data not yet reported.
//!
//! - **Sampling.** The input requires [`MIN_TICKS_PER_BIT`] samples per bit at `bit_rate`
//!   (`rateHz`), so the graph refuses a slower source. Should a decoder start on one all the
//!   same, it reports no packets and only passes gaps, progress and the end on.
//!
//! Limits: one CC line; no glitch filter, so a comparator that rings at the edges shows as
//! `timing` errors; a preamble whose first transition was not captured cannot be aligned; the
//! meaning of objects other than power and request data objects, and of extended data, is the
//! next layer's business.

use std::convert::Infallible;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use uiharu_core::{
    Change, ChannelMask, InputRef, InputSpec, Io, LogicBlock, MAX_CHANNELS, Node, NodeFuture,
    NodeId, NodeSpec, Outbox, OutputRef, OutputSpec, Props, Provenance, Requires, Sample,
    SignalCursor, Tick, TickRange,
};

use super::crc::Crc32;
use super::events::{
    DATA_BATCH, DataMessage, EndReason, ErrorReason, ExtendedMessageHeader, MessageHeader,
    PacketId, PowerObject, RequestObject, UsbPdEvent,
};
use super::symbols::{self, KCode, Start, Symbol};

/// The fewest samples per bit the decoder reads, at the configured bit rate: a slower source is
/// refused when it is connected.
pub const MIN_TICKS_PER_BIT: u64 = 6;

/// The largest tolerance, in percent: beyond it the middle of a bit and its end can no longer
/// be told apart.
pub const MAX_TOLERANCE: u8 = 25;

/// The bits of the preamble.
const PREAMBLE_BITS: u64 = 64;

/// The bytes after an unchunked extended message's data: its CRC.
const CRC_BYTES: usize = 4;

/// Which channel carries the CC line and how fast it signals.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct UsbPdConfig {
    /// The channel of the CC line.
    pub cc: u8,
    /// The nominal bit rate in bits per second: 300 000 for USB PD.
    pub bit_rate: u32,
    /// How far, in percent, the bit rate and each bit may be off: 10 for USB PD, at most
    /// [`MAX_TOLERANCE`].
    pub tolerance: u8,
}

impl Default for UsbPdConfig {
    fn default() -> Self {
        Self {
            cc: 0,
            bit_rate: 300_000,
            tolerance: 10,
        }
    }
}

impl UsbPdConfig {
    /// Checks the configuration.
    pub fn validate(&self) -> Result<(), UsbPdConfigError> {
        if self.cc >= MAX_CHANNELS {
            return Err(UsbPdConfigError::Channel(self.cc));
        }
        if self.bit_rate == 0 {
            return Err(UsbPdConfigError::BitRate);
        }
        if self.tolerance > MAX_TOLERANCE {
            return Err(UsbPdConfigError::Tolerance(self.tolerance));
        }
        Ok(())
    }

    /// The slowest sample rate the decoder reads: [`MIN_TICKS_PER_BIT`] samples per bit.
    pub fn min_rate_hz(&self) -> u64 {
        MIN_TICKS_PER_BIT * u64::from(self.bit_rate)
    }

    /// What the decoder's output port declares: `bitRate` and `tolerance`.
    pub fn props(&self) -> Props {
        Props::new()
            .with("bitRate", i64::from(self.bit_rate))
            .with("tolerance", i64::from(self.tolerance))
    }

    /// What the decoder's input requires of the source: the channel it reads and
    /// [`MIN_TICKS_PER_BIT`] samples per bit.
    pub fn requires(&self) -> Requires {
        Requires::new()
            .at_least("channels", i64::from(self.cc) + 1)
            .at_least(
                "rateHz",
                i64::try_from(self.min_rate_hz()).unwrap_or(i64::MAX),
            )
    }
}

/// Why a configuration is rejected.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum UsbPdConfigError {
    /// The CC channel is out of range.
    #[error("channel {0} is out of range; channels are 0 to {last}", last = MAX_CHANNELS - 1)]
    Channel(u8),
    /// The bit rate is zero.
    #[error("the bit rate must be at least 1 bit/s")]
    BitRate,
    /// The tolerance is above [`MAX_TOLERANCE`].
    #[error("a tolerance of {0} %; it must be 0 to {MAX_TOLERANCE} %")]
    Tolerance(u8),
}

/// The USB PD decoder node: logic samples in, `usbpd.events` out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UsbPd {
    config: UsbPdConfig,
}

impl UsbPd {
    /// A decoder for `config`.
    pub fn new(config: UsbPdConfig) -> Result<Self, UsbPdConfigError> {
        config.validate()?;
        Ok(Self { config })
    }

    /// The configuration.
    pub fn config(&self) -> &UsbPdConfig {
        &self.config
    }

    /// The logic input of the decoder added as `node`.
    pub fn input(node: NodeId) -> InputRef<Arc<LogicBlock>> {
        InputRef::of_node(node, 0)
    }

    /// The event output of the decoder added as `node`.
    pub fn events(node: NodeId) -> OutputRef<UsbPdEvent> {
        OutputRef::of_node(node, 0)
    }
}

impl Node for UsbPd {
    fn spec(&self) -> NodeSpec {
        NodeSpec {
            name: "usb-pd".into(),
            inputs: vec![InputSpec::new::<Arc<LogicBlock>>(
                "logic",
                self.config.requires(),
            )],
            outputs: vec![OutputSpec::new::<UsbPdEvent>("events", self.config.props())],
        }
    }

    fn start(self: Box<Self>, mut io: Io) -> NodeFuture {
        let signal = io.signal(0);
        let out = io.output(0);
        match io.time_base() {
            Some(base) if base.rate_hz >= self.config.min_rate_hz() => {
                Box::pin(Decoder::new(signal, out, &self.config, base.rate_hz).run())
            }
            // The graph refuses a source this slow; without enough samples per bit nothing is
            // decoded.
            _ => Box::pin(pass_through(signal, out)),
        }
    }
}

/// Converts a tick count computed in 128 bits, saturating.
fn ticks(value: u128) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

/// The timing of the configuration on the source's clock.
#[derive(Clone, Copy, Debug)]
struct Timing {
    /// Ticks per second.
    rate: u64,
    /// The nominal bits per second.
    bit_rate: u64,
    /// The tolerance in percent.
    tolerance: u64,
}

impl Timing {
    /// Ticks of one level after which the line is idle: more than any bit the decoder accepts,
    /// the slowest period the tolerance allows lengthened by the tolerance, plus two ticks.
    fn idle(&self) -> u64 {
        let longest = u128::from(self.rate) * u128::from(100 + self.tolerance);
        let per = u128::from(self.bit_rate) * u128::from(100 - self.tolerance);
        ticks(longest.div_ceil(per.max(1))).saturating_add(2)
    }

    /// The fewest ticks the first bit of a preamble may last: the fastest period the tolerance
    /// allows shortened by the tolerance, less a tick.
    fn shortest_first_bit(&self) -> u64 {
        let shortest = u128::from(self.rate) * u128::from(100 - self.tolerance);
        let per = u128::from(self.bit_rate) * u128::from(100 + self.tolerance);
        ticks(shortest / per.max(1)).saturating_sub(1).max(1)
    }

    /// Whether the 64 bits of a preamble lasting `span` ticks, give or take one, make a bit
    /// rate within the tolerance.
    fn rate_ok(&self, span: u64) -> bool {
        let wanted = u128::from(self.rate) * 100 * u128::from(PREAMBLE_BITS);
        let fast_enough =
            (u128::from(span) + 1) * u128::from(self.bit_rate) * u128::from(100 + self.tolerance)
                >= wanted;
        let slow_enough = u128::from(span.saturating_sub(1))
            * u128::from(self.bit_rate)
            * u128::from(100 - self.tolerance)
            <= wanted;
        fast_enough && slow_enough
    }
}

/// A bit period measured on the line: `ticks` over `bits`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Period {
    ticks: u64,
    bits: u64,
}

impl Period {
    /// Whether a transition `offset` ticks after a bit's first transition comes before three
    /// quarters of the period: the bit's middle, not its end.
    fn is_middle(&self, offset: u64) -> bool {
        4 * u128::from(self.bits) * u128::from(offset) < 3 * u128::from(self.ticks)
    }

    /// Whether a bit lasting `length` ticks is the period, within the tolerance of it plus one
    /// tick for sampling and the period's own uncertainty: each end of the measurement is off by
    /// less than a tick.
    fn whole_ok(&self, length: u64, tolerance: u64) -> bool {
        let (span, bits, tol) = (
            u128::from(self.ticks),
            u128::from(self.bits),
            u128::from(tolerance),
        );
        (100 * bits * u128::from(length)).abs_diff(100 * span)
            <= tol * span + 100 * bits + 100 + tol
    }

    /// Whether a middle transition `offset` ticks after a bit's first transition is half the
    /// period after it, within the tolerance of the period plus the same allowances.
    fn half_ok(&self, offset: u64, tolerance: u64) -> bool {
        let (span, bits, tol) = (
            u128::from(self.ticks),
            u128::from(self.bits),
            u128::from(tolerance),
        );
        (200 * bits * u128::from(offset)).abs_diff(100 * span)
            <= 2 * tol * span + 200 * bits + 2 * tol + 100
    }

    /// The longest bit [`Period::whole_ok`] accepts.
    fn latest(&self, tolerance: u64) -> u64 {
        let (span, bits, tol) = (
            u128::from(self.ticks),
            u128::from(self.bits),
            u128::from(tolerance),
        );
        ticks((100 * span + tol * span + 100 * bits + 100 + tol) / (100 * bits).max(1))
    }
}

/// Why reading stopped: samples are missing from here on, or they have ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stop {
    Gap(TickRange),
    End(Tick),
}

/// What ended a packet early, and where.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Fault {
    reason: ErrorReason,
    at: Provenance,
}

/// Why the body of a packet stopped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Halt {
    /// The packet went wrong on the line.
    Fault(Fault),
    /// The samples stopped while it was under way.
    Stop(Stop),
}

impl From<Stop> for Halt {
    fn from(stop: Stop) -> Self {
        Halt::Stop(stop)
    }
}

/// A fault as a halt.
fn fault(reason: ErrorReason, at: Provenance) -> Halt {
    Halt::Fault(Fault { reason, at })
}

/// What a wait for a transition found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Waited {
    /// A transition at this tick.
    Edge(Tick),
    /// None before the deadline.
    Deadline,
}

/// A bit as it was read: its value and the transition that ended it, or the tick its timing
/// failed at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BitRead {
    Zero(Tick),
    One(Tick),
    Timing(Tick),
}

/// A five-bit symbol as it was read, its first bit in bit 0, and the transition that ended it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SymbolRead {
    code: u8,
    end: Tick,
}

/// A field of data symbols: its value, the low nibble first, where it came from and where it
/// ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct FieldRead {
    value: u32,
    at: Provenance,
    end: Tick,
}

/// Data bytes waiting to go out, at most [`DATA_BATCH`].
#[derive(Debug)]
struct Batch {
    offset: u16,
    bytes: Vec<u8>,
    at: Option<Provenance>,
}

/// The packet under way.
#[derive(Debug)]
struct Open {
    pkt: PacketId,
    /// The preamble read so far, while `begin` has not been sent.
    preamble: Option<TickRange>,
    batch: Batch,
}

struct Decoder {
    signal: SignalCursor,
    out: Outbox<UsbPdEvent>,
    mask: ChannelMask,
    timing: Timing,
    tolerance: u64,
    /// Ticks of one level that make the line idle.
    idle: u64,
    /// The fewest ticks of a preamble's first bit.
    first_bit: u64,
    next_pkt: u64,
    open: Option<Open>,
    /// The earliest tick an event not yet sent may span from: progress goes no further.
    hold: Option<Tick>,
    /// The last progress sent.
    progressed: Option<Tick>,
    /// The last transition seen, or the first sample of the stretch being read.
    last_edge: Tick,
    /// A gap or end the cursor reported along with a deadline's passing, for the next wait.
    stopped: Option<Stop>,
}

impl Decoder {
    fn new(signal: SignalCursor, out: Outbox<UsbPdEvent>, config: &UsbPdConfig, rate: u64) -> Self {
        let timing = Timing {
            rate,
            bit_rate: u64::from(config.bit_rate),
            tolerance: u64::from(config.tolerance),
        };
        Self {
            signal,
            out,
            mask: ChannelMask::one(config.cc),
            timing,
            tolerance: timing.tolerance,
            idle: timing.idle(),
            first_bit: timing.shortest_first_bit(),
            next_pkt: 1,
            open: None,
            hold: None,
            progressed: None,
            last_edge: Tick::ZERO,
            stopped: None,
        }
    }

    async fn run(mut self) {
        // Each stretch of samples starts where nothing is known of the line: at the first
        // sample and after every gap.
        loop {
            let start = loop {
                match self.signal.sample().await {
                    Sample::At { tick, .. } => break tick,
                    Sample::Gap(range) => self.out.gap(range).await,
                    Sample::Progress(tick) => self.progress(tick).await,
                    Sample::End(tick) => {
                        self.out.end(tick).await;
                        return;
                    }
                }
            };
            let Err(stop) = self.stretch(start).await;
            self.hold = None;
            match stop {
                Stop::Gap(range) => {
                    self.abort(range.start).await;
                    self.out.gap(range).await;
                }
                Stop::End(tick) => {
                    self.abort(tick).await;
                    self.out.end(tick).await;
                    return;
                }
            }
        }
    }

    /// Follows the line from the first sample of a stretch until the samples stop: waits for
    /// it to be idle, then reads the packet the next transition starts, and again.
    async fn stretch(&mut self, start: Tick) -> Result<Infallible, Stop> {
        self.last_edge = start;
        loop {
            loop {
                let until = self.last_edge.offset(self.idle);
                if self.wait(Some(until)).await? == Waited::Deadline {
                    break;
                }
            }
            let first = loop {
                if let Waited::Edge(tick) = self.wait(None).await? {
                    break tick;
                }
            };
            self.hold = Some(first);
            let read = match self.packet(first).await {
                Ok(()) => Ok(()),
                Err(Halt::Fault(fault)) => {
                    self.fail(fault).await;
                    Ok(())
                }
                Err(Halt::Stop(stop)) => Err(stop),
            };
            self.hold = None;
            read?;
        }
    }

    /// The packet whose preamble may start at the transition `first`.
    async fn packet(&mut self, first: Tick) -> Result<(), Halt> {
        // Bit 0, a 0: the next transition ends it.
        let one = match self.wait(Some(first.offset(self.idle))).await? {
            Waited::Edge(tick) if first.until(tick) >= self.first_bit => tick,
            _ => return Ok(()),
        };
        // Bit 1, a 1, timed by bit 0.
        let bit0 = Period {
            ticks: first.until(one),
            bits: 1,
        };
        let mut at = match self.bit(one, bit0).await? {
            BitRead::One(end) => end,
            _ => return Ok(()),
        };
        let pkt = PacketId(self.next_pkt);
        self.next_pkt += 1;
        self.open = Some(Open {
            pkt,
            preamble: Some(TickRange::new(first, at)),
            batch: Batch {
                offset: 0,
                bytes: Vec::with_capacity(DATA_BATCH),
                at: None,
            },
        });
        // The rest of the preamble, each bit timed by the bits before it.
        for bits in 2..PREAMBLE_BITS {
            let period = Period {
                ticks: first.until(at),
                bits,
            };
            match self.bit(at, period).await? {
                BitRead::Zero(end) | BitRead::One(end) => at = end,
                BitRead::Timing(tick) => {
                    let span = TickRange::new(at, tick);
                    return Err(fault(ErrorReason::Timing, Provenance::over(span, tick)));
                }
            }
            if let Some(open) = self.open.as_mut() {
                open.preamble = Some(TickRange::new(first, at));
            }
        }
        self.announce().await;
        if !self.timing.rate_ok(first.until(at)) {
            let preamble = Provenance::over(TickRange::new(first, at), at);
            return Err(fault(ErrorReason::Timing, preamble));
        }
        let period = Period {
            ticks: first.until(at),
            bits: PREAMBLE_BITS,
        };
        self.fields(pkt, at, period).await
    }

    /// The packet after its preamble: the ordered set, then the message, if it is one.
    async fn fields(&mut self, pkt: PacketId, start: Tick, period: Period) -> Result<(), Halt> {
        self.reading(start);
        let mut codes = [0u8; 4];
        let mut at = start;
        for code in &mut codes {
            let symbol = self.symbol(at, period).await?;
            *code = symbol.code;
            at = symbol.end;
        }
        let set_at = Provenance::over(TickRange::new(start, at), at);
        let set = match symbols::recognise(codes) {
            None => return Err(fault(ErrorReason::OrderedSet, set_at)),
            Some((Start::HardReset, corrupt)) => {
                self.out
                    .emit(set_at, UsbPdEvent::HardReset { pkt, corrupt })
                    .await;
                self.finish(EndReason::HardReset, Provenance::at(at)).await;
                return Ok(());
            }
            Some((Start::CableReset, corrupt)) => {
                self.out
                    .emit(set_at, UsbPdEvent::CableReset { pkt, corrupt })
                    .await;
                self.finish(EndReason::CableReset, Provenance::at(at)).await;
                return Ok(());
            }
            Some((Start::Message(set), corrupt)) => {
                self.out
                    .emit(set_at, UsbPdEvent::Sop { pkt, set, corrupt })
                    .await;
                set
            }
        };
        let mut crc = Crc32::new();
        self.reading(at);
        let header = self.data(at, 4, period, &mut crc).await?;
        at = header.end;
        let message = MessageHeader::from_raw(header.value as u16, set);
        self.out
            .emit(
                header.at,
                UsbPdEvent::Header {
                    pkt,
                    header: message,
                },
            )
            .await;
        if message.extended {
            self.reading(at);
            let field = self.data(at, 4, period, &mut crc).await?;
            at = field.end;
            let extended = ExtendedMessageHeader::from_raw(field.value as u16);
            self.out
                .emit(
                    field.at,
                    UsbPdEvent::ExtendedHeader {
                        pkt,
                        header: extended,
                    },
                )
                .await;
            if !extended.chunked {
                return self.unchunked(pkt, at, period, crc).await;
            }
            // The chunk: its data and padding fill the objects after the extended header.
            let bytes = (u16::from(message.objects) * 4).saturating_sub(2);
            for offset in 0..bytes {
                self.reading(at);
                let byte = self.data(at, 2, period, &mut crc).await?;
                at = byte.end;
                self.add(offset, byte.value as u8, byte.at).await;
            }
            self.flush(at).await;
        } else {
            for index in 1..=message.objects {
                self.reading(at);
                let object = self.data(at, 8, period, &mut crc).await?;
                at = object.end;
                let raw = object.value;
                self.out
                    .emit(object.at, UsbPdEvent::Object { pkt, index, raw })
                    .await;
                let companion = match message.data {
                    Some(DataMessage::SourceCapabilities | DataMessage::SinkCapabilities) => {
                        Some(UsbPdEvent::Pdo {
                            pkt,
                            index,
                            object: PowerObject::from_raw(raw),
                        })
                    }
                    Some(DataMessage::Request) if index == 1 => Some(UsbPdEvent::Rdo {
                        pkt,
                        request: RequestObject::from_raw(raw),
                    }),
                    _ => None,
                };
                if let Some(companion) = companion {
                    self.out.emit(object.at, companion).await;
                }
            }
        }
        self.reading(at);
        let field = self.data(at, 8, period, &mut crc).await?;
        at = field.end;
        let ok = crc.is_residual();
        self.out
            .emit(
                field.at,
                UsbPdEvent::Crc {
                    pkt,
                    value: field.value,
                    ok,
                },
            )
            .await;
        self.reading(at);
        let symbol = self.symbol(at, period).await?;
        let eop = Provenance::over(TickRange::new(at, symbol.end), symbol.end);
        match symbols::decode(symbol.code) {
            Symbol::K(KCode::Eop) => {
                self.finish(EndReason::Eop, eop).await;
                Ok(())
            }
            Symbol::Invalid => Err(fault(ErrorReason::Symbol, eop)),
            Symbol::Data(_) | Symbol::K(_) => Err(fault(ErrorReason::MissingEop, eop)),
        }
    }

    /// The rest of an unchunked extended message, from the byte after its extended header to
    /// its EOP. The last four bytes before the EOP are the CRC, so each byte is reported once
    /// four more have followed it.
    async fn unchunked(
        &mut self,
        pkt: PacketId,
        start: Tick,
        period: Period,
        mut crc: Crc32,
    ) -> Result<(), Halt> {
        let mut held = [(0u8, Provenance::at(start)); CRC_BYTES];
        let mut count = 0;
        let mut offset = 0u16;
        let mut at = start;
        loop {
            let oldest = if count > 0 { held[0].1.span.start } else { at };
            self.reading(oldest);
            let low = self.symbol(at, period).await?;
            let low_at = Provenance::over(TickRange::new(at, low.end), low.end);
            let low = match symbols::decode(low.code) {
                Symbol::Data(nibble) => (nibble, low.end),
                Symbol::K(KCode::Eop) if count == CRC_BYTES => {
                    self.flush(low.end).await;
                    let value = u32::from_le_bytes(held.map(|(byte, _)| byte));
                    let span = held[0].1.span.join(held[CRC_BYTES - 1].1.span);
                    let ok = crc.is_residual();
                    self.out
                        .emit(
                            Provenance::over(span, low.end),
                            UsbPdEvent::Crc { pkt, value, ok },
                        )
                        .await;
                    self.finish(EndReason::Eop, low_at).await;
                    return Ok(());
                }
                Symbol::K(KCode::Eop) => return Err(fault(ErrorReason::EarlyEop, low_at)),
                Symbol::K(_) | Symbol::Invalid => {
                    return Err(fault(ErrorReason::Symbol, low_at));
                }
            };
            crc.push(u32::from(low.0), 4);
            let high = self.symbol(low.1, period).await?;
            let high_at = Provenance::over(TickRange::new(low.1, high.end), high.end);
            let nibble = match symbols::decode(high.code) {
                Symbol::Data(nibble) => nibble,
                Symbol::K(KCode::Eop) => return Err(fault(ErrorReason::EarlyEop, high_at)),
                Symbol::K(_) | Symbol::Invalid => {
                    return Err(fault(ErrorReason::Symbol, high_at));
                }
            };
            crc.push(u32::from(nibble), 4);
            let byte = (
                low.0 | nibble << 4,
                Provenance::over(TickRange::new(at, high.end), high.end),
            );
            at = high.end;
            if count < CRC_BYTES {
                held[count] = byte;
                count += 1;
                continue;
            }
            let (released, released_at) = held[0];
            held = [held[1], held[2], held[3], byte];
            // Four bytes follow it: it is data, known to be at the last of them.
            let decided = Provenance::over(released_at.span, at);
            self.add(offset, released, decided).await;
            offset = offset.saturating_add(1);
        }
    }

    /// Reads the bit that begins with the transition at `start`.
    async fn bit(&mut self, start: Tick, period: Period) -> Result<BitRead, Stop> {
        let tolerance = self.tolerance;
        let deadline = start.offset(period.latest(tolerance).saturating_add(1).min(self.idle));
        let first = match self.wait(Some(deadline)).await? {
            Waited::Edge(tick) => tick,
            Waited::Deadline => return Ok(BitRead::Timing(deadline)),
        };
        let offset = start.until(first);
        if !period.is_middle(offset) {
            return Ok(if period.whole_ok(offset, tolerance) {
                BitRead::Zero(first)
            } else {
                BitRead::Timing(first)
            });
        }
        if !period.half_ok(offset, tolerance) {
            return Ok(BitRead::Timing(first));
        }
        Ok(match self.wait(Some(deadline)).await? {
            Waited::Edge(end) => {
                let length = start.until(end);
                if !period.is_middle(length) && period.whole_ok(length, tolerance) {
                    BitRead::One(end)
                } else {
                    BitRead::Timing(end)
                }
            }
            Waited::Deadline => BitRead::Timing(deadline),
        })
    }

    /// Reads the five bits of a symbol, the first in bit 0, from the transition at `start`.
    async fn symbol(&mut self, start: Tick, period: Period) -> Result<SymbolRead, Halt> {
        let mut code = 0u8;
        let mut at = start;
        for bit in 0..5 {
            match self.bit(at, period).await? {
                BitRead::Zero(end) => at = end,
                BitRead::One(end) => {
                    code |= 1 << bit;
                    at = end;
                }
                BitRead::Timing(tick) => {
                    let span = TickRange::new(at, tick);
                    return Err(fault(ErrorReason::Timing, Provenance::over(span, tick)));
                }
            }
        }
        Ok(SymbolRead { code, end: at })
    }

    /// Reads a field of `count` data symbols from the transition at `start`, the low nibble
    /// first, running them through `crc`.
    async fn data(
        &mut self,
        start: Tick,
        count: u32,
        period: Period,
        crc: &mut Crc32,
    ) -> Result<FieldRead, Halt> {
        let mut value = 0u32;
        let mut at = start;
        for index in 0..count {
            let symbol = self.symbol(at, period).await?;
            let span = Provenance::over(TickRange::new(at, symbol.end), symbol.end);
            match symbols::decode(symbol.code) {
                Symbol::Data(nibble) => {
                    value |= u32::from(nibble) << (4 * index);
                    crc.push(u32::from(nibble), 4);
                    at = symbol.end;
                }
                Symbol::K(KCode::Eop) => return Err(fault(ErrorReason::EarlyEop, span)),
                Symbol::K(_) | Symbol::Invalid => return Err(fault(ErrorReason::Symbol, span)),
            }
        }
        Ok(FieldRead {
            value,
            at: Provenance::over(TickRange::new(start, at), at),
            end: at,
        })
    }

    /// Notes that what is read next starts at `at`: progress is held back to it, or to the
    /// data not yet sent.
    fn reading(&mut self, at: Tick) {
        let batch = self
            .open
            .as_ref()
            .and_then(|open| open.batch.at)
            .map(|batch| batch.span.start);
        self.hold = Some(batch.map_or(at, |start| start.min(at)));
    }

    /// Adds a data byte to the batch, which goes out once it is full.
    async fn add(&mut self, offset: u16, byte: u8, at: Provenance) {
        let Some(open) = self.open.as_mut() else {
            return;
        };
        let batch = &mut open.batch;
        match batch.at {
            Some(have) if !batch.bytes.is_empty() => batch.at = Some(have.join(at)),
            _ => {
                batch.offset = offset;
                batch.at = Some(at);
            }
        }
        batch.bytes.push(byte);
        if batch.bytes.len() >= DATA_BATCH {
            self.flush(at.settled).await;
        }
    }

    /// Sends the batch, if any, settled where it was `decided`: by its last byte, or by the end
    /// that cut the data short.
    async fn flush(&mut self, decided: Tick) {
        let Some(open) = self.open.as_mut() else {
            return;
        };
        if open.batch.bytes.is_empty() {
            return;
        }
        let pkt = open.pkt;
        let offset = open.batch.offset;
        let bytes = std::mem::replace(&mut open.batch.bytes, Vec::with_capacity(DATA_BATCH));
        let mut at = open.batch.at.take().unwrap_or(Provenance::at(decided));
        at.settled = at.settled.max(decided);
        self.out
            .emit(at, UsbPdEvent::Data { pkt, offset, bytes })
            .await;
    }

    /// Sends `begin`, if it has not gone out: the preamble as far as it was read.
    async fn announce(&mut self) {
        let Some(open) = self.open.as_mut() else {
            return;
        };
        let Some(preamble) = open.preamble.take() else {
            return;
        };
        let pkt = open.pkt;
        self.out
            .emit(
                Provenance::over(preamble, preamble.end),
                UsbPdEvent::Begin { pkt },
            )
            .await;
    }

    /// Ends the open packet for `reason`, after the data not yet sent.
    async fn finish(&mut self, reason: EndReason, at: Provenance) {
        self.flush(at.settled).await;
        if let Some(open) = self.open.take() {
            self.out
                .emit(
                    at,
                    UsbPdEvent::End {
                        pkt: open.pkt,
                        reason,
                    },
                )
                .await;
        }
    }

    /// Ends the open packet with an error: `begin` if it has not gone out, the data not yet
    /// sent, the error and the end.
    async fn fail(&mut self, fault: Fault) {
        self.announce().await;
        self.flush(fault.at.settled).await;
        if let Some(open) = self.open.take() {
            let pkt = open.pkt;
            self.out
                .emit(
                    fault.at,
                    UsbPdEvent::Error {
                        pkt,
                        reason: fault.reason,
                    },
                )
                .await;
            self.out
                .emit(
                    fault.at,
                    UsbPdEvent::End {
                        pkt,
                        reason: EndReason::Error,
                    },
                )
                .await;
        }
    }

    /// Ends the open packet, if any, where the samples stopped.
    async fn abort(&mut self, tick: Tick) {
        self.announce().await;
        self.flush(tick).await;
        if let Some(open) = self.open.take() {
            self.out
                .emit(
                    Provenance::at(tick),
                    UsbPdEvent::End {
                        pkt: open.pkt,
                        reason: EndReason::Aborted,
                    },
                )
                .await;
        }
    }

    /// Passes progress on, held back to the start of whatever is under way.
    async fn progress(&mut self, tick: Tick) {
        let tick = self.hold.map_or(tick, |hold| hold.min(tick));
        if self.progressed.is_some_and(|done| done >= tick) {
            return;
        }
        self.progressed = Some(tick);
        self.out.progress(tick).await;
    }

    /// Waits for a transition before `until`, or for the next one, passing progress on.
    ///
    /// Samples that reach the deadline with no transition are the deadline passing, however
    /// the cursor learns it: by `Quiet`, by progress at or past it, or by a gap or the end of
    /// the samples at or past it — which is then held back for the next wait, so that what ends
    /// at the deadline is still read.
    async fn wait(&mut self, until: Option<Tick>) -> Result<Waited, Stop> {
        if let Some(stop) = self.stopped.take() {
            return Err(stop);
        }
        let reached = |tick: Tick| until.is_some_and(|until| tick >= until);
        loop {
            let change = match until {
                Some(until) => self.signal.next_change_before(self.mask, until).await,
                None => self.signal.next_change(self.mask).await,
            };
            match change {
                Change::At { tick, .. } => {
                    self.last_edge = tick;
                    return Ok(Waited::Edge(tick));
                }
                Change::Quiet(_) => return Ok(Waited::Deadline),
                Change::Progress(tick) if reached(tick) => {
                    if let Some(until) = until {
                        self.signal.seek(until);
                    }
                    return Ok(Waited::Deadline);
                }
                Change::Progress(tick) => self.progress(tick).await,
                Change::Gap(range) if reached(range.start) => {
                    self.stopped = Some(Stop::Gap(range));
                    return Ok(Waited::Deadline);
                }
                Change::End(tick) if reached(tick) => {
                    self.stopped = Some(Stop::End(tick));
                    return Ok(Waited::Deadline);
                }
                Change::Gap(range) => return Err(Stop::Gap(range)),
                Change::End(tick) => return Err(Stop::End(tick)),
            }
        }
    }
}

/// Passes gaps, progress and the end on without decoding.
async fn pass_through(mut signal: SignalCursor, mut out: Outbox<UsbPdEvent>) {
    loop {
        match signal.next_change(ChannelMask::EMPTY).await {
            Change::Gap(range) => out.gap(range).await,
            Change::Progress(tick) => out.progress(tick).await,
            Change::End(tick) => {
                out.end(tick).await;
                return;
            }
            Change::At { .. } | Change::Quiet(_) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configurations_are_checked() {
        assert_eq!(UsbPdConfig::default().validate(), Ok(()));
        let bad = |config: UsbPdConfig| config.validate().expect_err("rejected");
        assert_eq!(
            bad(UsbPdConfig {
                cc: 64,
                ..UsbPdConfig::default()
            }),
            UsbPdConfigError::Channel(64)
        );
        assert_eq!(
            bad(UsbPdConfig {
                bit_rate: 0,
                ..UsbPdConfig::default()
            }),
            UsbPdConfigError::BitRate
        );
        assert_eq!(
            bad(UsbPdConfig {
                tolerance: 26,
                ..UsbPdConfig::default()
            }),
            UsbPdConfigError::Tolerance(26)
        );
        for tolerance in [0, 25] {
            assert!(
                UsbPd::new(UsbPdConfig {
                    tolerance,
                    ..UsbPdConfig::default()
                })
                .is_ok()
            );
        }
    }

    #[test]
    fn configurations_read_with_defaults() {
        let config: UsbPdConfig = serde_json::from_str(r#"{"cc":3}"#).expect("parses");
        assert_eq!(
            config,
            UsbPdConfig {
                cc: 3,
                ..UsbPdConfig::default()
            }
        );
        assert!(serde_json::from_str::<UsbPdConfig>(r#"{"line":3}"#).is_err());
    }

    #[test]
    fn ports_declare_the_rate_and_require_six_samples_per_bit() {
        let config = UsbPdConfig {
            cc: 2,
            ..UsbPdConfig::default()
        };
        let props = config.props();
        assert_eq!(props.get("bitRate"), Some(&300_000i64.into()));
        assert_eq!(props.get("tolerance"), Some(&10i64.into()));
        let decoder = UsbPd::new(config).expect("valid");
        let spec = decoder.spec();
        assert_eq!(spec.name, "usb-pd");
        let source = |channels: i64, rate: i64| {
            let props = Props::new().with("channels", channels).with("rateHz", rate);
            spec.inputs[0].requires.unmet(&props).map(|(key, _)| key)
        };
        assert_eq!(source(3, 1_800_000), None);
        assert_eq!(source(2, 1_800_000), Some("channels"));
        assert_eq!(source(3, 1_799_999), Some("rateHz"));
    }

    #[test]
    fn idle_and_the_first_bit_bound_every_bit_accepted() {
        // 12 MHz at 300 kbit/s: 40 ticks per bit.
        let timing = Timing {
            rate: 12_000_000,
            bit_rate: 300_000,
            tolerance: 10,
        };
        assert_eq!(timing.idle(), 51);
        assert_eq!(timing.shortest_first_bit(), 31);
        let period = Period {
            ticks: 2560,
            bits: 64,
        };
        assert_eq!(period.latest(10), 45);
        assert!(period.whole_ok(35, 10) && !period.whole_ok(34, 10));
        assert!(period.whole_ok(45, 10) && !period.whole_ok(46, 10));
        assert!(period.half_ok(15, 10) && !period.half_ok(14, 10));
        assert!(period.half_ok(25, 10) && !period.half_ok(26, 10));
        assert!(period.is_middle(29) && !period.is_middle(30));
        // The slowest period the tolerance allows, recovered with a tick to spare, still ends
        // every bit it accepts before the line counts as idle.
        for tolerance in 0..=u64::from(MAX_TOLERANCE) {
            for per_bit in [6u64, 7, 8, 13, 40, 333] {
                let timing = Timing {
                    rate: per_bit * 300_000,
                    bit_rate: 300_000,
                    tolerance,
                };
                let slowest = (0..per_bit * 128)
                    .rev()
                    .find(|span| timing.rate_ok(*span))
                    .expect("some preamble is slow enough");
                let period = Period {
                    ticks: slowest,
                    bits: 64,
                };
                assert!(
                    period.latest(tolerance) < timing.idle(),
                    "{per_bit} ticks per bit at {tolerance} %"
                );
            }
        }
    }

    #[test]
    fn the_rate_check_allows_one_tick() {
        // 2.4 MHz: 8 ticks per bit, 512 per preamble; ±10 % is 465.45 to 568.9 ticks.
        let timing = Timing {
            rate: 2_400_000,
            bit_rate: 300_000,
            tolerance: 10,
        };
        assert!(timing.rate_ok(512));
        assert!(timing.rate_ok(465));
        assert!(!timing.rate_ok(464));
        assert!(timing.rate_ok(569));
        assert!(!timing.rate_ok(570));
    }
}
