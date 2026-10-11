// SPDX-License-Identifier: Apache-2.0

//! The CAN decoder: classic frames (CAN 2.0A and 2.0B) and CAN FD frames, read from the receive
//! line of a transceiver.
//!
//! Implemented from ISO 11898-1:2015 (the data link layer and physical signalling), the Bosch
//! CAN Specification 2.0 parts A and B for the classic frame, and the Bosch CAN with Flexible
//! Data-Rate Specification 1.0 for the FD frame, with the ISO additions to it (the stuff count
//! and the CRC initial value) as published by Bosch in "CAN FD and the CRC issue" (CAN
//! Newsletter 1/2015). The rules:
//!
//! - **Levels.** The line is dominant when low (`dominant_low`, the transceiver's RX pin) or
//!   when high; dominant is the logical 0, recessive the logical 1. Idle is recessive.
//! - **Bit timing.** A bit lasts `R / B` ticks (`R` ticks per second from `Io::time_base`, `B`
//!   bits per second), kept as an exact fraction. Counted from the last synchronisation at
//!   tick `s`, bit `k` starts at `s + ⌊k · R / B⌋` and is sampled at
//!   `s + ⌊(100k + p) · R / (100 · B)⌋` for a sample point of `p` percent: the bit's start plus
//!   `p` percent of the bit time, rounded down once, so rounding never accumulates. The decoder
//!   waits for edges up to the sample point (`next_change_before`), then reads the level there
//!   (`sample`).
//! - **Synchronisation.** A simplified resynchronisation without phase segments: a hard
//!   synchronisation on the edge of SOF, and on every later recessive-to-dominant edge the edge
//!   becomes the start of the bit being read, however far it is from where that bit was
//!   expected. As the CAN specifications require, an edge synchronises only when the bit sampled
//!   before it was recessive, and only once between two sample points. Dominant-to-recessive
//!   edges never synchronise.
//! - **Integration.** At the first sample and after a gap the decoder waits for eleven
//!   consecutive recessive bits, bus idle, before it accepts a SOF. Traffic before that is not
//!   decoded: it is reported as one frame, `begin { partial: true }` at the first sample, settled
//!   where the bus was first seen dominant, and `end { aborted }` where the bus was found idle.
//! - **Frames.** A frame starts with SOF, a dominant bit on the idle bus or in the third bit of
//!   the intermission; a dominant pulse that is over by the sample point is no SOF. Then the
//!   11-bit base identifier; RTR (classic) or RRS (FD), or in the extended format SRR, IDE, the
//!   18-bit identifier extension and RTR or RRS; FDF (the reserved bit `r0`, `r1` in extended
//!   frames, when `fd` is off or the bit is dominant), and in FD frames `res`, BRS and ESI, in
//!   classic extended frames `r0`; the DLC; the data field, absent in remote frames (RTR
//!   recessive, classic frames only); the CRC field; the CRC delimiter; the ACK slot and ACK
//!   delimiter; seven bits of end of frame; three bits of intermission. Reserved bits, SRR and
//!   RRS are accepted with either value; a recessive `res` is a protocol exception.
//! - **Stuffing.** From SOF through the CRC sequence in classic frames, and from SOF through
//!   the data field in FD frames, five equal bits are followed by a stuff bit of the opposite
//!   value, which is removed; a sixth equal bit there is a stuff error (or, when dominant, an
//!   error flag). In FD frames the CRC field has fixed stuff bits instead: one before the stuff
//!   count and one after every fourth bit of the stuff count and CRC sequence, each the inverse
//!   of the bit before it; it takes the place of a dynamic stuff bit due after the data field,
//!   which is not sent.
//! - **Bit rate switching.** In FD frames with BRS recessive the data bit rate applies from
//!   the sample point of BRS to the sample point of the CRC delimiter; the sample point is the
//!   same percentage in both phases. Without `fd_bitrate` such a frame ends `unsupported` at
//!   BRS and the decoder integrates again: it waits for bus idle before the next SOF.
//! - **CRC.** CRC-15 over the destuffed bits from SOF through the data field in classic
//!   frames; in FD frames CRC-17 (up to 16 data bytes) or CRC-21 over the same bits with their
//!   dynamic stuff bits and then the stuff count, from an initial value of 1 followed by
//!   zeros. The FD stuff count is the number of dynamic stuff bits modulo 8, Gray-coded, with
//!   an even parity bit. All three registers run from SOF; FDF and the DLC choose one. `crc`
//!   reports the received sequence, and whether it, the stuff count and the parity match.
//! - **ACK.** A dominant ACK slot acknowledges the frame. FD frames accept the dominant ACK one
//!   bit late (a CRC delimiter two bits long) and two bits long, as the FD specification
//!   allows for the phase shift after the bit rate switch.
//! - **Errors.** Six consecutive dominant bits anywhere in a frame are an active error flag:
//!   the frame ends `error-frame` at the sixth. A dominant bit where the frame's form calls for
//!   a recessive one (the CRC delimiter, the ACK delimiter, end of frame bits 1 to 6) is a form
//!   error, and a fixed stuff bit equal to the bit before it a stuff error; when that bit is
//!   dominant the decision waits until the line turns recessive again, so that an error flag
//!   starting there is reported as `error-frame`. After an error the decoder waits for the
//!   eight recessive bits of the error delimiter, then the intermission, before it accepts a
//!   SOF.
//! - **Overload.** A dominant bit at the last bit of end of frame (which receivers do not
//!   check: the frame before it is whole) or in the first two bits of the intermission is an
//!   overload frame, reported as a frame of its own, `begin` and `end { overload }` at that
//!   bit; its delimiter and the intermission follow as after an error.
//! - **Unsupported frames.** A recessive `res` bit (a protocol exception: such frames carry a
//!   later format, CAN XL for one) ends the frame `unsupported`, as BRS does without
//!   `fd_bitrate`; the decoder then integrates again, without reporting the rest of the frame
//!   as a partial one.
//! - **Gaps and the end of the samples.** A frame under way ends `aborted` where the samples
//!   stop; the gap or the end is passed on, and after a gap the decoder integrates again.
//!   Leftover bits never make a field.
//! - **Provenance.** A field's event covers its bits, from the start of the first to the end of
//!   the last as the bit timing placed them when the last was sampled, stuff bits inside it
//!   included, and settles at the last bit's sample point. A base identifier settles at IDE,
//!   which tells it from an extended one. A data batch flushed by an abnormal end, and an end
//!   decided after its offending bit, settle where they were decided; `aborted` is placed,
//!   with no span, where the samples stopped.
//! - **Progress.** Progress is passed on, held back to the start of what a later event may
//!   still cover: the frame under way, from the start of its SOF, as a field, a data batch or
//!   an error flag not yet reported may begin well before the bit being read; the bit being
//!   read, which may begin a frame or an overload frame; and, while the decoder integrates at
//!   the first sample or after a gap and has not yet found the bus busy, that sample, where a
//!   partial frame would begin.
//!
//! Limits: one receive line; one sample point for both phases; the resynchronisation has no
//! jump width limit and no phase segments; non-ISO FD frames (without the stuff count) and
//! CAN XL frames are not decoded; error-passive flags and the suspend transmission field are
//! not reported, nor are dominant bits inside an error or overload delimiter.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use uiharu_core::{
    Change, ChannelMask, InputRef, InputSpec, Io, Levels, LogicBlock, MAX_CHANNELS, Node,
    NodeFuture, NodeId, NodeSpec, Outbox, OutputRef, OutputSpec, Props, Provenance, Requires,
    Sample, SignalCursor, Tick, TickRange, TimeBase,
};

