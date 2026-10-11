// SPDX-License-Identifier: Apache-2.0

//! The USB packet decoder: low- and full-speed packets from the D+ and D− lines, and the bus
//! signalling between them, after the Universal Serial Bus Specification Revision 2.0
//! (usb.org): §7.1 for the signalling, §8.3 and §8.4 for the packet fields and formats.
//!
//! The rules:
//!
//! - the two lines make the bus state: J and K are the differential states (full speed: J is
//!   D+ high and D− low; low speed: the opposite), SE0 is both lines low, SE1 both high, which
//!   is an error; the idle bus is J;
//! - the bit time comes from the speed (12 or 1.5 Mbit/s) and the source's time base, in
//!   integer arithmetic: cell `n` of a run starts `⌊n · rate / bit rate⌋` ticks after the edge
//!   that began the run and is read at its centre, `⌊(2n + 1) · rate / (2 · bit rate)⌋` ticks
//!   after it; the source must sample at four times the bit rate or more (6 MHz at low speed,
//!   48 MHz at full speed), which the input requires as `rateHz`, so a slower source is refused
//!   when the graph is built;
//! - the bit clock is re-aligned to the first edge after each cell centre, on either line, and
//!   a cell's state is the state at its centre: the brief single-ended state of a J/K
//!   transition whose lines change a few ticks apart is no bit; tested are transmitters 5 %
//!   slow to 3 % fast at four samples per bit and 5 % either way at eight (the specification
//!   allows 0.25 % at full speed, 1.5 % at low speed), and lines changing up to half a bit
//!   apart;
//! - NRZI: a cell in the other differential state than the cell before is a 0, in the same
//!   state a 1; after six 1s the next bit is a stuffed 0 and is dropped, and a seventh 1 ends
//!   the packet `bit-stuff-error`;
//! - a packet begins with SYNC: from the idle bus, K, then J and K alternating, ended by a
//!   second K in a row (`KJKJKJKK` in full; its last bit is the first 1 bit stuffing counts);
//!   at least three alternating cells before the final K are required (`KJKK`), which accepts
//!   a SYNC shortened by repeaters and refuses activity at another bit rate;
//! - the PID byte comes first: the type in the low nibble, its complement in the high one; a
//!   check that fails ends the packet `pid-error`;
//! - then, by type: tokens (OUT, IN, SETUP and PING) carry a 7-bit address, a 4-bit endpoint
//!   and a CRC5; SOF an 11-bit frame number and a CRC5; data packets (DATA0, DATA1, DATA2,
//!   MDATA) bytes and a CRC16; SPLIT 24 bits, reported raw; handshakes (ACK, NAK, STALL,
//!   NYET) and the reserved type nothing; PRE, the full-speed preamble, ends after its PID
//!   without an EOP (`preamble`), and the low-speed packet it announces cannot be read at full
//!   speed; type `1100` reads as PRE, as ERR, which shares it, is a high-speed handshake;
//! - fields are read least significant bit first, CRCs included, so `crc5` and `crc16` hold
//!   the bits as they appear on the wire; the CRC5 (x⁵ + x² + 1) and the CRC16
//!   (x¹⁶ + x¹⁵ + x² + 1) are checked by running them over the field and the CRC behind it,
//!   which leaves the residual `0b01100` or `0x800D` when nothing was damaged;
//! - an event spans the bits it was read from, from the start of the first one's cell through
//!   the centre of the last one's, and settles at that centre, where its last bit was read;
//!   what only a later bit decides settles there;
//! - a data packet's last two bytes are its CRC16, which only its end tells: a byte is
//!   reported once two more have followed it, in batches of at most [`DATA_BATCH`] bytes, each
//!   as soon as it is full, then `data-end` with the CRC16 and the payload's length when the
//!   EOP begins;
//! - EOP: SE0 in at least one cell; the packet ends when the SE0 does, whatever state follows;
//!   bits after the last whole byte, or after the fields a packet's type has, are dropped;
//!   lengths are reported, not checked against the speed's limits (8 bytes of payload at low
//!   speed, 1023 at full speed);
//! - between packets: an SE0 of 2.5 µs or more is a reset, reported when it ends with its
//!   length; J for 3 ms from the end of the last activity is a suspend, placed 3 ms after
//!   that and reported once the samples reach it; K for 20 ms or more, ended by an SE0 that is
//!   not a reset and then J, is a resume; at low speed, an SE0 of the idle bus ended by J that
//!   is not a reset is a keep-alive; a state that lasts less than half a bit is a glitch;
//! - the first sample, and the first after a gap, may fall inside a packet, whose bits cannot
//!   be aligned: the decoder waits for the bus to be idle — J for eight bit times, longer than
//!   bit stuffing lets a packet hold a level, or the J that ends an SE0 — before it looks for
//!   a SYNC; a reset, suspend or resume whose beginning was not seen is not reported;
//! - a gap or the end of the samples during a packet ends it `aborted`; the bytes not yet
//!   known to be payload, which may be its CRC16, are not reported.

use std::convert::Infallible;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use uiharu_core::{
    Change, ChannelMask, InputRef, InputSpec, Io, Levels, LogicBlock, MAX_CHANNELS, Node,
    NodeFuture, NodeId, NodeSpec, Outbox, OutputRef, OutputSpec, Props, Provenance, Requires,
    Sample, SignalCursor, Tick, TickRange,
};

use super::events::{DATA_BATCH, EndReason, PacketId, Pid, UsbEvent};

/// Bit times of J after which a bus whose state was not known counts as idle: bit stuffing
/// keeps a packet from holding a level for more than seven.
const IDLE_BITS: u64 = 8;

