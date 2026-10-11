// SPDX-License-Identifier: Apache-2.0

//! The RC-6 decoder: the mode 0 frames of a Philips RC-6 infrared remote control, read from the
//! demodulated output of an infrared receiver.
//!
//! Implemented from the RC-6 format as Philips published it and as the SB-Projects description
//! of RC-6 sets it out. The carrier is 36 kHz and the coding bi-phase; the unit `t` is 16
//! carrier periods, 444.4 µs. Mode 0 is the mode of consumer electronics. The rules:
//!
//! - **Levels.** A receiver's output is at mark while it receives the carrier and at space
//!   otherwise. Mark is low (`active-low`, as receiver modules put it out) or high
//!   (`active-high`); the idle line is at space.
//! - **Frame.** A leader — a mark of 6 `t` and a space of 2 `t` — then 21 bits, most
//!   significant bit first: the start bit, always 1; three mode bits; the trailer bit, which
//!   carries the toggle and lasts twice as long; eight control bits, the address; eight
//!   information bits, the command. A bit is two halves of `t`, the trailer bit's of 2 `t`.
//! - **Bits.** Opposite to RC-5, a 1 is a mark then a space and a 0 a space then a mark: every
//!   bit has an edge in its middle, whose direction is the bit, and two bits have an edge
//!   between them only when they are equal.
//! - **Edges.** The bits are read from the edges, never from a clock of the decoder's own. The
//!   leader space ends at the start of the start bit, whose mark of `t` ends at its middle. From
//!   each middle the next edge comes after the rest of the bit — an edge between two equal
//!   bits, the next middle following after the first half of the next bit — or after the rest
//!   of the bit and the first half of the next, the middle of a bit unequal to the last: one or
//!   two `t` within the frame, but one or three `t` before the trailer bit's middle and two or
//!   three after it. Every interval between two edges is judged on its own, and every middle
//!   re-anchors the timing.
//! - **Durations.** An interval of nominal length `d` is in its window from
//!   ⌈`d · rate · (100 − tolerance) / 100`⌉ to ⌊`d · rate · (100 + tolerance) / 100`⌋ ticks,
//!   `rate` being the source's samples per second, computed exactly in integer arithmetic. Where
//!   the windows of the two lengths the decoder may meet at one point overlap — two and three `t`
//!   after the trailer bit's middle from a tolerance of 20%, one and two `t` above 33% — an
//!   interval in both belongs to the length it is nearer to, the shorter one when it is as near
//!   to both.
//! - **Leader.** On the idle line a mark of 6 `t` begins a frame. A shorter mark is no leader
//!   and is passed over; a longer one is an error (`leader`) as soon as it has outlasted its
//!   window.
//! - **Mode.** Only mode 0 is read: once the three mode bits name another mode, the frame ends
//!   as an error (`unsupported-mode`) whose bits hold the mode.
//! - **End.** A frame is decided as soon as its last half bit is certain. A last bit of 0 ends
//!   with a mark, decided at the mark's end; a last bit of 1 ends with a space, decided once the
//!   space has lasted the shortest `t` its window allows. The signal-free time of 6 `t` after
//!   the frame is not waited for.
//! - **Errors.** An interval outside its windows, or an edge that does not come within the
//!   longest of them, ends the frame as an error naming the field of the bit being read:
//!   `leader`, `start`, `mode`, `toggle` (the trailer bit), `address` or `command` (the last
//!   bit's second half included). The error carries the bits read before it.
//! - **Provenance.** A frame spans from the first tick of its leader mark to the end of its last
//!   half bit — the end of its final mark, or ⌊`t`⌋ ticks after its last middle when it ends in
//!   a space — and is settled where it was decided. An error spans from the same start to the
//!   edge that decided it, or through the last tick of an interval that outlasted its window,
//!   and is settled there.
//! - **Alignment.** At the first sample, after a gap and after every frame or error, the
//!   decoder waits for the line to idle — to stay at space longer than the longest space a frame
//!   has, 3 `t` with its tolerance (1.667 ms at 25%) — before it takes a mark for a leader. A
//!   frame already under way when the samples begin or resume cannot be aligned and is passed
//!   over, and so is the rest of a frame after an error.
//! - **Gaps and the end of the samples.** A frame cut off by a gap or by the end of the samples
//!   is not reported. Progress is passed on, but never past the start of a leader mark whose
//!   frame is still being read; before a gap or the end goes out, progress is announced up to
//!   its start, or up to the start of the frame it cut off.
//! - **Rate.** The input requires a source at which the tolerance of `t` spans a tick:
//!   ⌈100 / (tolerance · 444.4 µs)⌉ samples per second, 9000 at 25%, or sampling alone could
//!   push an interval out of its window. Should a decoder start on a slower source all the same,
//!   it reports nothing and only passes gaps, progress and the end on.
//!
//! Limits: one receiver output, demodulated (the 36 kHz bursts themselves are not read); mode 0
//! only — mode 6 (RC-6A, as Windows Media Center remote controls send it) and the other modes
//! are reported as `unsupported-mode`; the signal-free time after a frame is not checked.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use uiharu_core::{
    Change, ChannelMask, InputRef, InputSpec, Io, Levels, LogicBlock, MAX_CHANNELS, Node,
    NodeFuture, NodeId, NodeSpec, Outbox, OutputRef, OutputSpec, Props, Provenance, Requires,
    Sample, SignalCursor, Tick, TickRange,
};