use super::crc::Crc;
use super::events::{CanEvent, DATA_BATCH, EndReason, FrameId};
use super::timing::Clock;

/// The fewest ticks a bit may last, at the fastest bit rate the decoder follows: a slower
/// source is refused when it is connected.
pub const MIN_TICKS_PER_BIT: u64 = 4;

/// How the bus is wired and timed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CanConfig {
    /// The channel carrying the receive line, as the transceiver puts it out.
    pub rx: u8,
    /// Whether the dominant level is low, as on a transceiver's RX pin.
    pub dominant_low: bool,
    /// The nominal bit rate in bits per second: the arbitration phase of every frame, and all
    /// of classic frames and of FD frames without bit rate switching.
    pub bitrate: u32,
    /// The bit rate of the data phase of FD frames with bit rate switching (BRS recessive).
    /// Without it such frames are not decoded past BRS: they end `unsupported`.
    pub fd_bitrate: Option<u32>,
    /// Where the line is sampled, in percent of the bit time from the start of the bit, 1 to
    /// 99; in both phases.
    pub sample_point: u8,
    /// Whether FD frames are recognised by their FDF bit. When off every frame is classic and
    /// the bit is the reserved bit `r0` (`r1` in the extended format).
    pub fd: bool,
}

impl Default for CanConfig {
    fn default() -> Self {
        Self {
            rx: 0,
            dominant_low: true,
            bitrate: 500_000,
            fd_bitrate: None,
            sample_point: 87,
            fd: true,
        }
    }
}

impl CanConfig {
    /// Whether `levels` show the bus dominant.
    pub fn dominant(&self, levels: Levels) -> bool {
        levels.high(self.rx) != self.dominant_low
    }

    /// The fastest bit rate the decoder follows: the data bit rate when FD frames switch to
    /// it, the nominal one otherwise.
    pub fn fastest_bitrate(&self) -> u32 {
        match self.fd_bitrate {
            Some(data) if self.fd => data.max(self.bitrate),
            _ => self.bitrate,
        }
    }

    /// The lowest sample rate the decoder accepts: [`MIN_TICKS_PER_BIT`] ticks per bit at the
    /// fastest bit rate.
    pub fn min_rate_hz(&self) -> u64 {
        MIN_TICKS_PER_BIT * u64::from(self.fastest_bitrate())
    }

    /// Checks the configuration.
    pub fn validate(&self) -> Result<(), CanConfigError> {
        if self.rx >= MAX_CHANNELS {
            return Err(CanConfigError::Channel(self.rx));
        }
        if self.bitrate == 0 {
            return Err(CanConfigError::Bitrate);
        }
        if let Some(data) = self.fd_bitrate
            && data < self.bitrate
        {
            return Err(CanConfigError::FdBitrate {
                nominal: self.bitrate,
                data,
            });
        }
        if !(1..=99).contains(&self.sample_point) {
            return Err(CanConfigError::SamplePoint(self.sample_point));
        }
        Ok(())
    }

    /// What the decoder's output port declares: `bitrate`, `samplePoint`, `fd`, and
    /// `fdBitrate` when FD frames switch to a data bit rate.
    pub fn props(&self) -> Props {
        let props = Props::new()
            .with("bitrate", i64::from(self.bitrate))
            .with("samplePoint", i64::from(self.sample_point))
            .with("fd", self.fd);
        match self.fd_bitrate {
            Some(data) if self.fd => props.with("fdBitrate", i64::from(data)),
            _ => props,
        }
    }
}

/// Why a configuration is rejected.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum CanConfigError {
    /// The receive channel is out of range.
    #[error("channel {0} is out of range; channels are 0 to {last}", last = MAX_CHANNELS - 1)]
    Channel(u8),
    /// The nominal bit rate is zero.
    #[error("the bit rate must be at least 1 bit/s")]
    Bitrate,
    /// The data bit rate is below the nominal one.
    #[error("the data bit rate of {data} bit/s is below the nominal bit rate of {nominal} bit/s")]
    FdBitrate {
        /// The nominal bit rate.
        nominal: u32,
        /// The data bit rate.
        data: u32,
    },
    /// The sample point is not 1 to 99 percent.
    #[error("a sample point at {0} % of the bit; it must be 1 to 99 %")]
    SamplePoint(u8),
}

/// The CAN decoder node: logic samples in, `can.events` out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Can {
    config: CanConfig,
}