/// The alternating cells a SYNC needs before its final K.
const SYNC_ZEROS: u32 = 3;

/// A reset is SE0 for 1 / 400 000 s (2.5 µs) or more.
const RESET_PER_SECOND: u64 = 400_000;

/// A resume is K for 1 / 50 s (20 ms) or more.
const RESUME_PER_SECOND: u64 = 50;

/// The signalling speed of the bus.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Speed {
    /// Low speed, 1.5 Mbit/s: J is D− high.
    Low,
    /// Full speed, 12 Mbit/s: J is D+ high.
    #[default]
    Full,
}

impl Speed {
    /// The name the port properties use.
    pub const fn name(self) -> &'static str {
        match self {
            Speed::Low => "low",
            Speed::Full => "full",
        }
    }

    /// Bits per second.
    pub const fn bit_rate_hz(self) -> u64 {
        match self {
            Speed::Low => 1_500_000,
            Speed::Full => 12_000_000,
        }
    }

    /// The slowest sample rate the decoder reads: four samples per bit.
    pub const fn min_rate_hz(self) -> u64 {
        4 * self.bit_rate_hz()
    }
}

/// The state of the bus at one tick.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LineState {
    /// The differential state of the idle bus.
    J,
    /// The other differential state.
    K,
    /// Single-ended zero: both lines low.
    Se0,
    /// Single-ended one: both lines high, an error.
    Se1,
}

impl LineState {
    /// The other differential state; SE0 and SE1 stay as they are.
    pub const fn toggled(self) -> LineState {
        match self {
            LineState::J => LineState::K,
            LineState::K => LineState::J,
            other => other,
        }
    }
}

/// How the bus is wired and how fast it signals.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsbConfig {
    /// The D+ channel.
    pub dp: u8,
    /// The D− channel.
    pub dm: u8,
    /// The signalling speed.
    pub speed: Speed,
}

impl Default for UsbConfig {
    fn default() -> Self {
        Self {
            dp: 0,
            dm: 1,
            speed: Speed::Full,
        }
    }
}

impl UsbConfig {
    /// The state of the bus at `levels`.
    pub fn state(&self, levels: Levels) -> LineState {
        let full = self.speed == Speed::Full;
        match (levels.high(self.dp), levels.high(self.dm)) {
            (false, false) => LineState::Se0,
            (true, true) => LineState::Se1,
            (true, false) if full => LineState::J,
            (false, true) if !full => LineState::J,
            _ => LineState::K,
        }
    }

    /// The levels of D+ and D− in `state`.
    pub fn lines(&self, state: LineState) -> (bool, bool) {
        let full = self.speed == Speed::Full;
        match state {
            LineState::J => (full, !full),
            LineState::K => (!full, full),
            LineState::Se0 => (false, false),
            LineState::Se1 => (true, true),
        }
    }

    /// Checks the configuration.
    pub fn validate(&self) -> Result<(), UsbConfigError> {
        for channel in [self.dp, self.dm] {
            if channel >= MAX_CHANNELS {
                return Err(UsbConfigError::Channel(channel));
            }
        }
        if self.dp == self.dm {
            return Err(UsbConfigError::DuplicateChannel(self.dp));
        }
        Ok(())
    }

    /// What the decoder's output port declares.
    pub fn props(&self) -> Props {
        Props::new().with("speed", self.speed.name()).with(
            "bitRateHz",
            i64::try_from(self.speed.bit_rate_hz()).unwrap_or(i64::MAX),
        )
    }

    /// What the decoder's input requires of the source: the channels it reads and four
    /// samples per bit.
    pub fn requires(&self) -> Requires {
        Requires::new()
            .at_least("channels", i64::from(self.dp.max(self.dm)) + 1)
            .at_least(
                "rateHz",
                i64::try_from(self.speed.min_rate_hz()).unwrap_or(i64::MAX),
            )
    }
}

/// Why a configuration is rejected.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum UsbConfigError {
    /// D+ and D− are on the same channel.
    #[error("channel {0} is used for both D+ and D−")]
    DuplicateChannel(u8),
    /// A channel number is out of range.
    #[error("channel {0} is out of range; channels are 0 to {last}", last = MAX_CHANNELS - 1)]
    Channel(u8),
}

/// The USB packet decoder node: logic samples in, `usb.packets` out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UsbPackets {
    config: UsbConfig,
}

impl UsbPackets {
    /// A decoder for `config`.
    pub fn new(config: UsbConfig) -> Result<Self, UsbConfigError> {
        config.validate()?;
        Ok(Self { config })
    }

    /// The configuration.
    pub fn config(&self) -> &UsbConfig {
        &self.config
    }

    /// The logic input of the decoder added as `node`.
    pub fn input(node: NodeId) -> InputRef<Arc<LogicBlock>> {
        InputRef::of_node(node, 0)
    }

    /// The packet output of the decoder added as `node`.
    pub fn packets(node: NodeId) -> OutputRef<UsbEvent> {
        OutputRef::of_node(node, 0)
    }
}

impl Node for UsbPackets {
    fn spec(&self) -> NodeSpec {
        NodeSpec {
            name: "usb-packets".into(),
            inputs: vec![InputSpec::new::<Arc<LogicBlock>>(
                "logic",
                self.config.requires(),
            )],
            outputs: vec![OutputSpec::new::<UsbEvent>("packets", self.config.props())],
        }
    }