use super::events::{ErrorReason, Rc6Event};

/// The largest tolerance a configuration may ask for, in percent.
pub const MAX_TOLERANCE: u8 = 50;

/// The unit `t`, 444.4 µs (16 periods of the 36 kHz carrier), as the fraction of a second
/// `UNIT.0 / UNIT.1`.
const UNIT: (u64, u64) = (1, 2250);
/// The leader mark, in units.
const LEADER_MARK: u64 = 6;
/// The leader space, in units.
const LEADER_SPACE: u64 = 2;
/// The bits of a frame: the start bit, three mode bits, the trailer bit, 16 data bits.
const FRAME_BITS: u8 = 21;
/// The trailer bit's position in a frame.
const TRAILER: u8 = 4;
/// The mode bits read: with the start bit, the four first bits of a frame.
const HEADER_BITS: u8 = 4;

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
pub struct Rc6Config {
    /// The channel of the receiver's output.
    pub channel: u8,
    /// Which level is a mark.
    pub polarity: Polarity,
    /// How far an interval between two edges may be from its nominal length, in percent of
    /// it, 1 to [`MAX_TOLERANCE`].
    pub tolerance: u8,
}

impl Default for Rc6Config {
    fn default() -> Self {
        Self {
            channel: 0,
            polarity: Polarity::ActiveLow,
            tolerance: 25,
        }
    }
}

impl Rc6Config {
    /// Checks the configuration.
    pub fn validate(&self) -> Result<(), Rc6ConfigError> {
        if self.channel >= MAX_CHANNELS {
            return Err(Rc6ConfigError::Channel(self.channel));
        }
        if !(1..=MAX_TOLERANCE).contains(&self.tolerance) {
            return Err(Rc6ConfigError::Tolerance(self.tolerance));
        }
        Ok(())
    }

    /// What the decoder's output port declares.
    pub fn props(&self) -> Props {
        Props::new().with("protocol", "rc6")
    }

    /// The fewest samples per second the decoder reads: the rate at which the tolerance of the
    /// unit, 444.4 µs, spans one tick.
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
pub enum Rc6ConfigError {
    /// The channel number is out of range.
    #[error("channel {0} is out of range; channels are 0 to {last}", last = MAX_CHANNELS - 1)]
    Channel(u8),
    /// The tolerance is not 1% to [`MAX_TOLERANCE`].
    #[error("a tolerance of {0}%; 1% to {MAX_TOLERANCE}% are supported")]
    Tolerance(u8),
}

/// The RC-6 decoder node: logic samples in, `ir_rc6.events` out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rc6 {
    config: Rc6Config,
}

impl Rc6 {
    /// A decoder for `config`.
    pub fn new(config: Rc6Config) -> Result<Self, Rc6ConfigError> {
        config.validate()?;
        Ok(Self { config })
    }

    /// The configuration.
    pub fn config(&self) -> &Rc6Config {
        &self.config
    }

    /// The logic input of the decoder added as `node`.
    pub fn input(node: NodeId) -> InputRef<Arc<LogicBlock>> {
        InputRef::of_node(node, 0)
    }

    /// The event output of the decoder added as `node`.
    pub fn events(node: NodeId) -> OutputRef<Rc6Event> {
        OutputRef::of_node(node, 0)
    }
}