impl Can {
    /// A decoder for `config`.
    pub fn new(config: CanConfig) -> Result<Self, CanConfigError> {
        config.validate()?;
        Ok(Self { config })
    }

    /// The configuration.
    pub fn config(&self) -> &CanConfig {
        &self.config
    }

    /// The logic input of the decoder added as `node`.
    pub fn input(node: NodeId) -> InputRef<Arc<LogicBlock>> {
        InputRef::of_node(node, 0)
    }

    /// The event output of the decoder added as `node`.
    pub fn events(node: NodeId) -> OutputRef<CanEvent> {
        OutputRef::of_node(node, 0)
    }
}

impl Node for Can {
    fn spec(&self) -> NodeSpec {
        NodeSpec {
            name: "can".into(),
            inputs: vec![InputSpec::new::<Arc<LogicBlock>>(
                "logic",
                Requires::new()
                    .at_least("channels", i64::from(self.config.rx) + 1)
                    .at_least(
                        "rateHz",
                        i64::try_from(self.config.min_rate_hz()).unwrap_or(i64::MAX),
                    ),
            )],
            outputs: vec![OutputSpec::new::<CanEvent>("events", self.config.props())],
        }
    }

    fn start(self: Box<Self>, mut io: Io) -> NodeFuture {
        let signal = io.signal(0);
        let out = io.output(0);
        match io.time_base() {
            Some(time_base) if time_base.rate_hz >= self.config.min_rate_hz() => {
                Box::pin(run(signal, out, self.config, time_base))
            }
            // The graph refuses a source this slow; without a usable clock nothing is decoded.
            _ => Box::pin(pass_through(signal, out)),
        }
    }
}

/// Why reading stopped: samples are missing from here on, or they have ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stop {
    Gap(TickRange),
    End(Tick),
}

impl Stop {
    /// Where the samples stopped.
    fn tick(self) -> Tick {
        match self {
            Stop::Gap(range) => range.start,
            Stop::End(tick) => tick,
        }
    }
}

/// A bit as it was sampled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Bit {
    dominant: bool,
    /// Where the bit timing placed the bit when it was sampled.
    start: Tick,
    end: Tick,
    /// Where it was sampled.
    sample: Tick,
}

impl Bit {
    /// The logical value: recessive is 1.
    fn value(&self) -> bool {
        !self.dominant
    }

    /// The bit's ticks, settled at its sample point.
    fn provenance(&self) -> Provenance {
        Provenance::over(TickRange::new(self.start, self.end), self.sample)
    }
}

/// Bits read as one field, most significant first.
#[derive(Clone, Copy, Debug)]
struct Field {
    value: u32,
    start: Tick,
    last: Bit,
}

impl Field {
    /// The field's ticks, settled at its last bit's sample point.
    fn provenance(&self) -> Provenance {
        Provenance::over(TickRange::new(self.start, self.last.end), self.last.sample)
    }
}

/// Where the decoder stands between frames.
#[derive(Clone, Copy, Debug)]
enum State {
    /// Waiting for eleven recessive bits; traffic meanwhile makes a partial frame if `report`.
    Integrate { report: bool },
    /// The bus is idle: the next dominant edge starts a frame.
    Idle,
    /// A frame starts with this SOF bit.
    Frame(Bit),
    /// The three bits after a frame or a delimiter.
    Intermission,
    /// After an error or overload flag: waiting for the eight recessive bits of its delimiter.
    Delimiter,
}

/// Why the body of a frame stopped.
#[derive(Clone, Copy, Debug)]
enum Halt {
    /// The frame ended on the bus for this reason.
    End(EndReason, Provenance),
    /// The samples stopped while the frame was under way.
    Stop(Stop),
    /// The frame had already failed for this reason when the samples stopped, before the
    /// failure could be told from an error flag.
    EndThenStop(EndReason, Provenance, Stop),
}

impl From<Stop> for Halt {
    fn from(stop: Stop) -> Self {
        Halt::Stop(stop)
    }
}

/// The state of the frame under way.
#[derive(Debug)]
struct FrameState {
    id: FrameId,
    /// Where the frame began, the start of its SOF: none of its events begins earlier.
    start: Tick,
    /// The value of the last bit and how many equal bits end there, for dynamic stuffing.
    last: bool,
    same: u8,
    /// The value of the last bit on the bus, stuff bits included, for fixed stuff bits.
    prev: bool,
    /// Consecutive dominant bits and the start of the first.
    run: u8,
    run_start: Tick,
    /// Whether bits still go into the CRCs: SOF through the data field.
    crc_on: bool,
    crc15: Crc,
    crc17: Crc,
    crc21: Crc,
    /// Dynamic stuff bits so far.
    stuff_bits: u32,
}

impl FrameState {
    fn new(id: FrameId, sof: &Bit) -> Self {
        let (mut crc15, mut crc17, mut crc21) = (Crc::crc15(), Crc::crc17(), Crc::crc21());
        for crc in [&mut crc15, &mut crc17, &mut crc21] {
            crc.push(sof.value());
        }
        Self {
            id,
            start: sof.start,
            last: sof.value(),
            same: 1,
            prev: sof.value(),
            run: 1,
            run_start: sof.start,
            crc_on: true,
            crc15,
            crc17,
            crc21,
            stuff_bits: 0,
        }
    }
}

/// What the end of frame looked like.
#[derive(Clone, Copy, Debug)]
struct Eof {
    first: Bit,
    last: Bit,
}

/// Data bytes waiting to go out, at most [`DATA_BATCH`].
#[derive(Debug)]
struct Batch {
    frame: FrameId,
    offset: u32,
    bytes: Vec<u8>,
    at: Option<Provenance>,
}

/// The number of data bytes a DLC stands for.
fn data_length(dlc: u8, fd: bool) -> u8 {
    match dlc {
        0..=8 => dlc,
        _ if !fd => 8,
        9 => 12,
        10 => 16,
        11 => 20,
        12 => 24,
        13 => 32,
        14 => 48,
        _ => 64,
    }
}