    fn start(self: Box<Self>, mut io: Io) -> NodeFuture {
        // A node with an input always has a time base; the fallback is the slowest rate the
        // input accepts.
        let rate = io
            .time_base()
            .map_or(self.config.speed.min_rate_hz(), |base| base.rate_hz);
        let decoder = Decoder::new(io.signal(0), io.output(0), self.config, rate);
        Box::pin(decoder.run())
    }
}

/// Bit times and the bus's timeouts in ticks, in integer arithmetic.
#[derive(Clone, Copy, Debug)]
struct Timing {
    rate: u64,
    bit_rate: u64,
}

impl Timing {
    /// `⌊numerator · rate / (denominator · bit rate)⌋` ticks: a fraction of bit times.
    fn bits(&self, numerator: u64, denominator: u64) -> u64 {
        let ticks = u128::from(numerator) * u128::from(self.rate)
            / (u128::from(denominator) * u128::from(self.bit_rate)).max(1);
        u64::try_from(ticks).unwrap_or(u64::MAX)
    }

    /// From the edge that began a run of cells to the start of its cell `n`.
    fn start(&self, n: u64) -> u64 {
        self.bits(n, 1)
    }

    /// From the edge that began a run of cells to the centre of its cell `n`.
    fn centre(&self, n: u64) -> u64 {
        self.bits(n.saturating_mul(2).saturating_add(1), 2)
    }

    /// Whether `ticks` last `1 / per_second` seconds or more.
    fn lasts(&self, ticks: u64, per_second: u64) -> bool {
        u128::from(ticks) * u128::from(per_second) >= u128::from(self.rate)
    }

    /// 3 ms in ticks, rounded up.
    fn suspend(&self) -> u64 {
        u64::try_from((u128::from(self.rate) * 3).div_ceil(1000)).unwrap_or(u64::MAX)
    }
}

/// The CRC5 of token and SOF packets, run as a receiver runs it: over the field and the CRC
/// behind it, in the order they are sent.
#[derive(Clone, Copy, Debug)]
struct Crc5(u8);

impl Crc5 {
    const POLY: u8 = 0b0_0101;
    const RESIDUAL: u8 = 0b0_1100;

    fn new() -> Self {
        Self(0b1_1111)
    }

    /// Runs `count` bits of `value`, the least significant first.
    fn push(&mut self, value: u32, count: u32) {
        for i in 0..count {
            let feedback = ((self.0 >> 4) & 1) ^ ((value >> i) & 1) as u8;
            self.0 = (self.0 << 1) & 0b1_1111;
            if feedback != 0 {
                self.0 ^= Self::POLY;
            }
        }
    }

    fn is_residual(&self) -> bool {
        self.0 == Self::RESIDUAL
    }
}

/// The CRC16 of data packets, run the same way.
#[derive(Clone, Copy, Debug)]
struct Crc16(u16);

impl Crc16 {
    const POLY: u16 = 0x8005;
    const RESIDUAL: u16 = 0x800d;

    fn new() -> Self {
        Self(0xffff)
    }

    /// Runs the eight bits of `byte`, the least significant first.
    fn push(&mut self, byte: u8) {
        for i in 0..8 {
            let feedback = ((self.0 >> 15) & 1) ^ u16::from((byte >> i) & 1);
            self.0 <<= 1;
            if feedback != 0 {
                self.0 ^= Self::POLY;
            }
        }
    }

    fn is_residual(&self) -> bool {
        self.0 == Self::RESIDUAL
    }
}

/// A data packet's bytes on their way out: the last two held back, since they are the CRC16
/// if the packet ends after them, and a batch of those known to be payload.
#[derive(Debug)]
struct Payload {
    crc: Crc16,
    /// The bytes held back, the older first.
    held: [(u8, Provenance); 2],
    held_len: usize,
    /// At most [`DATA_BATCH`] bytes known to be payload, not yet sent.
    batch: Vec<u8>,
    batch_offset: u32,
    batch_at: Option<Provenance>,
    /// The bytes known to be payload so far.
    count: u32,
}

impl Payload {
    fn new() -> Self {
        Self {
            crc: Crc16::new(),
            held: [(0, Provenance::at(Tick::ZERO)); 2],
            held_len: 0,
            batch: Vec::with_capacity(DATA_BATCH),
            batch_offset: 0,
            batch_at: None,
            count: 0,
        }
    }

    /// Holds `byte` back; returns the byte it pushes out, which two later bytes have shown to
    /// be payload.
    fn hold(&mut self, byte: u8, at: Provenance) -> Option<(u8, Provenance)> {
        if self.held_len < 2 {
            self.held[self.held_len] = (byte, at);
            self.held_len += 1;
            return None;
        }
        let released = self.held[0];
        self.held = [self.held[1], (byte, at)];
        Some(released)
    }

    /// Adds a byte known to be payload to the batch.
    fn add(&mut self, byte: u8, at: Provenance) {
        if self.batch.is_empty() {
            self.batch_offset = self.count;
            self.batch_at = Some(at);
        } else if let Some(have) = self.batch_at {
            self.batch_at = Some(have.join(at));
        }
        self.batch.push(byte);
        self.count = self.count.saturating_add(1);
    }

    /// The batch, if any, settled where it was `decided`: by the byte that showed its last byte
    /// to be payload, or by the end of the packet.
    fn take(&mut self, decided: Tick) -> Option<(u32, Vec<u8>, Provenance)> {
        if self.batch.is_empty() {
            return None;
        }
        let bytes = std::mem::replace(&mut self.batch, Vec::with_capacity(DATA_BATCH));
        let mut at = self.batch_at.take().unwrap_or(Provenance::at(decided));
        at.settled = at.settled.max(decided);
        Some((self.batch_offset, bytes, at))
    }

