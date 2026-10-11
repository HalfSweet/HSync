// SPDX-License-Identifier: Apache-2.0

//! The RC-5 decoder: the frames of a Philips RC-5 infrared remote control, and of its extension
//! RC-5X, read from the demodulated output of an infrared receiver.
//!
//! Implemented from Philips Semiconductors' data sheet of the SAA3010 RC-5 infrared transmitter
//! and the SB-Projects description of RC-5 and its extension. The carrier is 36 kHz and the
//! coding bi-phase: a bit lasts 64 carrier periods, 1.778 ms, in two halves of 888.9 µs at
//! opposite levels. The rules:
//!
//! - **Levels.** A receiver's output is at mark while it receives the carrier and at space
//!   otherwise. Mark is low (`active-low`, as receiver modules put it out) or high
//!   (`active-high`); the idle line is at space.
//! - **Bits.** A 1 is a space then a mark, a 0 a mark then a space: every bit has an edge in its
//!   middle, whose direction is the bit, and two bits have an edge between them only when they
//!   are equal.
//! - **Frame.** 14 bits, most significant bit first: two start bits, the toggle bit, five
//!   address bits and six command bits. The first start bit is always 1. The second is 1 in
//!   RC-5; in RC-5X it is the inverse of bit 6 of the command, so a 0 there makes the command
//!   one of 64 to 127.
//! - **Edges.** The bits are read from the edges, never from a clock of the decoder's own. The
//!   first edge of a frame is the middle of the first start bit. From each middle the next edge
//!   comes one half bit later — an edge between two equal bits, the next middle following one
//!   half bit after it — or two half bits later, the next middle of a bit unequal to the last.
//!   Every interval between two edges is judged on its own, and every middle re-anchors the
//!   timing, so a remote control whose clock is off by less than the tolerance is followed bit
//!   after bit without the error adding up.
//! - **Durations.** An interval of nominal length `d` is in its window from
//!   ⌈`d · rate · (100 − tolerance) / 100`⌉ to ⌊`d · rate · (100 + tolerance) / 100`⌋ ticks,
//!   `rate` being the source's samples per second, computed exactly in integer arithmetic. Where
//!   the windows of one and two half bits overlap (at tolerances above 33%), an interval in both
//!   belongs to the length it is nearer to, one half bit when it is as near to both.
//! - **Start.** On the idle line, an edge to mark is the middle of a first start bit. When the
//!   mark after it is shorter than a half bit's window it was a glitch, and is passed over.
//! - **End.** A frame is decided as soon as its last half bit is certain. A last bit of 1 ends
//!   with a mark, decided at the mark's end; a last bit of 0 ends with a space, decided once the
//!   space has lasted the shortest half bit its window allows, without waiting for an edge.
//! - **Errors.** An interval outside its windows, or an edge that does not come within the
//!   longest of them, ends the frame as an error naming the field of the bit being read:
//!   `start`, `toggle`, `address` or `command` (the last bit's second half included). The error
//!   carries the bits read before it.
//! - **Provenance.** The first half of the first start bit is a space, which cannot be told from
//!   the idle line: a frame is placed from ⌊half bit⌋ ticks before its first edge to the end of
//!   its last half bit — the end of its final mark, or ⌊half bit⌋ ticks after its last middle
//!   when it ends in a space — and is settled where it was decided. An error spans from the
//!   same start to the edge that decided it, or through the last tick of an interval that
//!   outlasted its window, and is settled there.
//! - **Alignment.** At the first sample, after a gap and after every frame or error, the
//!   decoder waits for the line to idle — to stay at space longer than the longest space a frame
//!   has, two half bits with their tolerance (2.222 ms at 25%) — before it takes an edge to mark
//!   for the start of a frame. A frame already under way when the samples begin or resume cannot
//!   be aligned and is passed over, and so is the rest of a frame after an error.
//! - **Gaps and the end of the samples.** A frame cut off by a gap or by the end of the samples
//!   is not reported. Progress is passed on, but never past the start of a frame still being
//!   read, and on the idle line half a bit short of the samples seen, since the next frame
//!   begins half a bit before its first edge; before a gap or the end goes out, progress is
//!   announced up to its start, or up to the start of the frame it cut off.
//! - **Rate.** The input requires a source at which the tolerance of a half bit spans a tick:
//!   ⌈100 / (tolerance · 888.9 µs)⌉ samples per second, 4500 at 25%, or sampling alone could
//!   push an interval out of its window. Should a decoder start on a slower source all the same,
//!   it reports nothing and only passes gaps, progress and the end on.
//!
//! Limits: one receiver output, demodulated (the 36 kHz bursts themselves are not read); the
//! repetition of frames every 64 bit times while a key is held is not checked.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use uiharu_core::{
    Change, ChannelMask, InputRef, InputSpec, Io, Levels, LogicBlock, MAX_CHANNELS, Node,
    NodeFuture, NodeId, NodeSpec, Outbox, OutputRef, OutputSpec, Props, Provenance, Requires,
    Sample, SignalCursor, Tick, TickRange,
};

