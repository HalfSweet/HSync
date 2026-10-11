// SPDX-License-Identifier: Apache-2.0

//! The NEC decoder: the frames and repeat codes of an NEC infrared remote control, read from the
//! demodulated output of an infrared receiver.
//!
//! Implemented from the NEC transmission format as its users publish it: Vishay's application
//! note "Data Formats for IR Remote Control" (document 80071), Altium's application note "NEC
//! Infrared Transmission Protocol" and the SB-Projects description of the NEC protocol. The
//! carrier is 38 kHz, the coding pulse distance, and every duration a multiple of 562.5 µs. The
//! rules:
//!
//! - **Levels.** A receiver's output is at mark while it receives the carrier and at space
//!   otherwise. Mark is low (`active-low`, as receiver modules put it out) or high
//!   (`active-high`); the idle line is at space.
//! - **Durations.** Marks and spaces are measured in ticks, from the edge that begins one to the
//!   edge that ends it, and compared with a window around their nominal length `d`: from
//!   ⌈`d · rate · (100 − tolerance) / 100`⌉ to ⌊`d · rate · (100 + tolerance) / 100`⌋ ticks,
//!   `rate` being the source's samples per second, computed exactly in integer arithmetic.
//!   Where the windows of two lengths the decoder may meet at one point overlap (at tolerances
//!   above 33%), a duration in both belongs to the length it is nearer to, the shorter one when
//!   it is as near to both.
//! - **Leader.** A frame begins with a 9 ms mark. A shorter mark on the idle line is no leader
//!   and is passed over; a longer one is an error (`leader`) as soon as it has outlasted its
//!   window. A 4.5 ms space after the leader mark begins a frame, a 2.25 ms space a repeat code,
//!   any other space is an error (`leader`).
//! - **Frame.** 32 data bits follow, least significant bit first, each a 562.5 µs mark and a
//!   space of 562.5 µs for a 0 or 1687.5 µs for a 1, then a final 562.5 µs mark. The bytes are
//!   the address, its inverse, the command and its inverse; a second byte that is not the
//!   inverse of the first makes, with it, a 16-bit address, the first byte the low one
//!   (extended NEC). The fourth byte is reported as received: `inverseOk` says whether it is the
//!   inverse of the command, and `raw` holds all 32 bits.
//! - **Repeat code.** The leader mark, a 2.25 ms space and a 562.5 µs mark, sent every 108 ms
//!   while a key stays pressed. It is reported on its own: neither its timing nor the frame
//!   before it is checked.
//! - **Errors.** A mark or space outside its window ends what the leader began as an error
//!   naming where: `leader`; `bit`, a data bit's mark, or a space shorter than a 0's or between
//!   a 0's and a 1's; `count`, a space longer than a 1's before the 32nd bit, the frame having
//!   stopped short; `trailer`, the final mark. The error carries the bits read before it.
//! - **Decisions.** A frame or a repeat code is decided at the end of its final mark, an error
//!   where it became certain: at the edge that ended an element too soon or inside the windows
//!   of neither of its lengths, or at the last tick of an element that outlasted its window.
//!   Nothing after the final mark is looked at.
//! - **Provenance.** Every event spans from the first tick of its leader mark: a frame or a
//!   repeat code to its final mark's end, where it is settled; an error to where it was
//!   decided, settled there — an error decided by an element that outlasted its window covers
//!   that element's last tick too.
//! - **Alignment.** At the first sample, after a gap and after every frame, repeat code or
//!   error, the decoder waits for the line to idle — to stay at space longer than the longest
//!   space a frame has, the 4.5 ms leader space with its tolerance (5.625 ms at 25%) — before it
//!   takes a mark for a leader. A frame already under way when the samples begin or resume
//!   cannot be aligned and is passed over, and so is whatever follows a final mark until the
//!   line idles.
//! - **Gaps and the end of the samples.** A frame or repeat code cut off by a gap or by the end
//!   of the samples is not reported. Progress is passed on, but never past the start of a leader
//!   mark whose frame is still being read; before a gap or the end goes out, progress is
//!   announced up to its start, or up to the start of the frame it cut off.
//! - **Rate.** The input requires a source at which the tolerance of 562.5 µs spans a tick:
//!   ⌈100 / (tolerance · 562.5 µs)⌉ samples per second, 7112 at 25%, or sampling alone could
//!   push a duration out of its window. Should a decoder start on a slower source all the same,
//!   it reports nothing and only passes gaps, progress and the end on.
//!
//! Limits: one receiver output, demodulated (the 38 kHz bursts themselves are not read);
//! repeat codes are not tied to the frame before them; frames longer than 32 bits are not told
//! apart from NEC frames.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use uiharu_core::{
    Change, ChannelMask, InputRef, InputSpec, Io, Levels, LogicBlock, MAX_CHANNELS, Node,
    NodeFuture, NodeId, NodeSpec, Outbox, OutputRef, OutputSpec, Props, Provenance, Requires,
    Sample, SignalCursor, Tick, TickRange,
};