    /// The CRC16 field — the two bytes held back when the packet ends — and the ticks of the
    /// bytes held back.
    fn crc_field(&self) -> (Option<u16>, Option<TickRange>) {
        let held = &self.held[..self.held_len];
        let span = held.iter().map(|(_, at)| at.span).reduce(TickRange::join);
        let crc = match held {
            [(low, _), (high, _)] => Some(u16::from_le_bytes([*low, *high])),
            _ => None,
        };
        (crc, span)
    }
}

/// The packet under way.
#[derive(Debug)]
struct Open {
    pkt: PacketId,
    /// The payload of a data packet.
    payload: Option<Payload>,
}

/// What stops the decoder's bus loop.
#[derive(Clone, Copy, Debug)]
enum Stop {
    /// Samples are missing.
    Gap(TickRange),
    /// The samples ended.
    End(Tick),
}

/// What a wait for an edge found.
#[derive(Clone, Copy, Debug)]
enum Waited {
    /// An edge, with the levels after it.
    Edge(Tick, Levels),
    /// No edge before the deadline.
    Deadline,
}

/// A bit cell, read at its centre.
#[derive(Clone, Copy, Debug)]
struct Cell {
    start: Tick,
    centre: Tick,
    state: LineState,
}

impl Cell {
    /// The cell from its start through its centre, settled at the centre.
    fn at(&self) -> Provenance {
        Provenance::over(TickRange::new(self.start, self.centre.next()), self.centre)
    }
}

/// What ended a packet's bits.
#[derive(Clone, Copy, Debug)]
enum Cut {
    /// An SE0 cell: the EOP begins.
    Se0(Cell),
    /// An SE1 cell.
    Se1(Cell),
    /// A seventh 1 in a row.
    Stuff(Cell),
}

/// A bit of a packet, or what ended its bits.
#[derive(Clone, Copy, Debug)]
enum Bit {
    Value(bool, Cell),
    Cut(Cut),
}

/// The bit clock: cells counted from the edge it was last aligned to.
#[derive(Clone, Copy, Debug)]
struct Clock {
    /// The edge that began the run of cells.
    anchor: Tick,
    /// The next cell to read.
    next: u64,
    /// Whether an edge has aligned the clock since the last centre.
    realigned: bool,
}

impl Clock {
    fn at(edge: Tick) -> Self {
        Self {
            anchor: edge,
            next: 0,
            realigned: true,
        }
    }
}

/// What an SE0 follows, which decides what it is once it ends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum After {
    /// The idle bus: a keep-alive at low speed.
    Idle,
    /// K since this tick: a resume if it lasted long enough.
    K(Tick),
    /// A packet's bits: its EOP.
    Packet,
    /// Anything else.
    Other,
}

/// The bus outside packets: the state of the lines, since when, and whether that beginning
/// was seen (not at the first sample or the first after a gap).
#[derive(Clone, Copy, Debug)]
enum Line {
    J {
        since: Tick,
        seen: bool,
    },
    K {
        since: Tick,
        seen: bool,
    },
    Se0 {
        since: Tick,
        seen: bool,
        after: After,
    },
    Se1,
}

struct Decoder {
    signal: SignalCursor,
    out: Outbox<UsbEvent>,
    config: UsbConfig,
    mask: ChannelMask,
    timing: Timing,
    /// 3 ms in ticks.
    suspend: u64,
    /// The levels at the last edge or sample read.
    levels: Levels,
    clock: Clock,
    /// The differential state of the packet's last cell, for NRZI.
    nrzi: LineState,
    /// 1 bits in a row, for bit stuffing.
    ones: u8,
    next_pkt: u64,
    open: Option<Open>,
    /// Whether the bus is known to be between packets.
    idle: bool,
    /// Whether the idle bus was reported suspended.
    suspended: bool,
    /// A gap or end the cursor reported along with a deadline's passing, for the next wait.
    stopped: Option<Stop>,
}

impl Decoder {
    fn new(signal: SignalCursor, out: Outbox<UsbEvent>, config: UsbConfig, rate: u64) -> Self {
        let timing = Timing {
            rate,
            bit_rate: config.speed.bit_rate_hz(),
        };
        Self {
            signal,
            out,
            mask: ChannelMask::one(config.dp).with(config.dm),
            config,
            timing,
            suspend: timing.suspend(),
            levels: Levels(0),
            clock: Clock {
                anchor: Tick::ZERO,
                next: 0,
                realigned: false,
            },
            nrzi: LineState::J,
            ones: 0,
            next_pkt: 1,
            open: None,
            idle: false,
            suspended: false,
            stopped: None,
        }
    }

    async fn run(mut self) {
        // Each round starts where the levels are not known: at the first sample and after
        // every gap.
        loop {
            let start = loop {
                match self.signal.sample().await {
                    Sample::At { tick, levels } => {
                        self.levels = levels;
                        break tick;
                    }
                    Sample::Gap(range) => self.out.gap(range).await,
                    Sample::Progress(tick) => self.out.progress(tick).await,
                    Sample::End(tick) => {
                        self.out.end(tick).await;
                        return;
                    }
                }
            };
            let Err(stop) = self.bus(start).await;
            match stop {
                Stop::Gap(range) => {
                    self.close(EndReason::Aborted, Provenance::at(range.start))
                        .await;
                    self.out.gap(range).await;
                }
                Stop::End(tick) => {
                    self.close(EndReason::Aborted, Provenance::at(tick)).await;
                    self.out.end(tick).await;
                    return;
                }
            }
        }
    }

