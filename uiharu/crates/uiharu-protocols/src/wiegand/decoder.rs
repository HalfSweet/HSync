// SPDX-License-Identifier: Apache-2.0

//! The Wiegand decoder: the frames an access-control reader sends on DATA0 and DATA1, each bit a
//! low pulse on one of the lines and each frame ended by a silence.
//!
//! Implemented from the Wiegand reader interface as the Security Industry Association's SIA
//! AC-01-1996.10, "Access Control Standard Protocol for the 26-Bit Wiegand Reader Interface",
//! and reader datasheets describe it: both lines idle high; a bit is a low pulse of 20 to 100 µs
//! (nominally 50 µs) on DATA0 for a 0 or on DATA1 for a 1; pulses follow each other 200 µs to
//! 20 ms apart; the 26-bit format is an even parity bit over the next 12 bits, 8 facility-code
//! bits, 16 card-number bits and an odd parity bit over the 12 bits before it.
//!
//! The rules:
//!
//! - a pulse runs from the tick a line falls to the tick it rises, and must last at least
//!   `pulse_min_us` and at most `pulse_max_us`; a bound of "at least `t` µs" is ⌈`t` · rate /
//!   10⁶⌉ ticks, one of "at most `t` µs" ⌊`t` · rate / 10⁶⌋ ticks, both in integer arithmetic;
//! - a frame is a run of pulses; it ends once the lines have stayed high for more than
//!   `frame_gap_us` (⌊`frame_gap_us` · rate / 10⁶⌋ ticks) after its last pulse, and a pulse
//!   falling later begins the next frame. Whether the next pulse or the samples reaching the
//!   end of that silence tell the decoder, the frame is settled at `last release + frame_gap`,
//!   the computed tick, so the result does not depend on how the samples were cut;
//! - a frame spans its first pulse's fall through its last pulse's release; its bits go out
//!   packed most significant bit first, the first bit on the lines in the top bit of the first
//!   byte. A 26-bit frame has both its parity bits checked; other lengths are reported as they
//!   are, their formats being the reader's;
//! - both lines low at once, a pulse shorter than `pulse_min_us` or one longer than
//!   `pulse_max_us`, and a 65th bit (frames carry at most [`MAX_BITS`]) end the frame with an
//!   `error` instead: a both-lines error at the tick both were first low, a short pulse or a
//!   65th bit at the release, a long pulse once it has outlasted `pulse_max_us` — at
//!   `fall + pulse_max`. The error spans the frame from its first pulse; the pulses after it up
//!   to the next silence belong to the broken frame and are skipped;
//! - the decoder knows nothing of what came before the first sample, or before the first sample
//!   after a gap: a frame may be under way there. It begins reading frames once the lines have
//!   been seen high for more than `frame_gap_us`, and skips the pulses before that;
//! - a gap or the end of the samples before a frame's silence has passed cuts it off: an
//!   `error { reason: aborted }` at the gap's start or the end, spanning the frame's pulses
//!   through the last edge seen. The gap or the end is passed on after it;
//! - progress is passed on, but never past the first pulse of a frame still being read.
//!
//! The input requires two samples in the shortest pulse, a `rateHz` of at least
//! ⌈2 · 10⁶ / `pulse_min_us`⌉ — 100 kHz by default. The spacing of the pulses is not checked: a
//! frame is any run of pulses closer together than the frame gap. What the bits mean beyond the
//! 26-bit parities — facility codes, card numbers, other formats — is the reader's business.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use uiharu_core::{
    Change, ChannelMask, InputRef, InputSpec, Io, Levels, LogicBlock, MAX_CHANNELS, Node,
    NodeFuture, NodeId, NodeSpec, Outbox, OutputRef, OutputSpec, Props, Provenance, Requires,
    Sample, SignalCursor, Tick, TickRange,
};

use super::events::{ErrorReason, FrameId, MAX_BITS, WiegandEvent};