/// The number a 3-bit Gray code stands for.
fn gray_decode(gray: u8) -> u8 {
    gray ^ (gray >> 1) ^ (gray >> 2)
}

struct Decoder {
    signal: SignalCursor,
    out: Outbox<CanEvent>,
    config: CanConfig,
    mask: ChannelMask,
    /// Ticks per second.
    rate: u64,
    /// The nominal bit rate.
    nominal: u64,
    /// The sample point, in hundredths of a bit.
    sample_point: u64,
    clock: Clock,
    /// The next bit to read, counted like the clock's phases.
    bit: u64,
    /// Whether the last bit sampled was dominant.
    last_dominant: bool,
    /// Whether the bit being read has been synchronised already.
    synced: bool,
    /// Where the last bit was sampled.
    last_sample: Tick,
    /// Whether the data bit rate applies.
    data_phase: bool,
    next_frame: u64,
    batch: Batch,
}

impl Decoder {
    fn new(
        signal: SignalCursor,
        out: Outbox<CanEvent>,
        config: CanConfig,
        time_base: TimeBase,
    ) -> Self {
        let nominal = u64::from(config.bitrate);
        Self {
            signal,
            out,
            mask: ChannelMask::one(config.rx),
            rate: time_base.rate_hz,
            nominal,
            sample_point: u64::from(config.sample_point),
            clock: Clock::new(Tick::ZERO, 0, time_base.rate_hz, nominal),
            bit: 0,
            last_dominant: false,
            synced: false,
            last_sample: Tick::ZERO,
            data_phase: false,
            next_frame: 1,
            batch: Batch {
                frame: FrameId(0),
                offset: 0,
                bytes: Vec::with_capacity(DATA_BATCH),
                at: None,
            },
            config,
        }
    }

    fn dominant(&self, levels: Levels) -> bool {
        self.config.dominant(levels)
    }

    fn new_frame(&mut self) -> FrameId {
        let frame = FrameId(self.next_frame);
        self.next_frame += 1;
        frame
    }

    async fn emit(&mut self, at: Provenance, event: CanEvent) {
        self.out.emit(at, event).await;
    }

    /// Restarts the bit timing at `tick`, on the nominal bit rate.
    fn restart(&mut self, tick: Tick) {
        self.clock = Clock::new(tick, 0, self.rate, self.nominal);
        self.bit = 0;
        self.data_phase = false;
    }

    /// Switches the bit rate at the sample point of `bit`, the bit just read: the rest of it
    /// and the bits after it last as long as `bitrate` makes them.
    fn switch_rate(&mut self, bit: &mut Bit, bitrate: u64, data_phase: bool) {
        self.clock = Clock::new(bit.sample, self.sample_point, self.rate, bitrate);
        self.bit = 1;
        self.data_phase = data_phase;
        bit.end = self.clock.tick(100);
    }

    /// Passes progress to `tick` on, held back to `hold` and to the start of the bit being
    /// read: no event that follows may begin earlier.
    async fn progress(&mut self, tick: Tick, hold: Option<Tick>) {
        let bit = self.clock.tick(self.bit.saturating_mul(100));
        let hold = hold.map_or(bit, |hold| hold.min(bit));
        self.out.progress(tick.min(hold)).await;
    }

    /// Reads the next bit: waits for edges up to its sample point, synchronising on them, and
    /// reads the level there. Progress met meanwhile is passed on held back to `hold`, where
    /// the oldest part a later event may still cover begins, and to the start of the bit.
    async fn raw_bit(&mut self, hold: Option<Tick>) -> Result<Bit, Stop> {
        let sample = loop {
            let target = self
                .clock
                .tick(
                    self.bit
                        .saturating_mul(100)
                        .saturating_add(self.sample_point),
                )
                .max(self.signal.position());
            match self.signal.next_change_before(self.mask, target).await {
                Change::At { tick, levels, .. } => {
                    if self.dominant(levels) && !self.last_dominant && !self.synced {
                        self.clock = Clock::new(tick, 0, self.rate, self.clock.bitrate());
                        self.bit = 0;
                        self.synced = true;
                    }
                }
                Change::Quiet(tick) => break tick,
                Change::Progress(tick) => self.progress(tick, hold).await,
                Change::Gap(range) => return Err(Stop::Gap(range)),
                Change::End(tick) => return Err(Stop::End(tick)),
            }
        };
        let levels = loop {
            match self.signal.sample().await {
                Sample::At { levels, .. } => break levels,
                Sample::Progress(tick) => self.progress(tick, hold).await,
                Sample::Gap(range) => return Err(Stop::Gap(range)),
                Sample::End(tick) => return Err(Stop::End(tick)),
            }
        };
        let dominant = self.dominant(levels);
        let start = self.clock.tick(self.bit.saturating_mul(100));
        self.bit = self.bit.saturating_add(1);
        let end = self.clock.tick(self.bit.saturating_mul(100));
        self.last_dominant = dominant;
        self.synced = false;
        self.last_sample = sample;
        Ok(Bit {
            dominant,
            start,
            end,
            sample,
        })
    }

    /// Passes over a dominant stretch, which a bit just sampled dominant began: waits for the
    /// line to turn recessive and goes on with the first bit sampled after that, as if every
    /// bit in between had been sampled dominant. Progress passes as it comes: stretches are
    /// skipped only in a delimiter and while integrating, where nothing is reported afterwards
    /// but the end of a partial frame, placed at a later sample point or where the samples stop.
    async fn skip_dominant(&mut self) -> Result<(), Stop> {
        loop {
            match self.signal.next_change(self.mask).await {
                Change::At { tick, levels, .. } => {
                    if !self.dominant(levels) {
                        let next = self.clock.first_sampled_from(tick, self.sample_point);
                        self.bit = self.bit.max(next);
                        return Ok(());
                    }
                }
                // Without a deadline the wait is never quiet.
                Change::Quiet(_) => {}
                Change::Progress(tick) => self.out.progress(tick).await,
                Change::Gap(range) => return Err(Stop::Gap(range)),
                Change::End(tick) => return Err(Stop::End(tick)),
            }
        }
    }