    /// Follows the bus from the first sample of a stretch of samples until they stop.
    async fn bus(&mut self, start: Tick) -> Result<Infallible, Stop> {
        self.idle = false;
        self.suspended = false;
        let mut line = match self.config.state(self.levels) {
            LineState::J => Line::J {
                since: start,
                seen: false,
            },
            LineState::K => Line::K {
                since: start,
                seen: false,
            },
            LineState::Se0 => Line::Se0 {
                since: start,
                seen: false,
                after: After::Other,
            },
            LineState::Se1 => Line::Se1,
        };
        loop {
            line = match line {
                Line::J { since, seen } => self.in_j(since, seen).await?,
                Line::K { since, seen } => self.in_k(since, seen).await?,
                Line::Se0 { since, seen, after } => self.in_se0(since, seen, after).await?,
                Line::Se1 => self.in_se1().await?,
            };
        }
    }

    /// J: the bus becomes known to be idle, then suspended, or something begins.
    async fn in_j(&mut self, since: Tick, seen: bool) -> Result<Line, Stop> {
        let same = Line::J { since, seen };
        let deadline = if !self.idle {
            Some(since.offset(self.timing.start(IDLE_BITS)))
        } else if seen && !self.suspended {
            Some(since.offset(self.suspend))
        } else {
            None
        };
        let edge = match self.wait(deadline).await? {
            Waited::Edge(tick, levels) => {
                self.levels = levels;
                tick
            }
            Waited::Deadline if self.idle => {
                let at = since.offset(self.suspend);
                self.suspended = true;
                self.out
                    .emit(
                        Provenance::over(TickRange::new(since, at), at),
                        UsbEvent::Suspend,
                    )
                    .await;
                return Ok(same);
            }
            Waited::Deadline => {
                self.idle = true;
                return Ok(same);
            }
        };
        let cell = self.settle(edge).await?;
        let next = match cell.state {
            // A glitch: the bus is still idle.
            LineState::J => return Ok(same),
            LineState::K if self.idle => self.search_sync(cell).await?,
            LineState::K => Line::K {
                since: edge,
                seen: true,
            },
            LineState::Se0 => Line::Se0 {
                since: edge,
                seen: true,
                after: if self.idle { After::Idle } else { After::Other },
            },
            LineState::Se1 => {
                self.idle = false;
                Line::Se1
            }
        };
        self.suspended = false;
        Ok(next)
    }

    /// K outside a packet: a resume under way, or activity the decoder cannot read.
    async fn in_k(&mut self, since: Tick, seen: bool) -> Result<Line, Stop> {
        let edge = self.edge().await?;
        let cell = self.settle(edge).await?;
        Ok(match cell.state {
            LineState::K => Line::K { since, seen },
            LineState::J => Line::J {
                since: edge,
                seen: true,
            },
            LineState::Se0 => Line::Se0 {
                since: edge,
                seen: true,
                after: if seen { After::K(since) } else { After::Other },
            },
            LineState::Se1 => {
                self.idle = false;
                Line::Se1
            }
        })
    }

    /// SE0: an EOP, a reset, the end of a resume or a keep-alive, told apart when it ends.
    async fn in_se0(&mut self, since: Tick, seen: bool, after: After) -> Result<Line, Stop> {
        let end = self.edge().await?;
        let state = self.config.state(self.levels);
        if state == LineState::Se0 {
            // No edge leaves both lines low; nothing ended.
            return Ok(Line::Se0 { since, seen, after });
        }
        if after == After::Packet
            && let Some(open) = self.open.take()
        {
            let at = Provenance::over(TickRange::new(since, end.next()), end);
            let reason = EndReason::Eop;
            self.out
                .emit(
                    at,
                    UsbEvent::End {
                        pkt: open.pkt,
                        reason,
                    },
                )
                .await;
        }
        let ticks = since.until(end);
        if seen && self.timing.lasts(ticks, RESET_PER_SECOND) {
            self.out
                .emit(
                    Provenance::over(TickRange::new(since, end), end),
                    UsbEvent::Reset { ticks },
                )
                .await;
        } else if seen && state == LineState::J {
            match after {
                After::K(k) if self.timing.lasts(k.until(since), RESUME_PER_SECOND) => {
                    self.out
                        .emit(
                            Provenance::over(TickRange::new(k, end.next()), end),
                            UsbEvent::Resume,
                        )
                        .await;
                }
                After::Idle if self.config.speed == Speed::Low => {
                    self.out
                        .emit(
                            Provenance::over(TickRange::new(since, end.next()), end),
                            UsbEvent::KeepAlive,
                        )
                        .await;
                }
                _ => {}
            }
        }
        Ok(match state {
            LineState::J => {
                self.idle = true;
                Line::J {
                    since: end,
                    seen: true,
                }
            }
            LineState::K => {
                self.idle = false;
                Line::K {
                    since: end,
                    seen: true,
                }
            }
            LineState::Se0 | LineState::Se1 => {
                self.idle = false;
                Line::Se1
            }
        })
    }

    /// SE1 outside a packet: waits for the bus to leave it.
    async fn in_se1(&mut self) -> Result<Line, Stop> {
        let edge = self.edge().await?;
        let cell = self.settle(edge).await?;
        Ok(match cell.state {
            LineState::Se1 => Line::Se1,
            LineState::J => Line::J {
                since: edge,
                seen: true,
            },
            LineState::K => Line::K {
                since: edge,
                seen: true,
            },
            LineState::Se0 => Line::Se0 {
                since: edge,
                seen: true,
                after: After::Other,
            },
        })
    }