use super::events::{ErrorReason, NecEvent};

/// The largest tolerance a configuration may ask for, in percent.
pub const MAX_TOLERANCE: u8 = 50;

/// The unit every NEC duration is a multiple of, 562.5 µs, as the fraction of a second
/// `UNIT.0 / UNIT.1`.
const UNIT: (u64, u64) = (9, 16_000);
/// The leader mark, 9 ms, in units.
const LEADER_MARK: u64 = 16;
/// The leader space of a frame, 4.5 ms.
const FRAME_SPACE: u64 = 8;
/// The leader space of a repeat code, 2.25 ms.
const REPEAT_SPACE: u64 = 4;
/// A data bit's mark, a 0's space and the final mark, 562.5 µs.
const SHORT: u64 = 1;
/// A 1's space, 1687.5 µs.
const ONE_SPACE: u64 = 3;
/// The data bits of a frame.
const DATA_BITS: u8 = 32;

/// Which level of the receiver's output is a mark, the carrier being received.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Polarity {
    /// A mark is low, as receiver modules put it out.
    #[default]
    ActiveLow,
    /// A mark is high.
    ActiveHigh,
}

impl Polarity {
    /// The name configurations use.
    pub const fn name(self) -> &'static str {
        match self {
            Polarity::ActiveLow => "active-low",
            Polarity::ActiveHigh => "active-high",
        }
    }

    /// Whether a line at `high` is at mark.
    pub const fn is_mark(self, high: bool) -> bool {
        high == matches!(self, Polarity::ActiveHigh)
    }
}

/// Where the receiver is connected and how strictly durations are judged.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct NecConfig {
    /// The channel of the receiver's output.
    pub channel: u8,
    /// Which level is a mark.
    pub polarity: Polarity,
    /// How far a mark or space may be from its nominal length, in percent of it, 1 to
    /// [`MAX_TOLERANCE`].
    pub tolerance: u8,
}

impl Default for NecConfig {
    fn default() -> Self {
        Self {
            channel: 0,
            polarity: Polarity::ActiveLow,
            tolerance: 25,
        }
    }
}

impl NecConfig {
    /// Checks the configuration.
    pub fn validate(&self) -> Result<(), NecConfigError> {
        if self.channel >= MAX_CHANNELS {
            return Err(NecConfigError::Channel(self.channel));
        }
        if !(1..=MAX_TOLERANCE).contains(&self.tolerance) {
            return Err(NecConfigError::Tolerance(self.tolerance));
        }
        Ok(())
    }

    /// What the decoder's output port declares.
    pub fn props(&self) -> Props {
        Props::new().with("protocol", "nec")
    }

    /// The fewest samples per second the decoder reads: the rate at which the tolerance of the
    /// shortest duration, 562.5 µs, spans one tick.
    pub fn min_rate_hz(&self) -> u64 {
        let tolerance = u64::from(self.tolerance.max(1));
        (100 * UNIT.1).div_ceil(tolerance * UNIT.0)
    }

    /// What the decoder's input requires of the source: the channel, and a fine enough rate.
    pub fn requires(&self) -> Requires {
        Requires::new()
            .at_least("channels", i64::from(self.channel) + 1)
            .at_least(
                "rateHz",
                i64::try_from(self.min_rate_hz()).unwrap_or(i64::MAX),
            )
    }
}

/// Why a configuration is rejected.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum NecConfigError {
    /// The channel number is out of range.
    #[error("channel {0} is out of range; channels are 0 to {last}", last = MAX_CHANNELS - 1)]
    Channel(u8),
    /// The tolerance is not 1% to [`MAX_TOLERANCE`].
    #[error("a tolerance of {0}%; 1% to {MAX_TOLERANCE}% are supported")]
    Tolerance(u8),
}