    /// Waits for eleven consecutive recessive bits from where the cursor stands; reports what
    /// was on the bus meanwhile as a partial frame when `report`.
    async fn integrate(&mut self, report: bool) -> Result<State, Stop> {
        let (first, dominant) = loop {
            match self.signal.sample().await {
                Sample::At { tick, levels } => break (tick, self.dominant(levels)),
                Sample::Progress(tick) => self.out.progress(tick).await,
                Sample::Gap(range) => return Err(Stop::Gap(range)),
                Sample::End(tick) => return Err(Stop::End(tick)),
            }
        };
        // Nothing is known of the bit timing yet: bits are counted from the first sample.
        self.restart(first);
        self.last_dominant = dominant;
        self.synced = false;
        let mut partial = None;
        let mut busy = dominant.then_some(first);
        let mut recessive = 0;
        loop {
            if let Some(seen) = busy {
                recessive = 0;
                if report && partial.is_none() {
                    let frame = self.new_frame();
                    let at = Provenance::over(TickRange::one(first), seen);
                    self.emit(
                        at,
                        CanEvent::Begin {
                            frame,
                            partial: true,
                        },
                    )
                    .await;
                    partial = Some(frame);
                }
                if let Err(stop) = self.skip_dominant().await {
                    self.abort_partial(partial, stop).await;
                    return Err(stop);
                }
            }
            // Until the bus is seen busy, a partial frame may still begin at the first sample.
            let hold = (report && partial.is_none()).then_some(first);
            let bit = match self.raw_bit(hold).await {
                Ok(bit) => bit,
                Err(stop) => {
                    self.abort_partial(partial, stop).await;
                    return Err(stop);
                }
            };
            busy = bit.dominant.then_some(bit.sample);
            if !bit.dominant {
                recessive += 1;
                if recessive == 11 {
                    if let Some(frame) = partial {
                        let end = CanEvent::End {
                            frame,
                            reason: EndReason::Aborted,
                        };
                        self.emit(Provenance::at(bit.sample), end).await;
                    }
                    return Ok(State::Idle);
                }
            }
        }
    }

    /// Ends the partial frame, if one is open, where the samples stopped.
    async fn abort_partial(&mut self, partial: Option<FrameId>, stop: Stop) {
        if let Some(frame) = partial {
            let end = CanEvent::End {
                frame,
                reason: EndReason::Aborted,
            };
            self.emit(Provenance::at(stop.tick()), end).await;
        }
    }

    /// Waits on the idle bus for a SOF: a dominant edge, hard synchronisation, and a dominant
    /// level at the sample point. A shorter dominant pulse leaves the bus idle.
    async fn wait_for_sof(&mut self) -> Result<Bit, Stop> {
        loop {
            match self.signal.next_change(self.mask).await {
                Change::At { tick, levels, .. } if self.dominant(levels) => {
                    self.restart(tick);
                    self.last_dominant = false;
                    self.synced = true;
                    // The bit may be a SOF: progress stops at its start, the edge.
                    let bit = self.raw_bit(None).await?;
                    if bit.dominant {
                        return Ok(bit);
                    }
                }
                Change::At { .. } | Change::Quiet(_) => {}
                Change::Progress(tick) => self.out.progress(tick).await,
                Change::Gap(range) => return Err(Stop::Gap(range)),
                Change::End(tick) => return Err(Stop::End(tick)),
            }
        }
    }

    /// The intermission: a dominant bit in its first two bits is an overload frame, in its
    /// third a SOF; after three recessive bits the bus is idle.
    async fn intermission(&mut self) -> Result<State, Stop> {
        for position in 1..=3 {
            // Each bit may begin a frame or an overload frame: progress stops at its start.
            let bit = self.raw_bit(None).await?;
            if bit.dominant {
                if position == 3 {
                    return Ok(State::Frame(bit));
                }
                self.overload(bit).await;
                return Ok(State::Delimiter);
            }
        }
        Ok(State::Idle)
    }

    /// The delimiter after an error or overload flag: eight consecutive recessive bits, however
    /// long the flags before them.
    async fn delimiter(&mut self) -> Result<State, Stop> {
        let mut recessive = 0;
        while recessive < 8 {
            let bit = self.raw_bit(None).await?;
            if bit.dominant {
                recessive = 0;
                self.skip_dominant().await?;
            } else {
                recessive += 1;
            }
        }
        Ok(State::Intermission)
    }

    /// Reports an overload frame at `bit`, its first dominant bit.
    async fn overload(&mut self, bit: Bit) {
        let frame = self.new_frame();
        let at = bit.provenance();
        self.emit(
            at,
            CanEvent::Begin {
                frame,
                partial: false,
            },
        )
        .await;
        self.emit(
            at,
            CanEvent::End {
                frame,
                reason: EndReason::Overload,
            },
        )
        .await;
    }

    /// Decodes the frame that `sof` begins and closes it however it ends.
    async fn frame(&mut self, sof: Bit) -> Result<State, Stop> {
        let id = self.new_frame();
        self.emit(
            sof.provenance(),
            CanEvent::Begin {
                frame: id,
                partial: false,
            },
        )
        .await;
        let mut frame = FrameState::new(id, &sof);
        let (reason, at, stop) = match self.body(&mut frame).await {
            Ok(eof) => {
                let at = Provenance::over(
                    TickRange::new(eof.first.start, eof.last.end),
                    eof.last.sample,
                );
                self.emit(
                    at,
                    CanEvent::End {
                        frame: id,
                        reason: EndReason::Eof,
                    },
                )
                .await;
                if eof.last.dominant {
                    self.overload(eof.last).await;
                    return Ok(State::Delimiter);
                }
                return Ok(State::Intermission);
            }
            Err(Halt::End(reason, at)) => (reason, at, None),
            Err(Halt::Stop(stop)) => (EndReason::Aborted, Provenance::at(stop.tick()), Some(stop)),
            Err(Halt::EndThenStop(reason, at, stop)) => (reason, at, Some(stop)),
        };
        self.flush(at.settled).await;
        self.emit(at, CanEvent::End { frame: id, reason }).await;
        if let Some(stop) = stop {
            return Err(stop);
        }
        if self.data_phase {
            // Error flags are sent at the nominal bit rate.
            self.clock = Clock::new(self.last_sample, self.sample_point, self.rate, self.nominal);
            self.bit = 1;
            self.data_phase = false;
        }
        Ok(match reason {
            EndReason::Unsupported => State::Integrate { report: false },
            _ => State::Delimiter,
        })
    }