use super::events::{ErrorReason, Rc5Event};

/// The largest tolerance a configuration may ask for, in percent.
pub const MAX_TOLERANCE: u8 = 50;

/// A half bit, 888.9 µs (32 periods of the 36 kHz carrier), as the fraction of a second
/// `HALF.0 / HALF.1`.
const HALF: (u64, u64) = (1, 1125);
/// The bits of a frame.
const FRAME_BITS: u8 = 14;

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
pub struct Rc5Config {
    /// The channel of the receiver's output.
    pub channel: u8,
    /// Which level is a mark.
    pub polarity: Polarity,
    /// How far an interval between two edges may be from its nominal length, in percent of
    /// it, 1 to [`MAX_TOLERANCE`].
    pub tolerance: u8,
}

impl Default for Rc5Config {
    fn default() -> Self {
        Self {
            channel: 0,
            polarity: Polarity::ActiveLow,
            tolerance: 25,
        }
    }
}

impl Rc5Config {
    /// Checks the configuration.
    pub fn validate(&self) -> Result<(), Rc5ConfigError> {
        if self.channel >= MAX_CHANNELS {
            return Err(Rc5ConfigError::Channel(self.channel));
        }
        if !(1..=MAX_TOLERANCE).contains(&self.tolerance) {
            return Err(Rc5ConfigError::Tolerance(self.tolerance));
        }
        Ok(())
    }

    /// What the decoder's output port declares.
    pub fn props(&self) -> Props {
        Props::new().with("protocol", "rc5")
    }

    /// The fewest samples per second the decoder reads: the rate at which the tolerance of a
    /// half bit, 888.9 µs, spans one tick.
    pub fn min_rate_hz(&self) -> u64 {
        let tolerance = u64::from(self.tolerance.max(1));
        (100 * HALF.1).div_ceil(tolerance * HALF.0)
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
pub enum Rc5ConfigError {
    /// The channel number is out of range.
    #[error("channel {0} is out of range; channels are 0 to {last}", last = MAX_CHANNELS - 1)]
    Channel(u8),
    /// The tolerance is not 1% to [`MAX_TOLERANCE`].
    #[error("a tolerance of {0}%; 1% to {MAX_TOLERANCE}% are supported")]
    Tolerance(u8),
}

/// The RC-5 decoder node: logic samples in, `ir_rc5.events` out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rc5 {
    config: Rc5Config,
}

impl Rc5 {
    /// A decoder for `config`.
    pub fn new(config: Rc5Config) -> Result<Self, Rc5ConfigError> {
        config.validate()?;
        Ok(Self { config })
    }

    /// The configuration.
    pub fn config(&self) -> &Rc5Config {
        &self.config
    }

    /// The logic input of the decoder added as `node`.
    pub fn input(node: NodeId) -> InputRef<Arc<LogicBlock>> {
        InputRef::of_node(node, 0)
    }