/// The NEC decoder node: logic samples in, `ir_nec.events` out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Nec {
    config: NecConfig,
}

impl Nec {
    /// A decoder for `config`.
    pub fn new(config: NecConfig) -> Result<Self, NecConfigError> {
        config.validate()?;
        Ok(Self { config })
    }

    /// The configuration.
    pub fn config(&self) -> &NecConfig {
        &self.config
    }

    /// The logic input of the decoder added as `node`.
    pub fn input(node: NodeId) -> InputRef<Arc<LogicBlock>> {
        InputRef::of_node(node, 0)
    }

    /// The event output of the decoder added as `node`.
    pub fn events(node: NodeId) -> OutputRef<NecEvent> {
        OutputRef::of_node(node, 0)
    }
}

impl Node for Nec {
    fn spec(&self) -> NodeSpec {
        NodeSpec {
            name: "ir-nec".into(),
            inputs: vec![InputSpec::new::<Arc<LogicBlock>>(
                "logic",
                self.config.requires(),
            )],
            outputs: vec![OutputSpec::new::<NecEvent>("events", self.config.props())],
        }
    }

    fn start(self: Box<Self>, mut io: Io) -> NodeFuture {
        let rate = io.time_base().map_or(0, |base| base.rate_hz);
        let (signal, out) = (io.signal(0), io.output(0));
        if rate < self.config.min_rate_hz() {
            return Box::pin(without_frames(signal, out));
        }
        Box::pin(run(Decoder::new(signal, out, self.config, rate)))
    }
}

/// A window of lengths in ticks, both bounds included.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Window {
    min: u64,
    max: u64,
}

impl Window {
    fn holds(self, ticks: u64) -> bool {
        (self.min..=self.max).contains(&ticks)
    }
}

/// NEC lengths in ticks at the source's rate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Timing {
    rate: u64,
    tolerance: u64,
}

impl Timing {
    /// `units` units in ticks, multiplied by the unit's denominator so that it stays exact.
    fn scaled(&self, units: u64) -> u128 {
        u128::from(self.rate) * u128::from(units) * u128::from(UNIT.0)
    }

    /// The window of a length of `units` units.
    fn window(&self, units: u64) -> Window {
        let scaled = self.scaled(units);
        let den = u128::from(UNIT.1) * 100;
        Window {
            min: saturate((scaled * u128::from(100 - self.tolerance)).div_ceil(den)),
            max: saturate(scaled * u128::from(100 + self.tolerance) / den),
        }
    }

    /// Which of two lengths, in units, the shorter first, `ticks` is: the one whose window holds
    /// it, or where both do, the one it is nearer to, the shorter one when it is as near to both.
    fn classify(&self, ticks: u64, lengths: [u64; 2]) -> Option<u64> {
        let mut best: Option<(u64, u128)> = None;
        for units in lengths {
            if !self.window(units).holds(ticks) {
                continue;
            }
            let distance = (u128::from(ticks) * u128::from(UNIT.1)).abs_diff(self.scaled(units));
            if best.is_none_or(|(_, nearest)| distance < nearest) {
                best = Some((units, distance));
            }
        }
        best.map(|(units, _)| units)
    }
}

fn saturate(ticks: u128) -> u64 {
    u64::try_from(ticks).unwrap_or(u64::MAX)
}

/// The windows the decoder compares with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Windows {
    leader_mark: Window,
    frame_space: Window,
    /// A data bit's mark and the final mark.
    short: Window,
    one_space: Window,
    /// The longest space a frame has: the line idles once a space outlasts it.
    idle: u64,
}

impl Windows {
    fn new(timing: &Timing) -> Self {
        let frame_space = timing.window(FRAME_SPACE);
        Self {
            leader_mark: timing.window(LEADER_MARK),
            frame_space,
            short: timing.window(SHORT),
            one_space: timing.window(ONE_SPACE),
            idle: frame_space.max,
        }
    }
}

/// What the decoder knows of the line where it stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Line {
    /// Nothing: at the first sample and after a gap.
    Unknown,
    /// At mark.
    Mark,
    /// At space since this tick.
    Space(Tick),
}

/// Why the samples stopped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stop {
    Gap(TickRange),
    End(Tick),
}