/// How the lines are wired and timed. A description may leave fields out: they take the
/// defaults.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct WiegandConfig {
    /// The DATA0 channel, whose pulses are 0 bits.
    pub d0: u8,
    /// The DATA1 channel, whose pulses are 1 bits.
    pub d1: u8,
    /// How long the lines stay high after a frame's last pulse before the frame ends, in
    /// microseconds; above zero.
    pub frame_gap_us: u32,
    /// The shortest pulse, in microseconds; above zero.
    pub pulse_min_us: u32,
    /// The longest pulse, in microseconds; at least `pulse_min_us`.
    pub pulse_max_us: u32,
}

impl Default for WiegandConfig {
    fn default() -> Self {
        Self {
            d0: 0,
            d1: 1,
            frame_gap_us: 10_000,
            pulse_min_us: 20,
            pulse_max_us: 100,
        }
    }
}

impl WiegandConfig {
    /// Checks the configuration.
    pub fn validate(&self) -> Result<(), WiegandConfigError> {
        for channel in [self.d0, self.d1] {
            if channel >= MAX_CHANNELS {
                return Err(WiegandConfigError::Channel(channel));
            }
        }
        if self.d0 == self.d1 {
            return Err(WiegandConfigError::DuplicateChannel(self.d0));
        }
        if self.frame_gap_us == 0 {
            return Err(WiegandConfigError::FrameGap);
        }
        if self.pulse_min_us == 0 || self.pulse_min_us > self.pulse_max_us {
            return Err(WiegandConfigError::PulseWidth {
                min: self.pulse_min_us,
                max: self.pulse_max_us,
            });
        }
        Ok(())
    }

    /// What the decoder's output port declares: the frame gap, and the most bits a frame
    /// carries.
    pub fn props(&self) -> Props {
        Props::new()
            .with("frameGapUs", i64::from(self.frame_gap_us))
            .with("maxBits", i64::from(MAX_BITS))
    }

    /// The lowest sample rate the decoder takes: two samples in the shortest pulse.
    pub fn min_rate_hz(&self) -> u64 {
        2_000_000u64.div_ceil(u64::from(self.pulse_min_us.max(1)))
    }

    /// What the decoder's input requires of the source: both channels, and a sample rate that
    /// sees the shortest pulse.
    pub fn requires(&self) -> Requires {
        Requires::new()
            .at_least("channels", i64::from(self.d0.max(self.d1)) + 1)
            .at_least(
                "rateHz",
                i64::try_from(self.min_rate_hz()).unwrap_or(i64::MAX),
            )
    }
}

/// Why a configuration is rejected.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum WiegandConfigError {
    /// A channel number is out of range.
    #[error("channel {0} is out of range; channels are 0 to {last}", last = MAX_CHANNELS - 1)]
    Channel(u8),
    /// DATA0 and DATA1 are the same channel.
    #[error("channel {0} is used for both DATA0 and DATA1")]
    DuplicateChannel(u8),
    /// The frame gap is zero.
    #[error("the frame gap must be above zero")]
    FrameGap,
    /// The pulse width window is empty or starts at zero.
    #[error("pulses of {min} to {max} µs: the shortest must be above zero and at most the longest")]
    PulseWidth {
        /// The shortest pulse, in microseconds.
        min: u32,
        /// The longest pulse, in microseconds.
        max: u32,
    },
}

/// The Wiegand decoder node: logic samples in, `wiegand.events` out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Wiegand {
    config: WiegandConfig,
}

impl Wiegand {
    /// A decoder for `config`.
    pub fn new(config: WiegandConfig) -> Result<Self, WiegandConfigError> {
        config.validate()?;
        Ok(Self { config })
    }

    /// The configuration.
    pub fn config(&self) -> &WiegandConfig {
        &self.config
    }

    /// The logic input of the decoder added as `node`.
    pub fn input(node: NodeId) -> InputRef<Arc<LogicBlock>> {
        InputRef::of_node(node, 0)
    }

    /// The event output of the decoder added as `node`.
    pub fn events(node: NodeId) -> OutputRef<WiegandEvent> {
        OutputRef::of_node(node, 0)
    }
}

impl Node for Wiegand {
    fn spec(&self) -> NodeSpec {
        NodeSpec {
            name: "wiegand".into(),
            inputs: vec![InputSpec::new::<Arc<LogicBlock>>(
                "logic",
                self.config.requires(),
            )],
            outputs: vec![OutputSpec::new::<WiegandEvent>(
                "events",
                self.config.props(),
            )],
        }
    }