    /// The event output of the decoder added as `node`.
    pub fn events(node: NodeId) -> OutputRef<Rc5Event> {
        OutputRef::of_node(node, 0)
    }
}

impl Node for Rc5 {
    fn spec(&self) -> NodeSpec {
        NodeSpec {
            name: "ir-rc5".into(),
            inputs: vec![InputSpec::new::<Arc<LogicBlock>>(
                "logic",
                self.config.requires(),
            )],
            outputs: vec![OutputSpec::new::<Rc5Event>("events", self.config.props())],
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

/// RC-5 lengths in ticks at the source's rate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Timing {
    rate: u64,
    tolerance: u64,
}

impl Timing {
    /// `halves` half bits in ticks, multiplied by the half bit's denominator so that it stays
    /// exact.
    fn scaled(&self, halves: u64) -> u128 {
        u128::from(self.rate) * u128::from(halves) * u128::from(HALF.0)
    }

    /// `halves` half bits in ticks, rounded down.
    fn ticks(&self, halves: u64) -> u64 {
        saturate(self.scaled(halves) / u128::from(HALF.1))
    }

    /// The window of `halves` half bits.
    fn window(&self, halves: u64) -> Window {
        let scaled = self.scaled(halves);
        let den = u128::from(HALF.1) * 100;
        Window {
            min: saturate((scaled * u128::from(100 - self.tolerance)).div_ceil(den)),
            max: saturate(scaled * u128::from(100 + self.tolerance) / den),
        }
    }

    /// Whether `ticks` is one half bit or two: the one whose window holds it, or where both
    /// do, the one it is nearer to, one half bit when it is as near to both.
    fn classify(&self, ticks: u64) -> Option<u64> {
        let mut best: Option<(u64, u128)> = None;
        for halves in [1, 2] {
            if !self.window(halves).holds(ticks) {
                continue;
            }
            let distance = (u128::from(ticks) * u128::from(HALF.1)).abs_diff(self.scaled(halves));
            if best.is_none_or(|(_, nearest)| distance < nearest) {
                best = Some((halves, distance));
            }
        }
        best.map(|(halves, _)| halves)
    }
}

fn saturate(ticks: u128) -> u64 {
    u64::try_from(ticks).unwrap_or(u64::MAX)
}

/// The windows the decoder compares with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Windows {
    /// One half bit.
    half: Window,
    /// Two half bits.
    full: Window,
    /// A half bit in ticks, rounded down: where a frame is placed before its first edge.
    half_ticks: u64,
    /// The longest space a frame has, two half bits: the line idles once a space outlasts it.
    idle: u64,
}

impl Windows {
    fn new(timing: &Timing) -> Self {
        let full = timing.window(2);
        Self {
            half: timing.window(1),
            full,
            half_ticks: timing.ticks(1),
            idle: full.max,
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

/// Why an edge to mark on the idle line did not give a frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Halt {
    /// The samples stopped.
    Stop(Stop),
    /// The mark after it was too short for a half bit.
    Glitch,
    /// An interval outside its windows: the error, and where the line stands after it.
    Failed(Provenance, Rc5Event, Line),
}

impl From<Stop> for Halt {
    fn from(stop: Stop) -> Self {
        Halt::Stop(stop)
    }
}

/// The field bit `bit` of a frame belongs to, the first start bit being bit 0.
fn field(bit: u8) -> ErrorReason {
    match bit {
        0 | 1 => ErrorReason::Start,
        2 => ErrorReason::Toggle,
        3..=7 => ErrorReason::Address,
        _ => ErrorReason::Command,
    }
}

/// What a first edge began: where the frame is placed, and the bits read so far.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Reading {
    start: Tick,
    count: u8,
    raw: u16,
}

impl Reading {
    fn push(&mut self, one: bool) {
        self.raw = (self.raw << 1) | u16::from(one);
        self.count += 1;
    }