/// How an element of a frame ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Element {
    /// At this edge.
    Ended(Tick),
    /// Not within the longest it may last: the line was still at its level at this tick.
    Outlasted(Tick),
}

/// Why a mark on the idle line did not give a frame or a repeat code.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Halt {
    /// The samples stopped.
    Stop(Stop),
    /// The mark was too short for a leader.
    NoLeader,
    /// An element outside its window: the error, and where the line stands after it.
    Failed(Provenance, NecEvent, Line),
}

impl From<Stop> for Halt {
    fn from(stop: Stop) -> Self {
        Halt::Stop(stop)
    }
}

/// What a leader began: where, and the data bits read so far.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Reading {
    start: Tick,
    count: u8,
    raw: u32,
}

impl Reading {
    fn push(&mut self, one: bool) {
        self.raw |= u32::from(one) << self.count;
        self.count += 1;
    }

    fn error(&self, reason: ErrorReason) -> NecEvent {
        NecEvent::Error {
            reason,
            bits: self.count,
            raw: self.raw,
        }
    }

    /// The error decided at `edge`, where an element ended too soon or outside its windows.
    fn failed_at(&self, edge: Tick, reason: ErrorReason, line: Line) -> Halt {
        let at = Provenance::over(TickRange::new(self.start, edge), edge);
        Halt::Failed(at, self.error(reason), line)
    }

    /// The error decided at `last`, the last tick of an element that outlasted its window.
    fn failed_after(&self, last: Tick, reason: ErrorReason, line: Line) -> Halt {
        let at = Provenance::over(TickRange::new(self.start, last.next()), last);
        Halt::Failed(at, self.error(reason), line)
    }
}

/// The frame of the 32 bits `raw`.
fn frame(raw: u32) -> NecEvent {
    let [first, second, command, check] = raw.to_le_bytes();
    let extended = second != !first;
    NecEvent::Frame {
        address: if extended {
            u16::from_le_bytes([first, second])
        } else {
            u16::from(first)
        },
        command,
        extended,
        inverse_ok: check == !command,
        raw,
    }
}

struct Decoder {
    signal: SignalCursor,
    out: Outbox<NecEvent>,
    polarity: Polarity,
    channel: u8,
    watch: ChannelMask,
    timing: Timing,
    windows: Windows,
    /// The latest progress passed on.
    promised: Option<Tick>,
}

impl Decoder {
    fn new(signal: SignalCursor, out: Outbox<NecEvent>, config: NecConfig, rate: u64) -> Self {
        let timing = Timing {
            rate,
            tolerance: u64::from(config.tolerance),
        };
        Self {
            signal,
            out,
            polarity: config.polarity,
            channel: config.channel,
            watch: ChannelMask::one(config.channel),
            windows: Windows::new(&timing),
            timing,
            promised: None,
        }
    }

    fn is_mark(&self, levels: Levels) -> bool {
        self.polarity.is_mark(levels.high(self.channel))
    }

    /// Passes progress on, unless it promises no more than what was passed on already.
    async fn progress(&mut self, tick: Tick) {
        if self.promised.is_none_or(|promised| tick > promised) {
            self.promised = Some(tick);
            self.out.progress(tick).await;
        }
    }

    /// Passes a gap or the end of the samples on, after progress up to it, or up to `under_way`,
    /// the start of a frame it cut off. Returns whether the stream has ended.
    async fn pass_on(&mut self, stop: Stop, under_way: Option<Tick>) -> bool {
        let (at, ended) = match stop {
            Stop::Gap(range) => (range.start, false),
            Stop::End(tick) => (tick, true),
        };
        self.progress(under_way.map_or(at, |start| start.min(at)))
            .await;
        match stop {
            Stop::Gap(range) => self.out.gap(range).await,
            Stop::End(tick) => self.out.end(tick).await,
        }
        ended
    }

    /// The level at the cursor, where no level before it is known: the tick and whether it is
    /// at mark.
    async fn sample(&mut self) -> Result<(Tick, bool), Stop> {
        loop {
            match self.signal.sample().await {
                Sample::At { tick, levels } => return Ok((tick, self.is_mark(levels))),
                Sample::Progress(tick) => self.progress(tick).await,
                Sample::Gap(range) => return Err(Stop::Gap(range)),
                Sample::End(tick) => return Err(Stop::End(tick)),
            }
        }
    }