    /// The fields of a frame, from the identifier through the end of frame.
    async fn body(&mut self, frame: &mut FrameState) -> Result<Eof, Halt> {
        let id = frame.id;
        let base = self.field(frame, 11).await?;
        let mut rtr = self.stuffed(frame).await?;
        let ide = self.stuffed(frame).await?;
        let extended = !ide.dominant;
        if extended {
            let extension = self.field(frame, 18).await?;
            let at = Provenance::over(
                TickRange::new(base.start, extension.last.end),
                extension.last.sample,
            );
            let event = CanEvent::Id {
                frame: id,
                id: base.value << 18 | extension.value,
                extended: true,
            };
            self.emit(at, event).await;
            rtr = self.stuffed(frame).await?;
        } else {
            // A base identifier is decided by IDE, which says it is not extended.
            let at = Provenance::over(TickRange::new(base.start, base.last.end), ide.sample);
            let event = CanEvent::Id {
                frame: id,
                id: base.value,
                extended: false,
            };
            self.emit(at, event).await;
        }
        let fdf = self.stuffed(frame).await?;
        let fd = self.config.fd && !fdf.dominant;
        let (mut brs, mut esi) = (false, false);
        if fd {
            let res = self.stuffed(frame).await?;
            if !res.dominant {
                return Err(Halt::End(EndReason::Unsupported, res.provenance()));
            }
            let mut switch = self.stuffed(frame).await?;
            brs = !switch.dominant;
            if brs {
                let Some(data) = self.config.fd_bitrate else {
                    return Err(Halt::End(EndReason::Unsupported, switch.provenance()));
                };
                self.switch_rate(&mut switch, u64::from(data), true);
            }
            esi = !self.stuffed(frame).await?.dominant;
        } else if extended {
            // r0
            self.stuffed(frame).await?;
        }
        let dlc_field = self.field(frame, 4).await?;
        let dlc = (dlc_field.value & 0xf) as u8;
        let remote = !fd && !rtr.dominant;
        let length = data_length(dlc, fd);
        let at = Provenance::over(
            TickRange::new(rtr.start, dlc_field.last.end),
            dlc_field.last.sample,
        );
        let control = CanEvent::Control {
            frame: id,
            remote,
            fd,
            brs,
            esi,
            dlc,
            length,
        };
        self.emit(at, control).await;
        if !remote {
            for index in 0..length {
                let byte = self.field(frame, 8).await?;
                self.data_byte(id, index, byte, index + 1 == length).await;
            }
        }
        frame.crc_on = false;
        if fd {
            self.fd_crc(frame, length).await?;
        } else {
            self.classic_crc(frame).await?;
        }
        let mut delimiter = self.frame_bit(frame).await?;
        if self.data_phase {
            self.switch_rate(&mut delimiter, self.nominal, false);
        }
        if delimiter.dominant {
            return Err(self.violation(frame, EndReason::FormError, delimiter).await);
        }
        self.ack(frame, fd).await?;
        let first = self.frame_bit(frame).await?;
        if first.dominant {
            return Err(self.violation(frame, EndReason::FormError, first).await);
        }
        for _ in 2..=6 {
            let bit = self.frame_bit(frame).await?;
            if bit.dominant {
                return Err(self.violation(frame, EndReason::FormError, bit).await);
            }
        }
        let last = self.frame_bit(frame).await?;
        Ok(Eof { first, last })
    }

    /// Reads a bit of the frame and watches for six consecutive dominant bits. Progress stops
    /// at the frame's start until it ends.
    async fn frame_bit(&mut self, frame: &mut FrameState) -> Result<Bit, Halt> {
        let bit = self.raw_bit(Some(frame.start)).await?;
        frame.prev = bit.value();
        if bit.dominant {
            if frame.run == 0 {
                frame.run_start = bit.start;
            }
            frame.run += 1;
            if frame.run >= 6 {
                let at = Provenance::over(TickRange::new(frame.run_start, bit.end), bit.sample);
                return Err(Halt::End(EndReason::ErrorFrame, at));
            }
        } else {
            frame.run = 0;
        }
        Ok(bit)
    }

    /// Reads and removes the stuff bit that five equal bits call for, if they do.
    async fn stuff_if_due(&mut self, frame: &mut FrameState) -> Result<(), Halt> {
        if frame.same < 5 {
            return Ok(());
        }
        let bit = self.frame_bit(frame).await?;
        if bit.value() == frame.last {
            // A sixth equal bit; six dominant ones were already taken for an error flag.
            return Err(Halt::End(EndReason::StuffError, bit.provenance()));
        }
        frame.stuff_bits += 1;
        if frame.crc_on {
            frame.crc17.push(bit.value());
            frame.crc21.push(bit.value());
        }
        frame.last = bit.value();
        frame.same = 1;
        Ok(())
    }

    /// Reads the next bit of a dynamically stuffed field.
    async fn stuffed(&mut self, frame: &mut FrameState) -> Result<Bit, Halt> {
        self.stuff_if_due(frame).await?;
        let bit = self.frame_bit(frame).await?;
        if bit.value() == frame.last {
            frame.same += 1;
        } else {
            frame.last = bit.value();
            frame.same = 1;
        }
        if frame.crc_on {
            frame.crc15.push(bit.value());
            frame.crc17.push(bit.value());
            frame.crc21.push(bit.value());
        }
        Ok(bit)
    }