    fn error(&self, reason: ErrorReason) -> Rc5Event {
        Rc5Event::Error {
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

    /// The frame of the 14 bits read.
    fn frame(&self) -> Rc5Event {
        let raw = self.raw;
        let second_start = raw >> 12 & 1 == 1;
        Rc5Event::Frame {
            toggle: raw >> 11 & 1 == 1,
            address: (raw >> 6 & 0x1f) as u8,
            command: (raw & 0x3f) as u8 | u8::from(!second_start) << 6,
            extended: !second_start,
        }
    }
}

struct Decoder {
    signal: SignalCursor,
    out: Outbox<Rc5Event>,
    polarity: Polarity,
    channel: u8,
    watch: ChannelMask,
    timing: Timing,
    windows: Windows,
    /// The latest progress passed on.
    promised: Option<Tick>,
}

impl Decoder {
    fn new(signal: SignalCursor, out: Outbox<Rc5Event>, config: Rc5Config, rate: u64) -> Self {
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

    /// Waits for the next edge, with no frame under way; progress is held back `behind` ticks.
    async fn edge(&mut self, behind: u64) -> Result<Tick, Stop> {
        loop {
            match self.signal.next_change(self.watch).await {
                Change::At { tick, .. } => return Ok(tick),
                // Without a deadline the wait is never quiet.
                Change::Quiet(_) => {}
                Change::Progress(tick) => self.progress(Tick(tick.0.saturating_sub(behind))).await,
                Change::Gap(range) => return Err(Stop::Gap(range)),
                Change::End(tick) => return Err(Stop::End(tick)),
            }
        }
    }

    /// Waits for the edge that ends an interval begun at `from`, for `max` ticks of it at most.
    /// Progress is held at `held`, if given.
    async fn element(&mut self, from: Tick, max: u64, held: Option<Tick>) -> Result<Element, Stop> {
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
                    let held = held.map_or(tick, |held| held.min(tick));
                    self.progress(held).await;
                }
                Change::Gap(range) => return Err(Stop::Gap(range)),
                Change::End(tick) => return Err(Stop::End(tick)),
            }
        }
    }

    /// Waits for the line to idle: to stay at space longer than any space of a frame.
    async fn idle(&mut self, mut line: Line) -> Result<(), Stop> {
        let Windows {
            idle, half_ticks, ..
        } = self.windows;
        loop {
            line = match line {
                Line::Unknown => match self.sample().await? {
                    (_, true) => Line::Mark,
                    (tick, false) => Line::Space(tick),
                },
                Line::Mark => Line::Space(self.edge(0).await?),
                Line::Space(since) => {
                    // A frame begins half a bit before its first edge, which comes no sooner
                    // than the line is seen idle.
                    let first = Tick(since.offset(idle).0.saturating_sub(half_ticks)).next();
                    match self.element(since, idle, Some(first)).await? {
                        Element::Outlasted(_) => return Ok(()),
                        Element::Ended(_) => Line::Mark,
                    }
                }
            };
        }
    }