    /// Waits for the next edge, with no frame under way.
    async fn edge(&mut self) -> Result<Tick, Stop> {
        loop {
            match self.signal.next_change(self.watch).await {
                Change::At { tick, .. } => return Ok(tick),
                // Without a deadline the wait is never quiet.
                Change::Quiet(_) => {}
                Change::Progress(tick) => self.progress(tick).await,
                Change::Gap(range) => return Err(Stop::Gap(range)),
                Change::End(tick) => return Err(Stop::End(tick)),
            }
        }
    }

    /// Waits for the edge that ends an element begun at `from`, for `max` ticks of it at most.
    /// Progress is held at `under_way`, the start of the frame being read, if any.
    async fn element(
        &mut self,
        from: Tick,
        max: u64,
        under_way: Option<Tick>,
    ) -> Result<Element, Stop> {
        let last = from.offset(max);
        loop {
            match self
                .signal
                .next_change_before(self.watch, last.next())
                .await
            {
                Change::At { tick, .. } => return Ok(Element::Ended(tick)),
                Change::Quiet(_) => return Ok(Element::Outlasted(last)),
                Change::Progress(tick) => {
                    let held = under_way.map_or(tick, |start| start.min(tick));
                    self.progress(held).await;
                }
                Change::Gap(range) => return Err(Stop::Gap(range)),
                Change::End(tick) => return Err(Stop::End(tick)),
            }
        }
    }

    /// Reads a mark of the frame `reading`, begun at `from`, that must fit `window`: its end.
    async fn mark(
        &mut self,
        reading: &Reading,
        from: Tick,
        window: Window,
        reason: ErrorReason,
    ) -> Result<Tick, Halt> {
        match self.element(from, window.max, Some(reading.start)).await? {
            Element::Ended(end) if from.until(end) < window.min => {
                Err(reading.failed_at(end, reason, Line::Space(end)))
            }
            Element::Ended(end) => Ok(end),
            Element::Outlasted(last) => Err(reading.failed_after(last, reason, Line::Mark)),
        }
    }

    /// Reads a space of the frame `reading`, begun at `from`, that lasts `max` ticks at most:
    /// its end. Which of its lengths it has is the caller's to judge.
    async fn space(
        &mut self,
        reading: &Reading,
        from: Tick,
        max: u64,
        reason: ErrorReason,
    ) -> Result<Tick, Halt> {
        match self.element(from, max, Some(reading.start)).await? {
            Element::Ended(end) => Ok(end),
            Element::Outlasted(last) => Err(reading.failed_after(last, reason, Line::Space(from))),
        }
    }

    /// Waits for the line to idle: to stay at space longer than any space of a frame.
    async fn idle(&mut self, mut line: Line) -> Result<(), Stop> {
        let idle = self.windows.idle;
        loop {
            line = match line {
                Line::Unknown => match self.sample().await? {
                    (_, true) => Line::Mark,
                    (tick, false) => Line::Space(tick),
                },
                Line::Mark => Line::Space(self.edge().await?),
                Line::Space(since) => match self.element(since, idle, None).await? {
                    Element::Outlasted(_) => return Ok(()),
                    Element::Ended(_) => Line::Mark,
                },
            };
        }
    }

    /// Reads what the mark that began at `start` on the idle line begins: a frame or a repeat
    /// code, and where the line stands after it.
    async fn read(&mut self, start: Tick) -> Result<(Provenance, NecEvent, Line), Halt> {
        let windows = self.windows;
        let mut reading = Reading {
            start,
            count: 0,
            raw: 0,
        };
        let leader = windows.leader_mark;
        let mark_end = match self.element(start, leader.max, Some(start)).await? {
            Element::Ended(end) if start.until(end) < leader.min => return Err(Halt::NoLeader),
            Element::Ended(end) => end,
            Element::Outlasted(last) => {
                return Err(reading.failed_after(last, ErrorReason::Leader, Line::Mark));
            }
        };
        let max = windows.frame_space.max;
        let space_end = self
            .space(&reading, mark_end, max, ErrorReason::Leader)
            .await?;
        let space = mark_end.until(space_end);
        let repeat = match self.timing.classify(space, [REPEAT_SPACE, FRAME_SPACE]) {
            Some(units) => units == REPEAT_SPACE,
            None => return Err(reading.failed_at(space_end, ErrorReason::Leader, Line::Mark)),
        };
        let mut mark_start = space_end;
        while !repeat && reading.count < DATA_BITS {
            let mark_end = self
                .mark(&reading, mark_start, windows.short, ErrorReason::Bit)
                .await?;
            let max = windows.one_space.max;
            let space_end = self
                .space(&reading, mark_end, max, ErrorReason::Count)
                .await?;
            let space = mark_end.until(space_end);
            match self.timing.classify(space, [SHORT, ONE_SPACE]) {
                Some(units) => reading.push(units == ONE_SPACE),
                None => return Err(reading.failed_at(space_end, ErrorReason::Bit, Line::Mark)),
            }
            mark_start = space_end;
        }
        let end = self
            .mark(&reading, mark_start, windows.short, ErrorReason::Trailer)
            .await?;
        let event = if repeat {
            NecEvent::Repeat
        } else {
            frame(reading.raw)
        };
        let at = Provenance::over(TickRange::new(start, end), end);
        Ok((at, event, Line::Space(end)))
    }