    /// K on the idle bus: a SYNC begins, or something else does.
    async fn search_sync(&mut self, first: Cell) -> Result<Line, Stop> {
        let mut zeros = 1u32;
        let mut state = LineState::K;
        let mut run = first.start;
        loop {
            let cell = self.cell().await?;
            match cell.state {
                LineState::Se0 => {
                    return Ok(Line::Se0 {
                        since: cell.start,
                        seen: true,
                        after: After::Other,
                    });
                }
                LineState::Se1 => {
                    self.idle = false;
                    return Ok(Line::Se1);
                }
                now if now != state => {
                    zeros = zeros.saturating_add(1);
                    state = now;
                    run = cell.start;
                }
                LineState::K if zeros >= SYNC_ZEROS => return self.packet(first.start, cell).await,
                LineState::K => {
                    return Ok(Line::K {
                        since: run,
                        seen: true,
                    });
                }
                LineState::J => {
                    return Ok(Line::J {
                        since: run,
                        seen: true,
                    });
                }
            }
        }
    }

    /// A packet whose SYNC ran from `start` to the cell `last`.
    async fn packet(&mut self, start: Tick, last: Cell) -> Result<Line, Stop> {
        let pkt = PacketId(self.next_pkt);
        self.next_pkt += 1;
        self.open = Some(Open { pkt, payload: None });
        self.nrzi = LineState::K;
        self.ones = 1;
        let sync = Provenance::over(TickRange::new(start, last.centre.next()), last.centre);
        self.out.emit(sync, UsbEvent::Begin { pkt }).await;
        let (byte, at) = match self.field(8).await? {
            Ok(field) => field,
            Err(cut) => return Ok(self.cut(cut).await),
        };
        let code = (byte & 0x0f) as u8;
        let ok = (byte >> 4) as u8 == (!code & 0x0f);
        let pid = Pid::from_code(code);
        self.out.emit(at, UsbEvent::Pid { pkt, pid, ok }).await;
        if !ok {
            self.close(EndReason::PidError, at).await;
            return Ok(self.line_now());
        }
        match pid {
            Pid::Out | Pid::In | Pid::Setup | Pid::Ping => {
                let (value, at) = match self.field(16).await? {
                    Ok(field) => field,
                    Err(cut) => return Ok(self.cut(cut).await),
                };
                let mut crc = Crc5::new();
                crc.push(value, 16);
                let token = UsbEvent::Token {
                    pkt,
                    address: (value & 0x7f) as u8,
                    endpoint: ((value >> 7) & 0x0f) as u8,
                    crc5: ((value >> 11) & 0x1f) as u8,
                    ok: crc.is_residual(),
                };
                self.out.emit(at, token).await;
            }
            Pid::Sof => {
                let (value, at) = match self.field(16).await? {
                    Ok(field) => field,
                    Err(cut) => return Ok(self.cut(cut).await),
                };
                let mut crc = Crc5::new();
                crc.push(value, 16);
                let sof = UsbEvent::Sof {
                    pkt,
                    frame: (value & 0x7ff) as u16,
                    crc5: ((value >> 11) & 0x1f) as u8,
                    ok: crc.is_residual(),
                };
                self.out.emit(at, sof).await;
            }
            Pid::Split => {
                let (raw, at) = match self.field(24).await? {
                    Ok(field) => field,
                    Err(cut) => return Ok(self.cut(cut).await),
                };
                self.out.emit(at, UsbEvent::Split { pkt, raw }).await;
            }
            Pid::Data0 | Pid::Data1 | Pid::Data2 | Pid::Mdata => return self.payload().await,
            Pid::Pre => {
                self.close(EndReason::Preamble, at).await;
                return Ok(self.line_now());
            }
            Pid::Ack | Pid::Nak | Pid::Stall | Pid::Nyet | Pid::Err | Pid::Reserved => {}
        }
        // The EOP, past any bits this type of packet does not have.
        loop {
            if let Bit::Cut(cut) = self.bit().await? {
                return Ok(self.cut(cut).await);
            }
        }
    }

    /// The bytes of a data packet, through the start of its EOP.
    async fn payload(&mut self) -> Result<Line, Stop> {
        if let Some(open) = self.open.as_mut() {
            open.payload = Some(Payload::new());
        }
        loop {
            match self.field(8).await? {
                Ok((byte, at)) => self.payload_byte(byte as u8, at).await,
                Err(cut) => return Ok(self.cut(cut).await),
            }
        }
    }

    /// Takes a byte of a data packet; sends the batch once it is full.
    async fn payload_byte(&mut self, byte: u8, at: Provenance) {
        let Some(Open {
            pkt,
            payload: Some(payload),
        }) = self.open.as_mut()
        else {
            return;
        };
        let pkt = *pkt;
        payload.crc.push(byte);
        let Some((released, released_at)) = payload.hold(byte, at) else {
            return;
        };
        payload.add(released, released_at);
        if payload.batch.len() < DATA_BATCH {
            return;
        }
        if let Some((offset, bytes, batch)) = payload.take(at.settled) {
            self.out
                .emit(batch, UsbEvent::Data { pkt, offset, bytes })
                .await;
        }
    }

    /// The packet's bits have ended: an SE0 begins the EOP, SE1 and a bit stuff error end the
    /// packet. Returns the bus as it is left.
    async fn cut(&mut self, cut: Cut) -> Line {
        match cut {
            Cut::Se0(cell) => {
                self.payload_end(cell).await;
                Line::Se0 {
                    since: cell.start,
                    seen: true,
                    after: After::Packet,
                }
            }
            Cut::Se1(cell) => {
                self.close(EndReason::Se1, cell.at()).await;
                self.idle = false;
                Line::Se1
            }
            Cut::Stuff(cell) => {
                self.close(EndReason::BitStuffError, cell.at()).await;
                self.line_now()
            }
        }
    }