impl Node for Rc6 {
    fn spec(&self) -> NodeSpec {
        NodeSpec {
            name: "ir-rc6".into(),
            inputs: vec![InputSpec::new::<Arc<LogicBlock>>(
                "logic",
                self.config.requires(),
            )],
            outputs: vec![OutputSpec::new::<Rc6Event>("events", self.config.props())],
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
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Window {
    min: u64,
    max: u64,
}

impl Window {
    fn holds(self, ticks: u64) -> bool {
        (self.min..=self.max).contains(&ticks)
    }
}

/// RC-6 lengths in ticks at the source's rate.
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

    /// `units` units in ticks, rounded down.
    fn ticks(&self, units: u64) -> u64 {
        saturate(self.scaled(units) / u128::from(UNIT.1))
    }

    /// The window of `units` units.
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

/// The half of bit `bit` of a frame, in units: the trailer bit's halves are twice as long.
fn half(bit: u8) -> u64 {
    if bit == TRAILER { 2 } else { 1 }
}

/// The field bit `bit` of a frame belongs to, the start bit being bit 0.
fn field(bit: u8) -> ErrorReason {
    match bit {
        0 => ErrorReason::Start,
        1..=3 => ErrorReason::Mode,
        TRAILER => ErrorReason::Toggle,
        5..=12 => ErrorReason::Address,
        _ => ErrorReason::Command,
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

impl Line {
    /// The line at mark, or at space since `since`.
    fn at(mark: bool, since: Tick) -> Line {
        if mark { Line::Mark } else { Line::Space(since) }
    }
}

/// Why the samples stopped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stop {
    Gap(TickRange),
    End(Tick),
}

/// How an interval between two edges ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Element {
    /// At this edge.
    Ended(Tick),
    /// Not within the longest it may last: the line was still at its level at this tick.
    Outlasted(Tick),
}

/// Why a mark on the idle line did not give a frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Halt {
    /// The samples stopped.
    Stop(Stop),
    /// The mark was too short for a leader.
    NoLeader,
    /// An interval outside its windows, or a mode not read: the error, and where the line
    /// stands after it.
    Failed(Provenance, Rc6Event, Line),
}

impl From<Stop> for Halt {
    fn from(stop: Stop) -> Self {
        Halt::Stop(stop)
    }
}

/// What a leader began: where, and the bits read so far.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Reading {
    start: Tick,
    count: u8,
    raw: u32,
}

impl Reading {
    fn push(&mut self, one: bool) {
        self.raw = (self.raw << 1) | u32::from(one);
        self.count += 1;
    }

    fn error(&self, reason: ErrorReason) -> Rc6Event {
        Rc6Event::Error {
            reason,
            bits: self.count,
            raw: self.raw,
        }
    }

    /// The error decided at `edge`, where an interval ended too soon or outside its windows.
    fn failed_at(&self, edge: Tick, reason: ErrorReason, line: Line) -> Halt {
        let at = Provenance::over(TickRange::new(self.start, edge), edge);
        Halt::Failed(at, self.error(reason), line)
    }

    /// The error decided at `last`, the last tick of an interval that outlasted its window.
    fn failed_after(&self, last: Tick, reason: ErrorReason, line: Line) -> Halt {
        let at = Provenance::over(TickRange::new(self.start, last.next()), last);
        Halt::Failed(at, self.error(reason), line)
    }

    /// The frame of the 21 bits read.
    fn frame(&self) -> Rc6Event {
        let raw = self.raw;
        Rc6Event::Frame {
            mode: (raw >> 17 & 0b111) as u8,
            toggle: raw >> 16 & 1 == 1,
            address: (raw >> 8 & 0xff) as u8,
            command: (raw & 0xff) as u8,
        }
    }
}

struct Decoder {
    signal: SignalCursor,
    out: Outbox<Rc6Event>,
    polarity: Polarity,
    channel: u8,
    watch: ChannelMask,
    timing: Timing,
    /// The windows of one to six units, by their length.
    windows: [Window; 7],
    /// The longest space a frame has, 3 `t`: the line idles once a space outlasts it.
    idle: u64,
    /// The latest progress passed on.
    promised: Option<Tick>,
}

impl Decoder {
    fn new(signal: SignalCursor, out: Outbox<Rc6Event>, config: Rc6Config, rate: u64) -> Self {
        let timing = Timing {
            rate,
            tolerance: u64::from(config.tolerance),
        };
        let windows = [0, 1, 2, 3, 4, 5, 6].map(|units| timing.window(units));
        Self {
            signal,
            out,
            polarity: config.polarity,
            channel: config.channel,
            watch: ChannelMask::one(config.channel),
            timing,
            idle: windows[3].max,
            windows,
            promised: None,
        }
    }