    /// Waits on the idle line for a mark and reads what it begins: the state after it, or the
    /// stop that came first, with the start of the frame it cut off.
    async fn armed(&mut self) -> Result<State, (Stop, Option<Tick>)> {
        let start = self.edge().await.map_err(|stop| (stop, None))?;
        match self.read(start).await {
            Ok((at, event, line)) | Err(Halt::Failed(at, event, line)) => {
                self.out.emit(at, event).await;
                Ok(State::Align(line))
            }
            Err(Halt::NoLeader) => Ok(State::Armed),
            Err(Halt::Stop(stop)) => Err((stop, Some(start))),
        }
    }
}

/// Where the decoder stands between frames.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    /// Waiting for the line to idle, from what is known of it.
    Align(Line),
    /// The line idles: the next mark may be a leader.
    Armed,
}

async fn run(mut decoder: Decoder) {
    let mut state = State::Align(Line::Unknown);
    loop {
        let next = match state {
            State::Align(line) => match decoder.idle(line).await {
                Ok(()) => Ok(State::Armed),
                Err(stop) => Err((stop, None)),
            },
            State::Armed => decoder.armed().await,
        };
        state = match next {
            Ok(state) => state,
            Err((stop, under_way)) => {
                if decoder.pass_on(stop, under_way).await {
                    return;
                }
                State::Align(Line::Unknown)
            }
        };
    }
}