    fn start(self: Box<Self>, mut io: Io) -> NodeFuture {
        // The input's requirement guarantees a rate; without one every window is empty.
        let rate_hz = io.time_base().map_or(0, |base| base.rate_hz);
        let config = self.config;
        Box::pin(run(Reader {
            signal: io.signal(0),
            out: io.output(0),
            d0: config.d0,
            d1: config.d1,
            mask: ChannelMask::one(config.d0).with(config.d1),
            windows: Windows::new(&config, rate_hz),
            next_frame: 1,
            stopped: None,
        }))
    }
}

/// The most whole ticks within `us` microseconds.
fn at_most(us: u32, rate_hz: u64) -> u64 {
    let ticks = u128::from(us) * u128::from(rate_hz) / 1_000_000;
    u64::try_from(ticks).unwrap_or(u64::MAX)
}

/// The fewest whole ticks that last at least `us` microseconds.
fn at_least(us: u32, rate_hz: u64) -> u64 {
    let ticks = (u128::from(us) * u128::from(rate_hz)).div_ceil(1_000_000);
    u64::try_from(ticks).unwrap_or(u64::MAX)
}

/// The configuration's times in ticks of the source.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Windows {
    /// The shortest pulse.
    pulse_min: u64,
    /// The longest pulse.
    pulse_max: u64,
    /// The longest silence inside a frame.
    frame_gap: u64,
}

impl Windows {
    fn new(config: &WiegandConfig, rate_hz: u64) -> Self {
        Self {
            pulse_min: at_least(config.pulse_min_us, rate_hz),
            pulse_max: at_most(config.pulse_max_us, rate_hz),
            frame_gap: at_most(config.frame_gap_us, rate_hz),
        }
    }
}

/// A frame being read: at most [`MAX_BITS`] bits, the first in the highest place.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Frame {
    /// The first pulse's fall.
    start: Tick,
    bits: u16,
    value: u64,
    /// The last pulse's release.
    release: Tick,
}

impl Frame {
    fn new(start: Tick) -> Self {
        Self {
            start,
            bits: 0,
            value: 0,
            release: start,
        }
    }

    /// The bits packed most significant bit first, the first bit in the top bit of the first
    /// byte.
    fn data(&self) -> Vec<u8> {
        let bits = u32::from(self.bits.min(MAX_BITS));
        let aligned = self.value.checked_shl(64 - bits).unwrap_or(0);
        let bytes = aligned.to_be_bytes();
        bytes[..bits.div_ceil(8) as usize].to_vec()
    }

    /// For a 26-bit frame, whether the first bit makes the first 13 even and the last bit the
    /// last 13 odd.
    fn parity_ok(&self) -> Option<bool> {
        (self.bits == 26).then(|| {
            let first = self.value >> 13;
            let last = self.value & 0x1fff;
            first.count_ones().is_multiple_of(2) && !last.count_ones().is_multiple_of(2)
        })
    }
}

/// Where the decoder stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    /// Waiting for a silence longer than the frame gap; `since` is where both lines were last
    /// seen to turn high, `None` while one is low.
    Skip { since: Option<Tick> },
    /// No frame under way, both lines high.
    Idle,
    /// A frame under way, both lines high since its last release.
    Open(Frame),
    /// A pulse under way on `line`, which fell at `fall`.
    Pulse { frame: Frame, line: u8, fall: Tick },
}

impl State {
    /// Where the event the state may still emit begins: progress must not pass it.
    fn hold(&self) -> Option<Tick> {
        match self {
            State::Skip { .. } | State::Idle => None,
            State::Open(frame) | State::Pulse { frame, .. } => Some(frame.start),
        }
    }
}

/// Samples ran out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Cut {
    Gap(TickRange),
    End(Tick),
}

impl Cut {
    fn tick(self) -> Tick {
        match self {
            Cut::Gap(range) => range.start,
            Cut::End(tick) => tick,
        }
    }
}

/// What a wait found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Waited {
    /// The lines changed at this tick, to these levels.
    Edge(Tick, Levels),
    /// Neither line changed before the deadline.
    Deadline,
}