    /// At the first SE0 cell of a data packet: the rest of the payload, then its CRC16 and
    /// length.
    async fn payload_end(&mut self, cell: Cell) {
        let Some(open) = self.open.as_mut() else {
            return;
        };
        let pkt = open.pkt;
        let Some(mut payload) = open.payload.take() else {
            return;
        };
        if let Some((offset, bytes, batch)) = payload.take(cell.centre) {
            self.out
                .emit(batch, UsbEvent::Data { pkt, offset, bytes })
                .await;
        }
        let (crc16, span) = payload.crc_field();
        let ok = crc16.is_some() && payload.crc.is_residual();
        let length = u16::try_from(payload.count).unwrap_or(u16::MAX);
        let at = Provenance::over(span.unwrap_or(TickRange::at(cell.start)), cell.centre);
        self.out
            .emit(
                at,
                UsbEvent::DataEnd {
                    pkt,
                    crc16,
                    ok,
                    length,
                },
            )
            .await;
    }

    /// Ends the open packet, if any, with `reason`, sending the payload known so far.
    async fn close(&mut self, reason: EndReason, at: Provenance) {
        let Some(open) = self.open.take() else {
            return;
        };
        if let Some(mut payload) = open.payload
            && let Some((offset, bytes, batch)) = payload.take(at.settled)
        {
            self.out
                .emit(
                    batch,
                    UsbEvent::Data {
                        pkt: open.pkt,
                        offset,
                        bytes,
                    },
                )
                .await;
        }
        self.out
            .emit(
                at,
                UsbEvent::End {
                    pkt: open.pkt,
                    reason,
                },
            )
            .await;
    }

    /// The bus where a packet ended without an EOP: the lines as they are since the edge the
    /// clock was last aligned to, and no longer known to be between packets.
    fn line_now(&mut self) -> Line {
        self.idle = false;
        let since = self.clock.anchor;
        match self.config.state(self.levels) {
            LineState::J => Line::J { since, seen: true },
            LineState::K => Line::K { since, seen: true },
            LineState::Se0 => Line::Se0 {
                since,
                seen: true,
                after: After::Other,
            },
            LineState::Se1 => Line::Se1,
        }
    }

    /// Reads `count` bits of a packet, the least significant first, or what ended them.
    async fn field(&mut self, count: u32) -> Result<Result<(u32, Provenance), Cut>, Stop> {
        let mut value = 0u32;
        let mut start = Tick::ZERO;
        let mut last = Tick::ZERO;
        for i in 0..count {
            match self.bit().await? {
                Bit::Value(one, cell) => {
                    if i == 0 {
                        start = cell.start;
                    }
                    value |= u32::from(one) << i;
                    last = cell.centre;
                }
                Bit::Cut(cut) => return Ok(Err(cut)),
            }
        }
        let at = Provenance::over(TickRange::new(start, last.next()), last);
        Ok(Ok((value, at)))
    }

    /// Reads the next bit of a packet: NRZI, with stuffed bits dropped.
    async fn bit(&mut self) -> Result<Bit, Stop> {
        loop {
            let cell = self.cell().await?;
            let state = match cell.state {
                LineState::Se0 => return Ok(Bit::Cut(Cut::Se0(cell))),
                LineState::Se1 => return Ok(Bit::Cut(Cut::Se1(cell))),
                state => state,
            };
            let one = state == self.nrzi;
            self.nrzi = state;
            if self.ones >= 6 {
                self.ones = 0;
                if one {
                    return Ok(Bit::Cut(Cut::Stuff(cell)));
                }
                continue;
            }
            self.ones = if one { self.ones + 1 } else { 0 };
            return Ok(Bit::Value(one, cell));
        }
    }

    /// Aligns the clock to `edge` and reads the cell it begins.
    async fn settle(&mut self, edge: Tick) -> Result<Cell, Stop> {
        self.clock = Clock::at(edge);
        self.cell().await
    }

    /// Reads the next bit cell at its centre, aligning the clock to the first edge after the
    /// last centre.
    async fn cell(&mut self) -> Result<Cell, Stop> {
        loop {
            let centre = self
                .clock
                .anchor
                .offset(self.timing.centre(self.clock.next));
            match self.wait(Some(centre.next())).await? {
                Waited::Edge(tick, levels) => {
                    self.levels = levels;
                    if !self.clock.realigned {
                        self.clock = Clock::at(tick);
                    }
                }
                Waited::Deadline => {
                    let start = self.clock.anchor.offset(self.timing.start(self.clock.next));
                    self.clock.next += 1;
                    self.clock.realigned = false;
                    let state = self.config.state(self.levels);
                    return Ok(Cell {
                        start,
                        centre,
                        state,
                    });
                }
            }
        }
    }

    /// Waits for the next edge, whenever it comes.
    async fn edge(&mut self) -> Result<Tick, Stop> {
        loop {
            if let Waited::Edge(tick, levels) = self.wait(None).await? {
                self.levels = levels;
                return Ok(tick);
            }
        }
    }