/// Lets the samples pass without reading frames, on a source too slow for the windows.
async fn without_frames(mut signal: SignalCursor, mut out: Outbox<NecEvent>) {
    loop {
        match signal.next_change(ChannelMask::EMPTY).await {
            Change::At { .. } | Change::Quiet(_) => {}
            Change::Progress(tick) => out.progress(tick).await,
            Change::Gap(range) => {
                out.progress(range.start).await;
                out.gap(range).await;
            }
            Change::End(tick) => {
                out.progress(tick).await;
                out.end(tick).await;
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir_nec::IR_NEC_EVENTS;

    #[test]
    fn configurations_are_checked() {
        assert_eq!(NecConfig::default().validate(), Ok(()));
        let bad = |config: NecConfig| config.validate().expect_err("rejected");
        assert_eq!(
            bad(NecConfig {
                channel: 64,
                ..NecConfig::default()
            }),
            NecConfigError::Channel(64)
        );
        for tolerance in [0, 51, 255] {
            assert_eq!(
                bad(NecConfig {
                    tolerance,
                    ..NecConfig::default()
                }),
                NecConfigError::Tolerance(tolerance)
            );
        }
        for tolerance in [1, 50] {
            assert!(
                Nec::new(NecConfig {
                    tolerance,
                    ..NecConfig::default()
                })
                .is_ok()
            );
        }
    }

    #[test]
    fn descriptions_may_leave_fields_out() {
        let config: NecConfig =
            serde_json::from_str(r#"{"channel": 3, "polarity": "active-high"}"#).expect("parses");
        assert_eq!(
            config,
            NecConfig {
                channel: 3,
                polarity: Polarity::ActiveHigh,
                tolerance: 25,
            }
        );
        let whole: NecConfig = serde_json::from_str("{}").expect("parses");
        assert_eq!(whole, NecConfig::default());
        assert!(serde_json::from_str::<NecConfig>(r#"{"line": 0}"#).is_err());
        assert_eq!(Polarity::ActiveHigh.name(), "active-high");
        assert!(Polarity::ActiveLow.is_mark(false));
        assert!(Polarity::ActiveHigh.is_mark(true));
    }

    #[test]
    fn ports_declare_the_protocol_and_require_a_fine_enough_rate() {
        let config = NecConfig {
            channel: 2,
            ..NecConfig::default()
        };
        assert_eq!(config.props().get("protocol"), Some(&"nec".into()));
        // 562.5 µs × 25% is one tick at 7111.1 samples per second.
        assert_eq!(config.min_rate_hz(), 7112);
        let spec = Nec::new(config).expect("valid").spec();
        assert_eq!(spec.outputs[0].schema, IR_NEC_EVENTS);
        let requires = &spec.inputs[0].requires;
        let source =
            |channels: i64, rate: i64| Props::new().with("channels", channels).with("rateHz", rate);
        assert!(requires.unmet(&source(3, 7112)).is_none());
        assert_eq!(
            requires.unmet(&source(2, 7112)).map(|(key, _)| key),
            Some("channels")
        );
        assert_eq!(
            requires.unmet(&source(3, 7111)).map(|(key, _)| key),
            Some("rateHz")
        );
        let strict = NecConfig {
            tolerance: 5,
            ..NecConfig::default()
        };
        assert_eq!(strict.min_rate_hz(), 35_556);
    }

    #[test]
    fn windows_round_toward_their_nominal_length() {
        // 1 MHz: 562.5 ticks a unit.
        let timing = Timing {
            rate: 1_000_000,
            tolerance: 25,
        };
        assert_eq!(timing.window(SHORT), Window { min: 422, max: 703 });
        assert_eq!(
            timing.window(ONE_SPACE),
            Window {
                min: 1266,
                max: 2109
            }
        );
        assert_eq!(
            timing.window(LEADER_MARK),
            Window {
                min: 6750,
                max: 11_250
            }
        );
        assert_eq!(
            timing.window(REPEAT_SPACE),
            Window {
                min: 1688,
                max: 2812
            }
        );
        assert_eq!(Windows::new(&timing).idle, 5625);
        assert_eq!(timing.classify(562, [SHORT, ONE_SPACE]), Some(SHORT));
        assert_eq!(timing.classify(1000, [SHORT, ONE_SPACE]), None);
        assert_eq!(timing.classify(1700, [SHORT, ONE_SPACE]), Some(ONE_SPACE));
        // At 40% the windows of the two leader spaces overlap from 2700 to 3150 ticks, all of
        // it nearer to the repeat code's 2250 ticks than to a frame's 4500.
        let loose = Timing {
            rate: 1_000_000,
            tolerance: 40,
        };
        let spaces = [REPEAT_SPACE, FRAME_SPACE];
        assert_eq!(loose.classify(2800, spaces), Some(REPEAT_SPACE));
        assert_eq!(loose.classify(3150, spaces), Some(REPEAT_SPACE));
        assert_eq!(loose.classify(3151, spaces), Some(FRAME_SPACE));
        assert_eq!(loose.classify(2699, spaces), Some(REPEAT_SPACE));
        // At 50% 3375 ticks is as near to 2250 as to 4500: the shorter length wins.
        let widest = Timing {
            rate: 1_000_000,
            tolerance: 50,
        };
        assert_eq!(widest.classify(3375, spaces), Some(REPEAT_SPACE));
        assert_eq!(widest.classify(3376, spaces), Some(FRAME_SPACE));
    }

    #[test]
    fn frames_name_their_bytes() {
        assert_eq!(
            frame(0xba45_ff00),
            NecEvent::Frame {
                address: 0,
                command: 0x45,
                extended: false,
                inverse_ok: true,
                raw: 0xba45_ff00,
            }
        );
        assert_eq!(
            frame(0xa956_1234),
            NecEvent::Frame {
                address: 0x1234,
                command: 0x56,
                extended: true,
                inverse_ok: true,
                raw: 0xa956_1234,
            }
        );
        assert_eq!(
            frame(0x0156_e619),
            NecEvent::Frame {
                address: 0x19,
                command: 0x56,
                extended: false,
                inverse_ok: false,
                raw: 0x0156_e619,
            }
        );
    }
}