    /// The window of `units` units.
    fn window(&self, units: u64) -> Window {
        self.windows
            .get(usize::try_from(units).unwrap_or(usize::MAX))
            .copied()
            .unwrap_or_else(|| self.timing.window(units))
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

    /// Waits for the edge that ends an interval begun at `from`, for `max` ticks of it at most.
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

    /// Reads an element of the frame `reading`, begun at `from` at mark (`mark`) or at space,
    /// that must fit `window`: its end. A failure names `reason`.
    async fn fitting(
        &mut self,
        reading: &Reading,
        from: Tick,
        mark: bool,
        window: Window,
        reason: ErrorReason,
    ) -> Result<Tick, Halt> {
        match self.element(from, window.max, Some(reading.start)).await? {
            Element::Ended(end) if from.until(end) < window.min => {
                Err(reading.failed_at(end, reason, Line::at(!mark, end)))
            }
            Element::Ended(end) => Ok(end),
            Element::Outlasted(last) => {
                Err(reading.failed_after(last, reason, Line::at(mark, from)))
            }
        }
    }

    /// Waits for the line to idle: to stay at space longer than any space of a frame.
    async fn idle(&mut self, mut line: Line) -> Result<(), Stop> {
        let idle = self.idle;
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

    /// Reads what the mark that began at `start` on the idle line begins: a frame, and where
    /// the line stands after it.
    async fn read(&mut self, start: Tick) -> Result<(Provenance, Rc6Event, Line), Halt> {
        let under_way = Some(start);
        let mut reading = Reading {
            start,
            count: 0,
            raw: 0,
        };
        let leader = self.window(LEADER_MARK);
        let mark_end = match self.element(start, leader.max, under_way).await? {
            Element::Ended(end) if start.until(end) < leader.min => return Err(Halt::NoLeader),
            Element::Ended(end) => end,
            Element::Outlasted(last) => {
                return Err(reading.failed_after(last, ErrorReason::Leader, Line::Mark));
            }
        };
        let window = self.window(LEADER_SPACE);
        let space_end = self
            .fitting(&reading, mark_end, false, window, ErrorReason::Leader)
            .await?;
        // The start bit, a 1: its first half, a mark, ends at its middle.
        let window = self.window(1);
        let mut middle = self
            .fitting(&reading, space_end, true, window, ErrorReason::Start)
            .await?;
        reading.push(true);
        let mut at_mark = false;
        while reading.count < FRAME_BITS {
            let bit = reading.count;
            let reason = field(bit);
            let (rest, first) = (half(bit - 1), half(bit));
            let longest = self.window(rest + first).max;
            let edge = match self.element(middle, longest, under_way).await? {
                Element::Ended(edge) => edge,
                Element::Outlasted(last) => {
                    return Err(reading.failed_after(last, reason, Line::at(at_mark, middle)));
                }
            };
            match self
                .timing
                .classify(middle.until(edge), [rest, rest + first])
            {
                // An edge between two equal bits: the next middle follows after the first half
                // of the next bit.
                Some(units) if units == rest => {
                    let window = self.window(first);
                    middle = self
                        .fitting(&reading, edge, !at_mark, window, reason)
                        .await?;
                }
                // The next middle: the bit is unequal to the last.
                Some(_) => {
                    middle = edge;
                    at_mark = !at_mark;
                }
                None => return Err(reading.failed_at(edge, reason, Line::at(!at_mark, edge))),
            }
            // A bit whose second half is a space is a 1.
            reading.push(!at_mark);
            if reading.count == HEADER_BITS && reading.raw & 0b111 != 0 {
                let line = Line::at(at_mark, middle);
                return Err(reading.failed_at(middle, ErrorReason::UnsupportedMode, line));
            }
        }
        let reason = ErrorReason::Command;
        let window = self.window(1);
        if at_mark {
            let end = self.fitting(&reading, middle, true, window, reason).await?;
            let at = Provenance::over(TickRange::new(start, end), end);
            return Ok((at, reading.frame(), Line::Space(end)));
        }
        // A last half bit at space is certain once it has lasted the shortest unit.
        match self
            .element(middle, window.min.saturating_sub(1), under_way)
            .await?
        {
            Element::Ended(edge) => Err(reading.failed_at(edge, reason, Line::Mark)),
            Element::Outlasted(last) => {
                let end = middle.offset(self.timing.ticks(1));
                let at = Provenance::over(TickRange::new(start, end), last);
                Ok((at, reading.frame(), Line::Space(middle)))
            }
        }
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
async fn without_frames(mut signal: SignalCursor, mut out: Outbox<Rc6Event>) {
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
    use crate::ir_rc6::IR_RC6_EVENTS;

    #[test]
    fn configurations_are_checked() {
        assert_eq!(Rc6Config::default().validate(), Ok(()));
        let bad = |config: Rc6Config| config.validate().expect_err("rejected");
        assert_eq!(
            bad(Rc6Config {
                channel: 64,
                ..Rc6Config::default()
            }),
            Rc6ConfigError::Channel(64)
        );
        for tolerance in [0, 51] {
            assert_eq!(
                bad(Rc6Config {
                    tolerance,
                    ..Rc6Config::default()
                }),
                Rc6ConfigError::Tolerance(tolerance)
            );
        }
        assert!(Rc6::new(Rc6Config::default()).is_ok());
    }

    #[test]
    fn descriptions_may_leave_fields_out() {
        let config: Rc6Config =
            serde_json::from_str(r#"{"polarity": "active-high"}"#).expect("parses");
        assert_eq!(
            config,
            Rc6Config {
                polarity: Polarity::ActiveHigh,
                ..Rc6Config::default()
            }
        );
        let whole: Rc6Config = serde_json::from_str("{}").expect("parses");
        assert_eq!(whole, Rc6Config::default());
        assert!(serde_json::from_str::<Rc6Config>(r#"{"mode": 6}"#).is_err());
        assert_eq!(Polarity::ActiveHigh.name(), "active-high");
    }

    #[test]
    fn ports_declare_the_protocol_and_require_a_fine_enough_rate() {
        let config = Rc6Config {
            channel: 1,
            ..Rc6Config::default()
        };
        assert_eq!(config.props().get("protocol"), Some(&"rc6".into()));
        // 444.4 µs × 25% is one tick at 9000 samples per second.
        assert_eq!(config.min_rate_hz(), 9000);
        let spec = Rc6::new(config).expect("valid").spec();
        assert_eq!(spec.outputs[0].schema, IR_RC6_EVENTS);
        let requires = &spec.inputs[0].requires;
        let source =
            |channels: i64, rate: i64| Props::new().with("channels", channels).with("rateHz", rate);
        assert!(requires.unmet(&source(2, 9000)).is_none());
        assert_eq!(
            requires.unmet(&source(1, 9000)).map(|(key, _)| key),
            Some("channels")
        );
        assert_eq!(
            requires.unmet(&source(2, 8999)).map(|(key, _)| key),
            Some("rateHz")
        );
    }

    #[test]
    fn windows_and_the_lengths_met_at_each_point() {
        // 72 kHz: a unit is 32 ticks.
        let timing = Timing {
            rate: 72_000,
            tolerance: 25,
        };
        assert_eq!(timing.window(1), Window { min: 24, max: 40 });
        assert_eq!(timing.window(2), Window { min: 48, max: 80 });
        assert_eq!(timing.window(3), Window { min: 72, max: 120 });
        assert_eq!(timing.window(6), Window { min: 144, max: 240 });
        assert_eq!(timing.ticks(1), 32);
        // Before the trailer bit's middle: one unit or three.
        assert_eq!(timing.classify(40, [1, 3]), Some(1));
        assert_eq!(timing.classify(60, [1, 3]), None);
        assert_eq!(timing.classify(72, [1, 3]), Some(3));
        // After it: two units or three, whose windows overlap from 72 to 80 ticks; 80 is as
        // near to both, and the shorter wins.
        assert_eq!(timing.classify(72, [2, 3]), Some(2));
        assert_eq!(timing.classify(80, [2, 3]), Some(2));
        assert_eq!(timing.classify(81, [2, 3]), Some(3));
        assert_eq!(timing.classify(47, [2, 3]), None);
        assert_eq!(
            [0, 1, 2, 3, 4, 5, 6, 7, 20].map(half),
            [1, 1, 1, 1, 2, 1, 1, 1, 1]
        );
        assert_eq!(field(0), ErrorReason::Start);
        assert_eq!(field(3), ErrorReason::Mode);
        assert_eq!(field(4), ErrorReason::Toggle);
        assert_eq!(field(5), ErrorReason::Address);
        assert_eq!(field(12), ErrorReason::Address);
        assert_eq!(field(13), ErrorReason::Command);
        assert_eq!(field(20), ErrorReason::Command);
    }

    #[test]
    fn frames_name_their_fields() {
        let reading = |toggle: u32, address: u32, command: u32| Reading {
            start: Tick::ZERO,
            count: FRAME_BITS,
            raw: 1 << 20 | toggle << 16 | address << 8 | command,
        };
        assert_eq!(
            reading(1, 0x04, 0x5c).frame(),
            Rc6Event::Frame {
                mode: 0,
                toggle: true,
                address: 4,
                command: 0x5c,
            }
        );
        assert_eq!(
            reading(0, 0xff, 0x01).frame(),
            Rc6Event::Frame {
                mode: 0,
                toggle: false,
                address: 0xff,
                command: 1,
            }
        );
    }
}