    /// Reads `bits` bits of a dynamically stuffed field, most significant first.
    async fn field(&mut self, frame: &mut FrameState, bits: u32) -> Result<Field, Halt> {
        let first = self.stuffed(frame).await?;
        let mut field = Field {
            value: u32::from(first.value()),
            start: first.start,
            last: first,
        };
        for _ in 1..bits {
            let bit = self.stuffed(frame).await?;
            field.value = field.value << 1 | u32::from(bit.value());
            field.last = bit;
        }
        Ok(field)
    }

    /// Adds a data byte to the batch, which goes out once it is full or the data field ends.
    async fn data_byte(&mut self, frame: FrameId, index: u8, byte: Field, last: bool) {
        let at = byte.provenance();
        match self.batch.at {
            Some(have) if !self.batch.bytes.is_empty() => self.batch.at = Some(have.join(at)),
            _ => {
                self.batch.frame = frame;
                self.batch.offset = u32::from(index);
                self.batch.at = Some(at);
            }
        }
        self.batch.bytes.push((byte.value & 0xff) as u8);
        if last || self.batch.bytes.len() >= DATA_BATCH {
            self.flush(at.settled).await;
        }
    }

    /// Sends the batch, if any. A batch is settled when it is decided: by its last byte, or by
    /// the end that cut the data field short.
    async fn flush(&mut self, decided: Tick) {
        if self.batch.bytes.is_empty() {
            return;
        }
        let bytes = std::mem::replace(&mut self.batch.bytes, Vec::with_capacity(DATA_BATCH));
        let mut at = self.batch.at.take().unwrap_or(Provenance::at(decided));
        at.settled = at.settled.max(decided);
        let event = CanEvent::Data {
            frame: self.batch.frame,
            offset: self.batch.offset,
            bytes,
        };
        self.emit(at, event).await;
    }

    /// The CRC sequence of a classic frame, and the stuff bit that may follow it.
    async fn classic_crc(&mut self, frame: &mut FrameState) -> Result<(), Halt> {
        let expected = frame.crc15.value();
        let crc = self.field(frame, 15).await?;
        let event = CanEvent::Crc {
            frame: frame.id,
            value: crc.value,
            ok: crc.value == expected,
            stuff_count: None,
        };
        self.emit(crc.provenance(), event).await;
        self.stuff_if_due(frame).await
    }

    /// The CRC field of an FD frame: the stuff count, its parity and the CRC sequence, with
    /// fixed stuff bits.
    async fn fd_crc(&mut self, frame: &mut FrameState, length: u8) -> Result<(), Halt> {
        let (mut crc, width) = if length > 16 {
            (frame.crc21, 21u32)
        } else {
            (frame.crc17, 17u32)
        };
        let first = self.fixed_stuff(frame).await?;
        let (mut gray, mut parity, mut value) = (0u8, false, 0u32);
        let mut last = first;
        for index in 0..4 + width {
            if index > 0 && index % 4 == 0 {
                self.fixed_stuff(frame).await?;
            }
            let bit = self.frame_bit(frame).await?;
            match index {
                0..=2 => {
                    gray = gray << 1 | u8::from(bit.value());
                    crc.push(bit.value());
                }
                3 => {
                    parity = bit.value();
                    crc.push(bit.value());
                }
                _ => value = value << 1 | u32::from(bit.value()),
            }
            last = bit;
        }
        let count = gray_decode(gray);
        let ok = value == crc.value()
            && parity == (gray.count_ones() % 2 == 1)
            && u32::from(count) == frame.stuff_bits % 8;
        let at = Provenance::over(TickRange::new(first.start, last.end), last.sample);
        let event = CanEvent::Crc {
            frame: frame.id,
            value,
            ok,
            stuff_count: Some(count),
        };
        self.emit(at, event).await;
        Ok(())
    }

    /// Reads a fixed stuff bit, which must be the inverse of the bit before it.
    async fn fixed_stuff(&mut self, frame: &mut FrameState) -> Result<Bit, Halt> {
        let before = frame.prev;
        let bit = self.frame_bit(frame).await?;
        if bit.value() == before {
            return Err(self.violation(frame, EndReason::StuffError, bit).await);
        }
        Ok(bit)
    }

    /// The ACK slot and the ACK delimiter.
    async fn ack(&mut self, frame: &mut FrameState, fd: bool) -> Result<(), Halt> {
        let id = frame.id;
        let slot = self.frame_bit(frame).await?;
        if slot.dominant {
            let event = CanEvent::Ack {
                frame: id,
                acked: true,
            };
            self.emit(slot.provenance(), event).await;
            return self.ack_delimiter(frame, fd).await;
        }
        if fd {
            // FD receivers may acknowledge a bit late, after a CRC delimiter two bits long.
            let next = self.frame_bit(frame).await?;
            if next.dominant {
                let event = CanEvent::Ack {
                    frame: id,
                    acked: true,
                };
                self.emit(next.provenance(), event).await;
                return self.ack_delimiter(frame, fd).await;
            }
            // Not acknowledged; `next` was the ACK delimiter.
            let at = Provenance::over(TickRange::new(slot.start, slot.end), next.sample);
            let event = CanEvent::Ack {
                frame: id,
                acked: false,
            };
            self.emit(at, event).await;
            return Ok(());
        }
        let event = CanEvent::Ack {
            frame: id,
            acked: false,
        };
        self.emit(slot.provenance(), event).await;
        let delimiter = self.frame_bit(frame).await?;
        if delimiter.dominant {
            return Err(self.violation(frame, EndReason::FormError, delimiter).await);
        }
        Ok(())
    }

    /// The ACK delimiter after a dominant ACK bit; FD frames accept a second dominant bit of
    /// overlapping acknowledgements before it.
    async fn ack_delimiter(&mut self, frame: &mut FrameState, fd: bool) -> Result<(), Halt> {
        let mut delimiter = self.frame_bit(frame).await?;
        if delimiter.dominant && fd {
            delimiter = self.frame_bit(frame).await?;
        }
        if delimiter.dominant {
            return Err(self.violation(frame, EndReason::FormError, delimiter).await);
        }
        Ok(())
    }