struct Reader {
    signal: SignalCursor,
    out: Outbox<WiegandEvent>,
    d0: u8,
    d1: u8,
    mask: ChannelMask,
    windows: Windows,
    next_frame: u64,
    /// A gap or the end of the samples found at or past a deadline, for the next wait.
    stopped: Option<Cut>,
}

impl Reader {
    /// Waits for a line to change, before `until` if given; passes progress on, held at
    /// `hold`. Samples that reach the deadline with no change are the deadline passing, however
    /// the cursor learns it: by `Quiet`, by progress at or past it, or by a gap or the end of
    /// the samples that comes no earlier, which is then kept for the next wait.
    async fn wait(&mut self, until: Option<Tick>, hold: Option<Tick>) -> Result<Waited, Cut> {
        if let Some(cut) = self.stopped.take() {
            return Err(cut);
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
                Change::Progress(tick) => {
                    let held = hold.map_or(tick, |hold| hold.min(tick));
                    self.out.progress(held).await;
                    if let Some(until) = until
                        && tick >= until
                    {
                        self.signal.seek(until);
                        return Ok(Waited::Deadline);
                    }
                }
                Change::Gap(range) if reached(range.start) => {
                    self.stopped = Some(Cut::Gap(range));
                    return Ok(Waited::Deadline);
                }
                Change::End(tick) if reached(tick) => {
                    self.stopped = Some(Cut::End(tick));
                    return Ok(Waited::Deadline);
                }
                Change::Gap(range) => return Err(Cut::Gap(range)),
                Change::End(tick) => return Err(Cut::End(tick)),
            }
        }
    }

    fn both_high(&self, levels: Levels) -> bool {
        levels.high(self.d0) && levels.high(self.d1)
    }

    async fn error(&mut self, frame: &Frame, end: Tick, settled: Tick, reason: ErrorReason) {
        let at = Provenance::over(TickRange::new(frame.start, end), settled);
        self.out.emit(at, WiegandEvent::Error { reason }).await;
    }

    /// Reports `frame`, whose silence passed at `settled`.
    async fn frame(&mut self, frame: &Frame, settled: Tick) {
        let id = FrameId(self.next_frame);
        self.next_frame += 1;
        let event = WiegandEvent::Frame {
            frame: id,
            bits: frame.bits,
            data: frame.data(),
            parity_ok: frame.parity_ok(),
        };
        let span = TickRange::new(frame.start, frame.release.next());
        self.out.emit(Provenance::over(span, settled), event).await;
    }

    /// The lines changed at `tick` from both high, with `frame` under way (or begun there).
    async fn fall(&mut self, frame: Frame, tick: Tick, levels: Levels) -> State {
        match (levels.high(self.d0), levels.high(self.d1)) {
            (false, false) => {
                self.error(&frame, tick.next(), tick, ErrorReason::BothLines)
                    .await;
                State::Skip { since: None }
            }
            (false, true) => State::Pulse {
                frame,
                line: self.d0,
                fall: tick,
            },
            (true, false) => State::Pulse {
                frame,
                line: self.d1,
                fall: tick,
            },
            // A line that was high cannot have changed to high.
            (true, true) if frame.bits == 0 => State::Idle,
            (true, true) => State::Open(frame),
        }
    }

    /// Follows the lines from `state` to the next state, reporting what it completes.
    async fn step(&mut self, state: State) -> Result<State, Cut> {
        let windows = self.windows;
        let hold = state.hold();
        match state {
            State::Skip { since: None } => match self.wait(None, hold).await? {
                Waited::Edge(tick, levels) => Ok(State::Skip {
                    since: self.both_high(levels).then_some(tick),
                }),
                Waited::Deadline => Ok(state),
            },
            State::Skip { since: Some(since) } => {
                let until = since.offset(windows.frame_gap).next();
                match self.wait(Some(until), hold).await? {
                    Waited::Deadline => Ok(State::Idle),
                    Waited::Edge(tick, levels) => Ok(State::Skip {
                        since: self.both_high(levels).then_some(tick),
                    }),
                }
            }
            State::Idle => match self.wait(None, hold).await? {
                Waited::Edge(tick, levels) => Ok(self.fall(Frame::new(tick), tick, levels).await),
                Waited::Deadline => Ok(state),
            },
            State::Open(frame) => {
                let silence = frame.release.offset(windows.frame_gap);
                match self.wait(Some(silence.next()), hold).await? {
                    Waited::Deadline => {
                        self.frame(&frame, silence).await;
                        Ok(State::Idle)
                    }
                    Waited::Edge(tick, levels) => Ok(self.fall(frame, tick, levels).await),
                }
            }
            State::Pulse {
                mut frame,
                line,
                fall,
            } => {
                let longest = fall.offset(windows.pulse_max);
                let (tick, levels) = match self.wait(Some(longest.next()), hold).await? {
                    Waited::Edge(tick, levels) => (tick, levels),
                    Waited::Deadline => {
                        self.error(&frame, longest.next(), longest, ErrorReason::LongPulse)
                            .await;
                        return Ok(State::Skip { since: None });
                    }
                };
                let other = if line == self.d0 { self.d1 } else { self.d0 };
                if !levels.high(line) {
                    // The pulse goes on and the other line fell: both are low.
                    self.error(&frame, tick.next(), tick, ErrorReason::BothLines)
                        .await;
                    return Ok(State::Skip { since: None });
                }
                let other_high = levels.high(other);
                let reason = if fall.until(tick) < windows.pulse_min {
                    Some(ErrorReason::ShortPulse)
                } else if frame.bits >= MAX_BITS {
                    Some(ErrorReason::TooLong)
                } else {
                    None
                };
                if let Some(reason) = reason {
                    self.error(&frame, tick.next(), tick, reason).await;
                    return Ok(State::Skip {
                        since: other_high.then_some(tick),
                    });
                }
                frame.value = (frame.value << 1) | u64::from(line == self.d1);
                frame.bits += 1;
                frame.release = tick;
                if other_high {
                    Ok(State::Open(frame))
                } else {
                    // The other line fell as this one rose: the next pulse begins at once.
                    Ok(State::Pulse {
                        frame,
                        line: other,
                        fall: tick,
                    })
                }
            }
        }
    }

    /// Reports the frame `state` had under way when the samples ran out at `cut`.
    async fn cut_off(&mut self, state: State, cut: Cut) {
        let (frame, last) = match state {
            State::Open(frame) => (frame, frame.release),
            State::Pulse { frame, fall, .. } => (frame, fall),
            State::Skip { .. } | State::Idle => return,
        };
        self.error(&frame, last.next(), cut.tick(), ErrorReason::Aborted)
            .await;
    }
}