    /// Waits for an edge on either line before `until`, passing progress on.
    ///
    /// Samples that reach the deadline with no edge are the deadline passing, however the
    /// cursor learns it: by `Quiet`, by the progress of a block used up right at the deadline,
    /// or by a gap or the end of the samples that comes right after such a block — which is
    /// then held back for the next wait, so that what ends at the deadline is still read.
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
                Change::At { tick, levels, .. } => return Ok(Waited::Edge(tick, levels)),
                Change::Quiet(_) => return Ok(Waited::Deadline),
                Change::Progress(tick) if reached(tick) => {
                    if let Some(until) = until {
                        self.signal.seek(until);
                    }
                    return Ok(Waited::Deadline);
                }
                Change::Progress(tick) => self.out.progress(tick).await,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_states_follow_the_speed() {
        let full = UsbConfig::default();
        let low = UsbConfig {
            speed: Speed::Low,
            ..UsbConfig::default()
        };
        assert_eq!(full.state(Levels(0b01)), LineState::J);
        assert_eq!(full.state(Levels(0b10)), LineState::K);
        assert_eq!(low.state(Levels(0b10)), LineState::J);
        assert_eq!(low.state(Levels(0b01)), LineState::K);
        assert_eq!(full.state(Levels(0b00)), LineState::Se0);
        assert_eq!(low.state(Levels(0b11)), LineState::Se1);
        for config in [&full, &low] {
            for state in [LineState::J, LineState::K, LineState::Se0, LineState::Se1] {
                let (dp, dm) = config.lines(state);
                let levels = Levels(u64::from(dp) | (u64::from(dm) << 1));
                assert_eq!(config.state(levels), state);
            }
        }
    }

    #[test]
    fn configurations_are_checked() {
        assert_eq!(UsbConfig::default().validate(), Ok(()));
        let same = UsbConfig {
            dm: 0,
            ..UsbConfig::default()
        };
        assert_eq!(same.validate(), Err(UsbConfigError::DuplicateChannel(0)));
        let far = UsbConfig {
            dp: 64,
            ..UsbConfig::default()
        };
        assert_eq!(far.validate(), Err(UsbConfigError::Channel(64)));
    }

    #[test]
    fn ports_declare_the_speed_and_require_four_samples_per_bit() {
        let low = UsbConfig {
            dp: 4,
            dm: 2,
            speed: Speed::Low,
        };
        assert_eq!(low.props().get("speed"), Some(&"low".into()));
        assert_eq!(low.props().get("bitRateHz"), Some(&1_500_000i64.into()));
        let requires = low.requires();
        let source =
            |channels: i64, rate: i64| Props::new().with("channels", channels).with("rateHz", rate);
        assert!(requires.unmet(&source(5, 6_000_000)).is_none());
        assert_eq!(
            requires.unmet(&source(4, 6_000_000)).map(|(key, _)| key),
            Some("channels")
        );
        assert_eq!(
            requires.unmet(&source(5, 5_999_999)).map(|(key, _)| key),
            Some("rateHz")
        );
        assert_eq!(UsbConfig::default().requires(), {
            Requires::new()
                .at_least("channels", 2)
                .at_least("rateHz", 48_000_000)
        });
    }

    #[test]
    fn timing_rounds_down_and_compares_exactly() {
        // 50 MHz at 12 Mbit/s: 4⅙ ticks per bit.
        let timing = Timing {
            rate: 50_000_000,
            bit_rate: 12_000_000,
        };
        assert_eq!(timing.start(1), 4);
        assert_eq!(timing.start(6), 25);
        assert_eq!(timing.centre(0), 2);
        assert_eq!(timing.centre(1), 6);
        assert_eq!(timing.centre(6), 27);
        assert_eq!(timing.suspend(), 150_000);
        // 2.5 µs at 48 MHz is 120 ticks.
        let timing = Timing {
            rate: 48_000_000,
            bit_rate: 12_000_000,
        };
        assert!(timing.lasts(120, RESET_PER_SECOND));
        assert!(!timing.lasts(119, RESET_PER_SECOND));
        assert!(timing.lasts(960_000, RESUME_PER_SECOND));
        assert!(!timing.lasts(959_999, RESUME_PER_SECOND));
    }

    #[test]
    fn crcs_leave_their_residuals() {
        // SETUP to address 0, endpoint 0: the token bytes 2D 00 10.
        let mut crc = Crc5::new();
        crc.push(0x1000, 16);
        assert!(crc.is_residual());
        let mut crc = Crc5::new();
        crc.push(0x1001, 16);
        assert!(!crc.is_residual());
        // A setup packet and its CRC16 bytes, DD 94.
        let mut crc = Crc16::new();
        for byte in [0x80, 0x06, 0x00, 0x01, 0x00, 0x00, 0x40, 0x00, 0xdd, 0x94] {
            crc.push(byte);
        }
        assert!(crc.is_residual());
        // No payload: the CRC16 is 0.
        let mut crc = Crc16::new();
        crc.push(0);
        crc.push(0);
        assert!(crc.is_residual());
    }

    #[test]
    fn payload_holds_back_two_bytes() {
        let mut payload = Payload::new();
        let at = |tick: u64| Provenance::one(Tick(tick));
        assert_eq!(payload.hold(1, at(1)), None);
        assert_eq!(payload.hold(2, at(2)), None);
        assert_eq!(payload.hold(3, at(3)), Some((1, at(1))));
        assert_eq!(payload.crc_field().0, Some(0x0302));
        assert_eq!(
            payload.crc_field().1,
            Some(TickRange::new(Tick(2), Tick(4)))
        );
        payload.add(1, at(1));
        assert_eq!(
            payload.take(Tick(9)),
            Some((
                0,
                vec![1],
                Provenance::over(TickRange::one(Tick(1)), Tick(9))
            ))
        );
        assert_eq!(payload.take(Tick(9)), None);
    }
}