    /// Ends the frame for `reason` at `bit`, the bit that broke its form. A dominant bit may
    /// be the start of an error flag: the decision waits until the line turns recessive, and
    /// six dominant bits make it an error frame.
    async fn violation(&mut self, frame: &mut FrameState, reason: EndReason, bit: Bit) -> Halt {
        let span = TickRange::new(bit.start, bit.end);
        if !bit.dominant {
            return Halt::End(reason, Provenance::over(span, bit.sample));
        }
        loop {
            match self.frame_bit(frame).await {
                Ok(next) if next.dominant => {}
                Ok(next) => return Halt::End(reason, Provenance::over(span, next.sample)),
                Err(Halt::Stop(stop)) => {
                    return Halt::EndThenStop(reason, Provenance::over(span, stop.tick()), stop);
                }
                Err(halt) => return halt,
            }
        }
    }
}

async fn run(signal: SignalCursor, out: Outbox<CanEvent>, config: CanConfig, time_base: TimeBase) {
    let mut decoder = Decoder::new(signal, out, config, time_base);
    let mut state = State::Integrate { report: true };
    loop {
        let next = match state {
            State::Integrate { report } => decoder.integrate(report).await,
            State::Idle => decoder.wait_for_sof().await.map(State::Frame),
            State::Frame(sof) => decoder.frame(sof).await,
            State::Intermission => decoder.intermission().await,
            State::Delimiter => decoder.delimiter().await,
        };
        state = match next {
            Ok(state) => state,
            Err(Stop::Gap(range)) => {
                decoder.out.gap(range).await;
                State::Integrate { report: true }
            }
            Err(Stop::End(tick)) => {
                decoder.out.end(tick).await;
                return;
            }
        };
    }
}

/// Passes gaps, progress and the end on without decoding.
async fn pass_through(mut signal: SignalCursor, mut out: Outbox<CanEvent>) {
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
        assert_eq!(CanConfig::default().validate(), Ok(()));
        let bad = |config: CanConfig| config.validate().expect_err("rejected");
        assert_eq!(
            bad(CanConfig {
                rx: 64,
                ..CanConfig::default()
            }),
            CanConfigError::Channel(64)
        );
        assert_eq!(
            bad(CanConfig {
                bitrate: 0,
                ..CanConfig::default()
            }),
            CanConfigError::Bitrate
        );
        assert_eq!(
            bad(CanConfig {
                fd_bitrate: Some(250_000),
                ..CanConfig::default()
            }),
            CanConfigError::FdBitrate {
                nominal: 500_000,
                data: 250_000
            }
        );
        assert_eq!(
            bad(CanConfig {
                sample_point: 0,
                ..CanConfig::default()
            }),
            CanConfigError::SamplePoint(0)
        );
        assert_eq!(
            bad(CanConfig {
                sample_point: 100,
                ..CanConfig::default()
            }),
            CanConfigError::SamplePoint(100)
        );
    }

    #[test]
    fn props_describe_the_bus() {
        let props = CanConfig {
            fd_bitrate: Some(2_000_000),
            ..CanConfig::default()
        }
        .props();
        assert_eq!(props.get("bitrate"), Some(&500_000i64.into()));
        assert_eq!(props.get("fdBitrate"), Some(&2_000_000i64.into()));
        assert_eq!(props.get("samplePoint"), Some(&87i64.into()));
        assert_eq!(props.get("fd"), Some(&true.into()));
        let classic = CanConfig {
            fd: false,
            fd_bitrate: Some(2_000_000),
            ..CanConfig::default()
        };
        assert_eq!(classic.props().get("fdBitrate"), None);
        assert_eq!(classic.min_rate_hz(), 2_000_000);
    }

    #[test]
    fn the_input_requires_the_channel_and_enough_samples_per_bit() {
        let can = Can::new(CanConfig {
            rx: 3,
            fd_bitrate: Some(5_000_000),
            ..CanConfig::default()
        })
        .expect("valid");
        let requires = &can.spec().inputs[0].requires;
        let source =
            |channels: i64, rate: i64| Props::new().with("channels", channels).with("rateHz", rate);
        assert!(requires.unmet(&source(4, 20_000_000)).is_none());
        assert_eq!(
            requires.unmet(&source(3, 20_000_000)).map(|(key, _)| key),
            Some("channels")
        );
        assert_eq!(
            requires.unmet(&source(4, 19_999_999)).map(|(key, _)| key),
            Some("rateHz")
        );
    }

    #[test]
    fn dlc_codes_stand_for_lengths() {
        let classic: Vec<u8> = (0..16).map(|dlc| data_length(dlc, false)).collect();
        assert_eq!(classic, [0, 1, 2, 3, 4, 5, 6, 7, 8, 8, 8, 8, 8, 8, 8, 8]);
        let fd: Vec<u8> = (0..16).map(|dlc| data_length(dlc, true)).collect();
        assert_eq!(fd, [0, 1, 2, 3, 4, 5, 6, 7, 8, 12, 16, 20, 24, 32, 48, 64]);
    }

    #[test]
    fn gray_codes_decode() {
        let codes = [0b000, 0b001, 0b011, 0b010, 0b110, 0b111, 0b101, 0b100];
        for (count, code) in codes.iter().enumerate() {
            assert_eq!(usize::from(gray_decode(*code)), count);
        }
    }

    #[test]
    fn levels_follow_the_polarity() {
        let low = CanConfig::default();
        assert!(low.dominant(Levels(0)));
        assert!(!low.dominant(Levels(1)));
        let high = CanConfig {
            rx: 2,
            dominant_low: false,
            ..CanConfig::default()
        };
        assert!(high.dominant(Levels(0b100)));
        assert!(!high.dominant(Levels(0b011)));
    }
}