async fn run(mut reader: Reader) {
    // Each round of this loop starts where nothing is known of the lines: at the first sample
    // and after every gap.
    'resync: loop {
        let mut state = loop {
            match reader.signal.sample().await {
                Sample::At { tick, levels } => {
                    break State::Skip {
                        since: reader.both_high(levels).then_some(tick),
                    };
                }
                Sample::Gap(range) => reader.out.gap(range).await,
                Sample::Progress(tick) => reader.out.progress(tick).await,
                Sample::End(tick) => {
                    reader.out.end(tick).await;
                    return;
                }
            }
        };
        loop {
            state = match reader.step(state).await {
                Ok(next) => next,
                Err(cut) => {
                    reader.cut_off(state, cut).await;
                    match cut {
                        Cut::Gap(range) => {
                            reader.out.gap(range).await;
                            continue 'resync;
                        }
                        Cut::End(tick) => {
                            reader.out.end(tick).await;
                            return;
                        }
                    }
                }
            };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configurations_are_checked() {
        assert_eq!(WiegandConfig::default().validate(), Ok(()));
        let bad = |config: WiegandConfig| config.validate().expect_err("rejected");
        assert_eq!(
            bad(WiegandConfig {
                d1: 64,
                ..WiegandConfig::default()
            }),
            WiegandConfigError::Channel(64)
        );
        assert_eq!(
            bad(WiegandConfig {
                d0: 3,
                d1: 3,
                ..WiegandConfig::default()
            }),
            WiegandConfigError::DuplicateChannel(3)
        );
        assert_eq!(
            bad(WiegandConfig {
                frame_gap_us: 0,
                ..WiegandConfig::default()
            }),
            WiegandConfigError::FrameGap
        );
        assert_eq!(
            bad(WiegandConfig {
                pulse_min_us: 0,
                ..WiegandConfig::default()
            }),
            WiegandConfigError::PulseWidth { min: 0, max: 100 }
        );
        assert_eq!(
            bad(WiegandConfig {
                pulse_min_us: 120,
                ..WiegandConfig::default()
            }),
            WiegandConfigError::PulseWidth { min: 120, max: 100 }
        );
        assert!(Wiegand::new(WiegandConfig::default()).is_ok());
    }

    #[test]
    fn descriptions_may_leave_fields_out() {
        let config: WiegandConfig =
            serde_json::from_str(r#"{"d0": 4, "frame_gap_us": 25000}"#).expect("parses");
        assert_eq!(
            config,
            WiegandConfig {
                d0: 4,
                frame_gap_us: 25_000,
                ..WiegandConfig::default()
            }
        );
        let whole: WiegandConfig = serde_json::from_str("{}").expect("parses");
        assert_eq!(whole, WiegandConfig::default());
        assert!(serde_json::from_str::<WiegandConfig>(r#"{"data0": 0}"#).is_err());
    }

    #[test]
    fn ports_declare_the_frames_and_require_a_fine_enough_rate() {
        let config = WiegandConfig {
            d0: 5,
            d1: 2,
            ..WiegandConfig::default()
        };
        let props = config.props();
        assert_eq!(props.get("frameGapUs"), Some(&10_000i64.into()));
        assert_eq!(props.get("maxBits"), Some(&64i64.into()));
        let source =
            |channels: i64, rate: i64| Props::new().with("channels", channels).with("rateHz", rate);
        let requires = Wiegand::new(config).expect("valid").spec().inputs[0]
            .requires
            .clone();
        // Two samples in the shortest pulse of 20 µs: 100 kHz.
        assert!(requires.unmet(&source(6, 100_000)).is_none());
        assert_eq!(
            requires.unmet(&source(5, 100_000)).map(|(key, _)| key),
            Some("channels")
        );
        assert_eq!(
            requires.unmet(&source(6, 99_999)).map(|(key, _)| key),
            Some("rateHz")
        );
        let narrow = WiegandConfig {
            pulse_min_us: 3,
            ..WiegandConfig::default()
        };
        assert_eq!(narrow.min_rate_hz(), 666_667);
    }

    #[test]
    fn windows_round_toward_the_bound() {
        let config = WiegandConfig::default();
        let windows = Windows::new(&config, 1_000_000);
        assert_eq!(
            windows,
            Windows {
                pulse_min: 20,
                pulse_max: 100,
                frame_gap: 10_000
            }
        );
        // 333 kHz: a lower bound rounds up, an upper bound down.
        let odd = Windows::new(&config, 333_000);
        assert_eq!(odd.pulse_min, 7); // 6.66 ticks
        assert_eq!(odd.pulse_max, 33); // 33.3 ticks
        assert_eq!(odd.frame_gap, 3_330);
    }

    #[test]
    fn frames_pack_their_bits_from_the_top() {
        let mut frame = Frame::new(Tick(0));
        for bit in [true, false, true, true] {
            frame.value = (frame.value << 1) | u64::from(bit);
            frame.bits += 1;
        }
        assert_eq!(frame.data(), vec![0xb0]);
        assert_eq!(frame.parity_ok(), None);
        frame.bits = 64;
        frame.value = 0x0123_4567_89ab_cdef;
        assert_eq!(
            frame.data(),
            vec![0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef]
        );
        // Facility 123, card 12345, between an even parity bit 0 and an odd parity bit 1:
        // 0 01111011 0011000000111001 1.
        let mut card = Frame::new(Tick(0));
        card.bits = 26;
        card.value = 0b00_1111_0110_0110_0000_0111_0011;
        assert_eq!(card.data(), vec![0x3d, 0x98, 0x1c, 0xc0]);
        assert_eq!(card.parity_ok(), Some(true));
        // Either parity bit wrong fails the check.
        for flip in [1u64 << 25, 1] {
            card.value ^= flip;
            assert_eq!(card.parity_ok(), Some(false));
            card.value ^= flip;
        }
        assert_eq!(card.parity_ok(), Some(true));
    }
}