    /// Reads the frame whose first edge, the middle of its first start bit, is at `first`: the
    /// frame, and where the line stands after it.
    async fn read(&mut self, first: Tick) -> Result<(Provenance, Rc5Event, Line), Halt> {
        let windows = self.windows;
        let start = Tick(first.0.saturating_sub(windows.half_ticks));
        let held = Some(start);
        // The first start bit is a 1: it is read at its middle, which leaves the line at mark.
        let mut reading = Reading {
            start,
            count: 1,
            raw: 1,
        };
        let mut middle = first;
        let mut at_mark = true;
        while reading.count < FRAME_BITS {
            let reason = field(reading.count);
            let edge = match self.element(middle, windows.full.max, held).await? {
                Element::Ended(edge) => edge,
                Element::Outlasted(last) => {
                    return Err(reading.failed_after(last, reason, Line::at(at_mark, middle)));
                }
            };
            let interval = middle.until(edge);
            if reading.count == 1 && interval < windows.half.min {
                return Err(Halt::Glitch);
            }
            match self.timing.classify(interval) {
                // An edge between two equal bits: the next middle follows a half bit later.
                Some(1) => {
                    let next = match self.element(edge, windows.half.max, held).await? {
                        Element::Ended(next) => next,
                        Element::Outlasted(last) => {
                            let line = Line::at(!at_mark, edge);
                            return Err(reading.failed_after(last, reason, line));
                        }
                    };
                    if edge.until(next) < windows.half.min {
                        let line = Line::at(at_mark, next);
                        return Err(reading.failed_at(next, reason, line));
                    }
                    middle = next;
                }
                // The next middle: the bit is unequal to the last.
                Some(_) => {
                    middle = edge;
                    at_mark = !at_mark;
                }
                None => return Err(reading.failed_at(edge, reason, Line::at(!at_mark, edge))),
            }
            // A bit whose second half is a mark is a 1.
            reading.push(at_mark);
        }
        let reason = ErrorReason::Command;
        if at_mark {
            let end = match self.element(middle, windows.half.max, held).await? {
                Element::Ended(end) if middle.until(end) < windows.half.min => {
                    return Err(reading.failed_at(end, reason, Line::Space(end)));
                }
                Element::Ended(end) => end,
                Element::Outlasted(last) => {
                    return Err(reading.failed_after(last, reason, Line::Mark));
                }
            };
            let at = Provenance::over(TickRange::new(start, end), end);
            return Ok((at, reading.frame(), Line::Space(end)));
        }
        // A last half bit at space is certain once it has lasted the shortest half bit.
        match self
            .element(middle, windows.half.min.saturating_sub(1), held)
            .await?
        {
            Element::Ended(edge) => Err(reading.failed_at(edge, reason, Line::Mark)),
            Element::Outlasted(last) => {
                let end = middle.offset(windows.half_ticks);
                let at = Provenance::over(TickRange::new(start, end), last);
                Ok((at, reading.frame(), Line::Space(middle)))
            }
        }
    }

