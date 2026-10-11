// SPDX-License-Identifier: Apache-2.0

//! The SIRC decoder: the 12, 15 and 20-bit frames of a Sony SIRC infrared remote control, read
//! from the demodulated output of an infrared receiver.
//!
//! Implemented from the SIRC format as public descriptions of it give it, the SB-Projects
//! description of the Sony SIRC protocol among them. The carrier is 40 kHz and the coding pulse
//! width; the unit `T` is 600 µs, 24 carrier periods. The rules:
//!
//! - **Levels.** A receiver's output is at mark while it receives the carrier and at space
//!   otherwise. Mark is low (`active-low`, as receiver modules put it out) or high
//!   (`active-high`); the idle line is at space.
//! - **Frame.** A start mark of 4 `T` (2.4 ms) and a space of `T`, then 12, 15 or 20 bits,
//!   least significant bit first, each a mark of `T` for a 0 or 2 `T` for a 1 followed by a space
//!   of `T`. The bits are the 7-bit command, then a 5-bit address (12 bits), an 8-bit address
//!   (15 bits), or a 5-bit address and an 8-bit extended field (20 bits). A remote control sends
//!   its frames every 45 ms while a key is held.
//! - **Durations.** Marks and spaces are measured in ticks, from the edge that begins one to the
//!   edge that ends it, and compared with a window around their nominal length `d`: from
//!   ⌈`d · rate · (100 − tolerance) / 100`⌉ to ⌊`d · rate · (100 + tolerance) / 100`⌋ ticks,
//!   `rate` being the source's samples per second, computed exactly in integer arithmetic.
//!   Where the windows of a 0's and a 1's mark overlap (at tolerances above 33%), a mark in both
//!   belongs to the length it is nearer to, the shorter one when it is as near to both.
//! - **Start.** On the idle line a mark of 4 `T` begins a frame. A shorter mark is no start and
//!   is passed over; a longer one is an error (`start`) as soon as it has outlasted its window.
//! - **Length.** The frame says nothing of its length: it ends when the line stays at space
//!   longer than a bit's space allows, `T` with its tolerance (750 µs at 25%). That silence is
//!   known when the samples up to its last tick have been seen without an edge — in the block
//!   that holds them, or in one that comes after the decoder announced progress — and the frame
//!   is decided at that computed tick, wherever the samples were cut. A frame of 12, 15 or 20
//!   bits is reported; any other count is an error (`count`), and so is a 21st bit, as soon as
//!   its mark begins.
//! - **Errors.** A mark or space outside its window ends the frame as an error naming where:
//!   `start`, the start mark or the space after it; `bit`, a bit's mark, or its space shorter
//!   than its window; `count`. The error carries the bits read before it.
//! - **Provenance.** A frame spans from the first tick of its start mark to the end of its last
//!   bit, ⌊`T`⌋ ticks after the last mark ends, and is settled where its silence became certain,
//!   the last tick of the longest space a bit may have. An error spans from the start mark to
//!   the edge that decided it, or through the last tick of an element that outlasted its window,
//!   and is settled there.
//! - **Alignment.** At the first sample, after a gap and after every frame or error, the
//!   decoder waits for the line to idle — to stay at space longer than the longest space a frame
//!   has, `T` with its tolerance, the same silence that ends a frame — before it takes a mark
//!   for a start. A frame already under way when the samples begin or resume cannot be aligned
//!   and is passed over, and so is the rest of a frame after an error.
//! - **Gaps and the end of the samples.** A frame whose silence was not seen before a gap or
//!   the end of the samples is not reported: more bits could have followed. Progress is passed
//!   on, but never past the start of a frame still being read; before a gap or the end goes out,
//!   progress is announced up to its start, or up to the start of the frame it cut off.
//! - **Rate.** The input requires a source at which the tolerance of `T` spans a tick:
//!   ⌈100 / (tolerance · 600 µs)⌉ samples per second, 6667 at 25%, or sampling alone could push
//!   a duration out of its window. Should a decoder start on a slower source all the same, it
//!   reports nothing and only passes gaps, progress and the end on.
//!
//! Limits: one receiver output, demodulated (the 40 kHz bursts themselves are not read); the
//! repetition of frames every 45 ms is not checked, and frames are not tied to one another.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use uiharu_core::{
    Change, ChannelMask, InputRef, InputSpec, Io, Levels, LogicBlock, MAX_CHANNELS, Node,
    NodeFuture, NodeId, NodeSpec, Outbox, OutputRef, OutputSpec, Props, Provenance, Requires,
    Sample, SignalCursor, Tick, TickRange,
};

use super::events::{ErrorReason, SircEvent};

/// The largest tolerance a configuration may ask for, in percent.
pub const MAX_TOLERANCE: u8 = 50;

/// The unit `T`, 600 µs, as the fraction of a second `UNIT.0 / UNIT.1`.
const UNIT: (u64, u64) = (3, 5000);
/// The start mark, in units.
const START_MARK: u64 = 4;
/// The most bits a frame has.
const MAX_BITS: u8 = 20;

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
pub struct SircConfig {
    /// The channel of the receiver's output.
    pub channel: u8,
    /// Which level is a mark.
    pub polarity: Polarity,
    /// How far a mark or space may be from its nominal length, in percent of it, 1 to
    /// [`MAX_TOLERANCE`].
    pub tolerance: u8,
}

impl Default for SircConfig {
    fn default() -> Self {
        Self {
            channel: 0,
            polarity: Polarity::ActiveLow,
            tolerance: 25,
        }
    }
}

impl SircConfig {
    /// Checks the configuration.
    pub fn validate(&self) -> Result<(), SircConfigError> {
        if self.channel >= MAX_CHANNELS {
            return Err(SircConfigError::Channel(self.channel));
        }
        if !(1..=MAX_TOLERANCE).contains(&self.tolerance) {
            return Err(SircConfigError::Tolerance(self.tolerance));
        }
        Ok(())
    }

    /// What the decoder's output port declares.
    pub fn props(&self) -> Props {
        Props::new().with("protocol", "sirc")
    }

    /// The fewest samples per second the decoder reads: the rate at which the tolerance of the
    /// unit, 600 µs, spans one tick.
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
pub enum SircConfigError {
    /// The channel number is out of range.
    #[error("channel {0} is out of range; channels are 0 to {last}", last = MAX_CHANNELS - 1)]
    Channel(u8),
    /// The tolerance is not 1% to [`MAX_TOLERANCE`].
    #[error("a tolerance of {0}%; 1% to {MAX_TOLERANCE}% are supported")]
    Tolerance(u8),
}

/// The SIRC decoder node: logic samples in, `ir_sirc.events` out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sirc {
    config: SircConfig,
}

impl Sirc {
    /// A decoder for `config`.
    pub fn new(config: SircConfig) -> Result<Self, SircConfigError> {
        config.validate()?;
        Ok(Self { config })
    }

    /// The configuration.
    pub fn config(&self) -> &SircConfig {
        &self.config
    }

    /// The logic input of the decoder added as `node`.
    pub fn input(node: NodeId) -> InputRef<Arc<LogicBlock>> {
        InputRef::of_node(node, 0)
    }

    /// The event output of the decoder added as `node`.
    pub fn events(node: NodeId) -> OutputRef<SircEvent> {
        OutputRef::of_node(node, 0)
    }
}