    /// Waits on the idle line for an edge to mark and reads the frame it begins: the state
    /// after it, or the stop that came first, with the start of the frame it cut off.
    async fn armed(&mut self) -> Result<State, (Stop, Option<Tick>)> {
        let half_ticks = self.windows.half_ticks;
        let first = self.edge(half_ticks).await.map_err(|stop| (stop, None))?;
        match self.read(first).await {
            Ok((at, event, line)) | Err(Halt::Failed(at, event, line)) => {
                self.out.emit(at, event).await;
                Ok(State::Align(line))
            }
            Err(Halt::Glitch) => Ok(State::Armed),
            Err(Halt::Stop(stop)) => Err((stop, Some(Tick(first.0.saturating_sub(half_ticks))))),
        }
    }
}

/// Where the decoder stands between frames.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    /// Waiting for the line to idle, from what is known of it.
    Align(Line),
    /// The line idles: the next edge to mark may begin a frame.
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
async fn without_frames(mut signal: SignalCursor, mut out: Outbox<Rc5Event>) {
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
    use crate::ir_rc5::IR_RC5_EVENTS;

    #[test]
    fn configurations_are_checked() {
        assert_eq!(Rc5Config::default().validate(), Ok(()));
        let bad = |config: Rc5Config| config.validate().expect_err("rejected");
        assert_eq!(
            bad(Rc5Config {
                channel: 64,
                ..Rc5Config::default()
            }),
            Rc5ConfigError::Channel(64)
        );
        for tolerance in [0, 51] {
            assert_eq!(
                bad(Rc5Config {
                    tolerance,
                    ..Rc5Config::default()
                }),
                Rc5ConfigError::Tolerance(tolerance)
            );
        }
        assert!(
            Rc5::new(Rc5Config {
                tolerance: 50,
                ..Rc5Config::default()
            })
            .is_ok()
        );
    }

    #[test]
    fn descriptions_may_leave_fields_out() {
        let config: Rc5Config = serde_json::from_str(r#"{"tolerance": 10}"#).expect("parses");
        assert_eq!(
            config,
            Rc5Config {
                tolerance: 10,
                ..Rc5Config::default()
            }
        );
        let whole: Rc5Config = serde_json::from_str("{}").expect("parses");
        assert_eq!(whole, Rc5Config::default());
        assert!(serde_json::from_str::<Rc5Config>(r#"{"bits": 14}"#).is_err());
        assert_eq!(Polarity::ActiveLow.name(), "active-low");
    }

    #[test]
    fn ports_declare_the_protocol_and_require_a_fine_enough_rate() {
        let config = Rc5Config {
            channel: 7,
            ..Rc5Config::default()
        };
        assert_eq!(config.props().get("protocol"), Some(&"rc5".into()));
        // 888.9 µs × 25% is one tick at 4500 samples per second.
        assert_eq!(config.min_rate_hz(), 4500);
        let spec = Rc5::new(config).expect("valid").spec();
        assert_eq!(spec.outputs[0].schema, IR_RC5_EVENTS);
        let requires = &spec.inputs[0].requires;
        let source =
            |channels: i64, rate: i64| Props::new().with("channels", channels).with("rateHz", rate);
        assert!(requires.unmet(&source(8, 4500)).is_none());
        assert_eq!(
            requires.unmet(&source(7, 4500)).map(|(key, _)| key),
            Some("channels")
        );
        assert_eq!(
            requires.unmet(&source(8, 4499)).map(|(key, _)| key),
            Some("rateHz")
        );
    }

    #[test]
    fn windows_round_toward_their_nominal_length() {
        // 36 kHz: a half bit is 32 ticks.
        let timing = Timing {
            rate: 36_000,
            tolerance: 25,
        };
        let windows = Windows::new(&timing);
        assert_eq!(windows.half, Window { min: 24, max: 40 });
        assert_eq!(windows.full, Window { min: 48, max: 80 });
        assert_eq!((windows.half_ticks, windows.idle), (32, 80));
        assert_eq!(timing.classify(23), None);
        assert_eq!(timing.classify(40), Some(1));
        assert_eq!(timing.classify(44), None);
        assert_eq!(timing.classify(48), Some(2));
        assert_eq!(timing.classify(81), None);
        // 1 MHz: 888.9 ticks, read from 667 to 1111.
        let fine = Timing {
            rate: 1_000_000,
            tolerance: 25,
        };
        assert_eq!(
            fine.window(1),
            Window {
                min: 667,
                max: 1111
            }
        );
        assert_eq!(fine.ticks(1), 888);
        // At 40% the windows overlap from 1067 to 1244 ticks, all of it nearer to one half bit.
        let loose = Timing {
            rate: 1_000_000,
            tolerance: 40,
        };
        assert_eq!(loose.classify(1100), Some(1));
        assert_eq!(loose.classify(1244), Some(1));
        assert_eq!(loose.classify(1245), Some(2));
        // At 50% and 36 kHz, 48 ticks is as near to one half bit as to two: one wins.
        let widest = Timing {
            rate: 36_000,
            tolerance: 50,
        };
        assert_eq!(widest.classify(48), Some(1));
        assert_eq!(widest.classify(49), Some(2));
    }

    #[test]
    fn frames_name_their_fields() {
        // The 14 bits from their fields, the first start bit the most significant.
        let reading = |start: u16, toggle: u16, address: u16, command: u16| Reading {
            start: Tick::ZERO,
            count: 14,
            raw: start << 12 | toggle << 11 | address << 6 | command,
        };
        assert_eq!(
            reading(0b11, 0, 5, 12).frame(),
            Rc5Event::Frame {
                toggle: false,
                address: 5,
                command: 12,
                extended: false,
            }
        );
        // A second start bit of 0: command 64 + 53.
        assert_eq!(
            reading(0b10, 1, 31, 53).frame(),
            Rc5Event::Frame {
                toggle: true,
                address: 31,
                command: 117,
                extended: true,
            }
        );
        assert_eq!(field(0), ErrorReason::Start);
        assert_eq!(field(1), ErrorReason::Start);
        assert_eq!(field(2), ErrorReason::Toggle);
        assert_eq!(field(7), ErrorReason::Address);
        assert_eq!(field(8), ErrorReason::Command);
        assert_eq!(field(13), ErrorReason::Command);
    }
}