impl Node for Sirc {
    fn spec(&self) -> NodeSpec {
        NodeSpec {
            name: "ir-sirc".into(),
            inputs: vec![InputSpec::new::<Arc<LogicBlock>>(
                "logic",
                self.config.requires(),
            )],
            outputs: vec![OutputSpec::new::<SircEvent>("events", self.config.props())],
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

/// SIRC lengths in ticks at the source's rate.
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

    /// Whether a mark of `ticks` is a 0 or a 1: the one whose window holds it, or where both do,
    /// the one it is nearer to, a 0 when it is as near to both.
    fn bit(&self, ticks: u64) -> Option<bool> {
        let mut best: Option<(bool, u128)> = None;
        for (units, one) in [(1, false), (2, true)] {
            if !self.window(units).holds(ticks) {
                continue;
            }
            let distance = (u128::from(ticks) * u128::from(UNIT.1)).abs_diff(self.scaled(units));
            if best.is_none_or(|(_, nearest)| distance < nearest) {
                best = Some((one, distance));
            }
        }
        best.map(|(one, _)| one)
    }
}

fn saturate(ticks: u128) -> u64 {
    u64::try_from(ticks).unwrap_or(u64::MAX)
}

/// The windows the decoder compares with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Windows {
    start_mark: Window,
    /// A 0's mark and every space.
    unit: Window,
    /// A 1's mark.
    double: Window,
    /// The unit in ticks, rounded down: how far a frame reaches past its last mark.
    unit_ticks: u64,
}

impl Windows {
    fn new(timing: &Timing) -> Self {
        Self {
            start_mark: timing.window(START_MARK),
            unit: timing.window(1),
            double: timing.window(2),
            unit_ticks: timing.ticks(1),
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

/// Why a mark on the idle line did not give a frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Halt {
    /// The samples stopped.
    Stop(Stop),
    /// The mark was too short for a start mark.
    NoStart,
    /// An element outside its window, or a wrong count: the error, and where the line stands
    /// after it.
    Failed(Provenance, SircEvent, Line),
}

impl From<Stop> for Halt {
    fn from(stop: Stop) -> Self {
        Halt::Stop(stop)
    }
}

/// What a start mark began: where, and the bits read so far.
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

    fn error(&self, reason: ErrorReason) -> SircEvent {
        SircEvent::Error {
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

    /// The frame of the bits read, if their count is one a frame has.
    fn frame(&self) -> Option<SircEvent> {
        let raw = self.raw;
        let command = (raw & 0x7f) as u8;
        let short_address = (raw >> 7 & 0x1f) as u8;
        let (address, extended) = match self.count {
            12 => (short_address, None),
            15 => ((raw >> 7 & 0xff) as u8, None),
            20 => (short_address, Some((raw >> 12 & 0xff) as u8)),
            _ => return None,
        };
        Some(SircEvent::Frame {
            bits: self.count,
            command,
            address,
            extended,
        })
    }
}

struct Decoder {
    signal: SignalCursor,
    out: Outbox<SircEvent>,
    polarity: Polarity,
    channel: u8,
    watch: ChannelMask,
    timing: Timing,
    windows: Windows,
    /// The latest progress passed on.
    promised: Option<Tick>,
}

impl Decoder {
    fn new(signal: SignalCursor, out: Outbox<SircEvent>, config: SircConfig, rate: u64) -> Self {
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
    /// Progress is held at `under_way`, the start of the frame being read, if any: it carries
    /// the tick the samples have reached, and a silence that ends a frame is decided by the
    /// samples up to its last tick, never by when they arrived.
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

    /// Waits for the line to idle: to stay at space longer than any space of a frame.
    async fn idle(&mut self, mut line: Line) -> Result<(), Stop> {
        let idle = self.windows.unit.max;
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
    async fn read(&mut self, start: Tick) -> Result<(Provenance, SircEvent, Line), Halt> {
        let windows = self.windows;
        let under_way = Some(start);
        let mut reading = Reading {
            start,
            count: 0,
            raw: 0,
        };
        let start_mark = windows.start_mark;
        let mark_end = match self.element(start, start_mark.max, under_way).await? {
            Element::Ended(end) if start.until(end) < start_mark.min => {
                return Err(Halt::NoStart);
            }
            Element::Ended(end) => end,
            Element::Outlasted(last) => {
                return Err(reading.failed_after(last, ErrorReason::Start, Line::Mark));
            }
        };
        let unit = windows.unit;
        let mut mark_start = match self.element(mark_end, unit.max, under_way).await? {
            Element::Ended(end) if mark_end.until(end) < unit.min => {
                return Err(reading.failed_at(end, ErrorReason::Start, Line::Mark));
            }
            Element::Ended(end) => end,
            Element::Outlasted(last) => {
                let line = Line::Space(mark_end);
                return Err(reading.failed_after(last, ErrorReason::Start, line));
            }
        };
        loop {
            let max = windows.double.max;
            let mark_end = match self.element(mark_start, max, under_way).await? {
                Element::Ended(end) => end,
                Element::Outlasted(last) => {
                    return Err(reading.failed_after(last, ErrorReason::Bit, Line::Mark));
                }
            };
            match self.timing.bit(mark_start.until(mark_end)) {
                Some(one) => reading.push(one),
                None => {
                    let line = Line::Space(mark_end);
                    return Err(reading.failed_at(mark_end, ErrorReason::Bit, line));
                }
            }
            // The space after the bit: a bit's, or the silence that ends the frame.
            match self.element(mark_end, unit.max, under_way).await? {
                Element::Ended(end) if mark_end.until(end) < unit.min => {
                    return Err(reading.failed_at(end, ErrorReason::Bit, Line::Mark));
                }
                Element::Ended(end) if reading.count == MAX_BITS => {
                    return Err(reading.failed_at(end, ErrorReason::Count, Line::Mark));
                }
                Element::Ended(end) => mark_start = end,
                Element::Outlasted(last) => {
                    let line = Line::Space(mark_end);
                    let Some(frame) = reading.frame() else {
                        return Err(reading.failed_after(last, ErrorReason::Count, line));
                    };
                    let end = mark_end.offset(windows.unit_ticks);
                    let at = Provenance::over(TickRange::new(start, end), last);
                    return Ok((at, frame, line));
                }
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
            Err(Halt::NoStart) => Ok(State::Armed),
            Err(Halt::Stop(stop)) => Err((stop, Some(start))),
        }
    }
}

/// Where the decoder stands between frames.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    /// Waiting for the line to idle, from what is known of it.
    Align(Line),
    /// The line idles: the next mark may be a start mark.
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
async fn without_frames(mut signal: SignalCursor, mut out: Outbox<SircEvent>) {
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
    use crate::ir_sirc::IR_SIRC_EVENTS;

    #[test]
    fn configurations_are_checked() {
        assert_eq!(SircConfig::default().validate(), Ok(()));
        let bad = |config: SircConfig| config.validate().expect_err("rejected");
        assert_eq!(
            bad(SircConfig {
                channel: 64,
                ..SircConfig::default()
            }),
            SircConfigError::Channel(64)
        );
        for tolerance in [0, 51] {
            assert_eq!(
                bad(SircConfig {
                    tolerance,
                    ..SircConfig::default()
                }),
                SircConfigError::Tolerance(tolerance)
            );
        }
        assert!(Sirc::new(SircConfig::default()).is_ok());
    }

    #[test]
    fn descriptions_may_leave_fields_out() {
        let config: SircConfig = serde_json::from_str(r#"{"channel": 4}"#).expect("parses");
        assert_eq!(
            config,
            SircConfig {
                channel: 4,
                ..SircConfig::default()
            }
        );
        let whole: SircConfig = serde_json::from_str("{}").expect("parses");
        assert_eq!(whole, SircConfig::default());
        assert!(serde_json::from_str::<SircConfig>(r#"{"bits": 12}"#).is_err());
        assert_eq!(Polarity::ActiveLow.name(), "active-low");
    }

    #[test]
    fn ports_declare_the_protocol_and_require_a_fine_enough_rate() {
        let config = SircConfig {
            channel: 3,
            ..SircConfig::default()
        };
        assert_eq!(config.props().get("protocol"), Some(&"sirc".into()));
        // 600 µs × 25% is one tick at 6666.7 samples per second.
        assert_eq!(config.min_rate_hz(), 6667);
        let spec = Sirc::new(config).expect("valid").spec();
        assert_eq!(spec.outputs[0].schema, IR_SIRC_EVENTS);
        let requires = &spec.inputs[0].requires;
        let source =
            |channels: i64, rate: i64| Props::new().with("channels", channels).with("rateHz", rate);
        assert!(requires.unmet(&source(4, 6667)).is_none());
        assert_eq!(
            requires.unmet(&source(3, 6667)).map(|(key, _)| key),
            Some("channels")
        );
        assert_eq!(
            requires.unmet(&source(4, 6666)).map(|(key, _)| key),
            Some("rateHz")
        );
    }

    #[test]
    fn windows_and_bits() {
        // 40 kHz: the unit is 24 ticks.
        let timing = Timing {
            rate: 40_000,
            tolerance: 25,
        };
        let windows = Windows::new(&timing);
        assert_eq!(windows.unit, Window { min: 18, max: 30 });
        assert_eq!(windows.double, Window { min: 36, max: 60 });
        assert_eq!(windows.start_mark, Window { min: 72, max: 120 });
        assert_eq!(windows.unit_ticks, 24);
        assert_eq!(timing.bit(17), None);
        assert_eq!(timing.bit(18), Some(false));
        assert_eq!(timing.bit(30), Some(false));
        assert_eq!(timing.bit(33), None);
        assert_eq!(timing.bit(36), Some(true));
        assert_eq!(timing.bit(60), Some(true));
        assert_eq!(timing.bit(61), None);
        // At 50% the windows overlap from 24 to 36 ticks; 36 is as near to 24 as to 48.
        let widest = Timing {
            rate: 40_000,
            tolerance: 50,
        };
        assert_eq!(widest.bit(35), Some(false));
        assert_eq!(widest.bit(36), Some(false));
        assert_eq!(widest.bit(37), Some(true));
    }

    #[test]
    fn frames_name_their_fields_by_length() {
        let reading = |count: u8, raw: u32| Reading {
            start: Tick::ZERO,
            count,
            raw,
        };
        // Command 21, address 1.
        assert_eq!(
            reading(12, 1 << 7 | 21).frame(),
            Some(SircEvent::Frame {
                bits: 12,
                command: 21,
                address: 1,
                extended: None,
            })
        );
        assert_eq!(
            reading(15, 0x97 << 7 | 0x2a).frame(),
            Some(SircEvent::Frame {
                bits: 15,
                command: 0x2a,
                address: 0x97,
                extended: None,
            })
        );
        assert_eq!(
            reading(20, 0xe2 << 12 | 0x1a << 7 | 0x7f).frame(),
            Some(SircEvent::Frame {
                bits: 20,
                command: 0x7f,
                address: 0x1a,
                extended: Some(0xe2),
            })
        );
        for count in [0, 7, 11, 13, 14, 16, 19, 21] {
            assert_eq!(reading(count, 0).frame(), None, "{count} bits");
        }
    }
}
