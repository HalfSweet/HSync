// SPDX-License-Identifier: Apache-2.0

//! The 1-Wire link layer: resets, presence pulses and time slots, read from the data line.
//!
//! Implemented from Maxim Integrated (Analog Devices) application note 126, "1-Wire
//! Communication Through Software", application note 937, "Book of iButton Standards", and the
//! DS2401, DS2431 and DS18B20 datasheets. The line idles high and every exchange begins with
//! the master pulling it low; the decoder classifies each low pulse by how long it lasts:
//!
//! - a low released by the slot's sampling point — 15 µs after the fall, 2 µs at overdrive —
//!   is a 1, provided the line is still high at that point;
//! - a low held past the sampling point and released by the end of the longest slot — 120 µs,
//!   16 µs at overdrive — is a 0. A write-0 lasts 60 to 120 µs (6 to 16 µs), but in a read
//!   slot the slave holds a 0 from the master's fall until anywhere between the sampling point
//!   and the end of the slot, so the window for a 0 starts at the sampling point;
//! - read and write slots look alike on the line — a read-1 is a short pulse like a write-1, a
//!   read-0 a long one like a write-0 — so the link layer reports bits, not who drove them;
//! - a low of at least 480 µs is a reset. At overdrive a reset lasts 48 to 80 µs, and a low of
//!   at least 480 µs is still a reset: the standard-speed one, which the slaves answer at
//!   standard speed;
//! - after a reset, a presence pulse is the line pulled low by the slaves 15 to 60 µs after the
//!   master released it (2 to 6 µs at overdrive), lasting at least 60 µs (8 µs) and released no
//!   later than 300 µs (30 µs) after the master released: the latest a single slave's pulse of
//!   at most 240 µs (24 µs) can end, which leaves room for the pulses of several slaves to
//!   merge. The first fall in the presence window decides: when it is no presence pulse, the
//!   reset is reported without presence and that pulse as a bad slot (or as a reset, if it is
//!   one), never as a bit;
//! - any other low is a bad slot: longer than a slot without being a reset (at overdrive, 16 to
//!   48 µs or 80 to 480 µs), or in a presence window without being a presence pulse; so is a 1
//!   whose sampling point falls into the next pulse;
//! - a low found at the first sample or after a gap began unseen and is dropped, unless what
//!   was seen of it is already a standard-speed reset, which is then reported from the first
//!   sample seen;
//! - a pulse, or a reset's presence window, cut off by a gap or the end of the samples is
//!   dropped — a reset whose presence could not be seen is not reported — and the gap or the
//!   end is passed on.
//!
//! The speed is configured: the link layer cannot see the Overdrive Skip ROM or Overdrive
//! Match ROM command that switches the slaves to overdrive, nor the standard-speed reset that
//! switches them back, so a capture that changes speed decodes correctly only where it runs at
//! the configured speed.
//!
//! A low pulse is measured in ticks, from the tick the line falls to the tick it rises, and
//! compared with the windows exactly: a bound of "at most `t` µs" is ⌊`t` · rate / 10⁶⌋ ticks,
//! one of "at least `t` µs" ⌈`t` · rate / 10⁶⌉ ticks, both in integer arithmetic. The input
//! requires at least 1 MHz at standard speed — one sample per microsecond, the shortest write-1
//! the standard allows — and 10 MHz at overdrive, whose windows are a few microseconds wide.
//!
//! Every event is placed on the ticks it covers: a bit spans its fall through the later of its
//! release and its sampling point, and is settled there; a reset spans its fall through the
//! end of the presence pulse, or through the end of the presence window when no slave
//! answered, and is settled there; a bad slot spans its low pulse and its release. Progress is
//! passed on, but never past the start of the pulse under way — its fall or, for a low already
//! under way where the samples begin or resume, the first sample seen — until the pulse is
//! reported or dropped, which for a reset comes after its presence window; so no event that
//! follows starts before the progress passed on.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use uiharu_core::{
    Change, ChannelMask, InputRef, InputSpec, Io, LogicBlock, MAX_CHANNELS, Node, NodeFuture,
    NodeId, NodeSpec, Outbox, OutputRef, OutputSpec, Props, Provenance, Requires, Sample,
    SignalCursor, Tick, TickRange,
};

use super::events::LinkEvent;

/// The speed the bus runs at.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Speed {
    /// Standard speed: slots of 60 to 120 µs, resets of at least 480 µs.
    #[default]
    Standard,
    /// Overdrive: slots of 6 to 16 µs, resets of 48 to 80 µs.
    Overdrive,
}

impl Speed {
    /// The name the port properties use.
    pub const fn name(self) -> &'static str {
        match self {
            Speed::Standard => "standard",
            Speed::Overdrive => "overdrive",
        }
    }

    /// The lowest sample rate the decoder takes at this speed.
    pub const fn min_rate_hz(self) -> u64 {
        match self {
            Speed::Standard => 1_000_000,
            Speed::Overdrive => 10_000_000,
        }
    }
}

/// How the bus is wired and how fast it runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LinkConfig {
    /// The data line's channel.
    pub data: u8,
    /// The speed the bus runs at.
    pub speed: Speed,
}

impl Default for LinkConfig {
    fn default() -> Self {
        Self {
            data: 0,
            speed: Speed::Standard,
        }
    }
}

impl LinkConfig {
    /// Checks the configuration.
    pub fn validate(&self) -> Result<(), LinkConfigError> {
        if self.data >= MAX_CHANNELS {
            return Err(LinkConfigError::Channel(self.data));
        }
        Ok(())
    }

    /// What the decoder's output port declares.
    pub fn props(&self) -> Props {
        Props::new().with("speed", self.speed.name())
    }

    /// What the decoder's input requires of the source: the data channel, and a sample rate
    /// fine enough for the speed's windows.
    pub fn requires(&self) -> Requires {
        Requires::new()
            .at_least("channels", i64::from(self.data) + 1)
            .at_least(
                "rateHz",
                i64::try_from(self.speed.min_rate_hz()).unwrap_or(i64::MAX),
            )
    }
}

/// Why a configuration is rejected.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum LinkConfigError {
    /// The channel number is out of range.
    #[error("channel {0} is out of range; channels are 0 to {last}", last = MAX_CHANNELS - 1)]
    Channel(u8),
}

/// The 1-Wire link-layer node: logic samples in, `onewire.link` out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OneWireLink {
    config: LinkConfig,
}

impl OneWireLink {
    /// A decoder for `config`.
    pub fn new(config: LinkConfig) -> Result<Self, LinkConfigError> {
        config.validate()?;
        Ok(Self { config })
    }

    /// The configuration.
    pub fn config(&self) -> &LinkConfig {
        &self.config
    }

    /// The logic input of the decoder added as `node`.
    pub fn input(node: NodeId) -> InputRef<Arc<LogicBlock>> {
        InputRef::of_node(node, 0)
    }

    /// The event output of the decoder added as `node`.
    pub fn events(node: NodeId) -> OutputRef<LinkEvent> {
        OutputRef::of_node(node, 0)
    }
}

impl Node for OneWireLink {
    fn spec(&self) -> NodeSpec {
        NodeSpec {
            name: "onewire-link".into(),
            inputs: vec![InputSpec::new::<Arc<LogicBlock>>(
                "logic",
                self.config.requires(),
            )],
            outputs: vec![OutputSpec::new::<LinkEvent>("events", self.config.props())],
        }
    }

    fn start(self: Box<Self>, mut io: Io) -> NodeFuture {
        // The input's requirement guarantees a rate; without one every window is empty.
        let rate_hz = io.time_base().map_or(0, |base| base.rate_hz);
        let windows = Windows::new(self.config.speed, rate_hz);
        Box::pin(run(Link {
            signal: io.signal(0),
            out: io.output(0),
            data: self.config.data,
            mask: ChannelMask::one(self.config.data),
            windows,
            under_way: None,
            held_progress: None,
            pending_gap: None,
        }))
    }
}

/// The most whole ticks within `us` microseconds.
fn at_most(us: u64, rate_hz: u64) -> u64 {
    let ticks = u128::from(us) * u128::from(rate_hz) / 1_000_000;
    u64::try_from(ticks).unwrap_or(u64::MAX)
}

/// The fewest whole ticks that last at least `us` microseconds.
fn at_least(us: u64, rate_hz: u64) -> u64 {
    let ticks = (u128::from(us) * u128::from(rate_hz)).div_ceil(1_000_000);
    u64::try_from(ticks).unwrap_or(u64::MAX)
}

/// Where a presence pulse may lie, in ticks after the master released the reset.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PresenceWindow {
    /// The earliest fall.
    fall_from: u64,
    /// The latest fall.
    fall_until: u64,
    /// The shortest low.
    low_min: u64,
    /// The latest release.
    release_until: u64,
}

impl PresenceWindow {
    /// The window whose bounds are, in microseconds, `[fall_from, fall_until, low_min,
    /// release_until]`, in ticks at `rate_hz`.
    fn new(us: [u64; 4], rate_hz: u64) -> Self {
        let [fall_from, fall_until, low_min, release_until] = us;
        Self {
            fall_from: at_least(fall_from, rate_hz),
            fall_until: at_most(fall_until, rate_hz),
            low_min: at_least(low_min, rate_hz),
            release_until: at_most(release_until, rate_hz),
        }
    }
}

/// The overdrive reset and the presence window after it, in ticks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct OverdriveReset {
    shortest: u64,
    longest: u64,
    presence: PresenceWindow,
}

/// The windows of the configured speed, in ticks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Windows {
    /// The longest low that is a 1: the sampling point, counted from the fall.
    sample: u64,
    /// The longest low that is a 0.
    slot_max: u64,
    /// At overdrive, the overdrive reset.
    overdrive_reset: Option<OverdriveReset>,
    /// The shortest low that is a standard-speed reset.
    standard_reset: u64,
    /// The presence window after a standard-speed reset.
    standard_presence: PresenceWindow,
}

/// What a low pulse is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Pulse {
    One,
    Zero,
    Reset(PresenceWindow),
    Bad,
}

impl Windows {
    fn new(speed: Speed, rate_hz: u64) -> Self {
        let standard_presence = PresenceWindow::new([15, 60, 60, 300], rate_hz);
        let standard_reset = at_least(480, rate_hz);
        match speed {
            Speed::Standard => Self {
                sample: at_most(15, rate_hz),
                slot_max: at_most(120, rate_hz),
                overdrive_reset: None,
                standard_reset,
                standard_presence,
            },
            Speed::Overdrive => Self {
                sample: at_most(2, rate_hz),
                slot_max: at_most(16, rate_hz),
                overdrive_reset: Some(OverdriveReset {
                    shortest: at_least(48, rate_hz),
                    longest: at_most(80, rate_hz),
                    presence: PresenceWindow::new([2, 6, 8, 30], rate_hz),
                }),
                standard_reset,
                standard_presence,
            },
        }
    }

    fn classify(&self, low: u64) -> Pulse {
        if low <= self.sample {
            return Pulse::One;
        }
        if low <= self.slot_max {
            return Pulse::Zero;
        }
        if let Some(reset) = self.overdrive_reset
            && (reset.shortest..=reset.longest).contains(&low)
        {
            return Pulse::Reset(reset.presence);
        }
        if low >= self.standard_reset {
            return Pulse::Reset(self.standard_presence);
        }
        Pulse::Bad
    }
}

/// Samples ran out under a pulse.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Cut {
    Gap(TickRange),
    End(Tick),
}

/// Where the decoder stands on the line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    /// The line is high.
    Idle,
    /// The line fell at `fall`; `slot` says whether the pulse may be a time slot, which a
    /// pulse in a presence window may not.
    Low { fall: Tick, slot: bool },
    /// The line was found low at this tick, the first sample or the first after a gap.
    Unseen(Tick),
}

impl State {
    /// Where the low pulse the state follows began; `None` while the line is high.
    fn start(self) -> Option<Tick> {
        match self {
            State::Idle => None,
            State::Low { fall, .. } => Some(fall),
            State::Unseen(from) => Some(from),
        }
    }
}

struct Link {
    signal: SignalCursor,
    out: Outbox<LinkEvent>,
    data: u8,
    mask: ChannelMask,
    windows: Windows,
    /// Where the pulse being followed began, while one is: whatever it turns out to be starts
    /// there, so progress goes no further.
    under_way: Option<Tick>,
    /// Progress the last timed wait saw past its deadline, passed on after the result it
    /// would otherwise precede.
    held_progress: Option<Tick>,
    /// A gap the last timed wait found at its deadline, handled after the result.
    pending_gap: Option<TickRange>,
}

impl Link {
    async fn emit(&mut self, span: TickRange, settled: Tick, event: LinkEvent) {
        self.out.emit(Provenance::over(span, settled), event).await;
    }

    /// Passes progress to `tick` on, held back to the start of the pulse under way.
    async fn progress(&mut self, tick: Tick) {
        let tick = self.under_way.map_or(tick, |start| start.min(tick));
        self.out.progress(tick).await;
    }

    /// Passes on the progress the last timed wait held back, and hands out the gap it found.
    async fn flush(&mut self) -> Result<(), Cut> {
        if let Some(tick) = self.held_progress.take() {
            self.progress(tick).await;
        }
        self.pending_gap
            .take()
            .map_or(Ok(()), |range| Err(Cut::Gap(range)))
    }

    /// Waits for the line to change level; passes progress on.
    async fn edge(&mut self) -> Result<Tick, Cut> {
        self.flush().await?;
        loop {
            match self.signal.next_change(self.mask).await {
                Change::At { tick, .. } => return Ok(tick),
                Change::Progress(tick) => self.progress(tick).await,
                Change::Gap(range) => return Err(Cut::Gap(range)),
                Change::End(tick) => return Err(Cut::End(tick)),
                // Without a deadline the wait is never quiet.
                Change::Quiet(_) => {}
            }
        }
    }

    /// Waits for the line to change level before `deadline`; `None` when it held its level
    /// through the tick before.
    async fn edge_before(&mut self, deadline: Tick) -> Result<Option<Tick>, Cut> {
        self.flush().await?;
        loop {
            match self.signal.next_change_before(self.mask, deadline).await {
                Change::At { tick, .. } => return Ok(Some(tick)),
                Change::Quiet(_) => return Ok(None),
                Change::Progress(tick) if tick < deadline => self.progress(tick).await,
                // Every sample before `tick` was seen without a change: the deadline passed.
                Change::Progress(tick) => {
                    self.held_progress = Some(tick);
                    self.signal.seek(deadline);
                    return Ok(None);
                }
                Change::Gap(range) if range.start >= deadline => {
                    self.pending_gap = Some(range);
                    return Ok(None);
                }
                Change::Gap(range) => return Err(Cut::Gap(range)),
                // The end stays in the stream, for the next wait to find.
                Change::End(tick) if tick >= deadline => return Ok(None),
                Change::End(tick) => return Err(Cut::End(tick)),
            }
        }
    }

    /// Follows the line from `state` to the next state, reporting what it completes.
    async fn step(&mut self, state: State) -> Result<State, Cut> {
        // Nothing this step reports starts before the pulse it follows began.
        self.under_way = state.start();
        match state {
            State::Idle => Ok(State::Low {
                fall: self.edge().await?,
                slot: true,
            }),
            State::Unseen(from) => {
                let rise = self.edge().await?;
                // What was seen is a reset whenever it began; anything shorter cannot be told.
                if from.until(rise) >= self.windows.standard_reset {
                    let window = self.windows.standard_presence;
                    self.presence(from, rise, window).await
                } else {
                    Ok(State::Idle)
                }
            }
            State::Low { fall, slot } => {
                let rise = self.edge().await?;
                let low = fall.until(rise);
                match self.windows.classify(low) {
                    Pulse::One if slot => {
                        let sample = fall.offset(self.windows.sample);
                        match self.edge_before(sample.next()).await? {
                            None => {
                                let span = TickRange::new(fall, sample.next());
                                self.emit(span, sample, LinkEvent::Bit { value: true })
                                    .await;
                                Ok(State::Idle)
                            }
                            // The next pulse began before this slot was sampled.
                            Some(next) => {
                                let span = TickRange::new(fall, next);
                                self.emit(span, next, LinkEvent::BadSlot { low_ticks: low })
                                    .await;
                                Ok(State::Low {
                                    fall: next,
                                    slot: true,
                                })
                            }
                        }
                    }
                    Pulse::Zero if slot => {
                        let span = TickRange::new(fall, rise.next());
                        self.emit(span, rise, LinkEvent::Bit { value: false }).await;
                        Ok(State::Idle)
                    }
                    Pulse::Reset(window) => self.presence(fall, rise, window).await,
                    Pulse::One | Pulse::Zero | Pulse::Bad => {
                        let span = TickRange::new(fall, rise.next());
                        self.emit(span, rise, LinkEvent::BadSlot { low_ticks: low })
                            .await;
                        Ok(State::Idle)
                    }
                }
            }
        }
    }

    /// Looks for the presence pulse after a reset that fell at `fall` and was released at
    /// `release`, and reports the reset.
    async fn presence(
        &mut self,
        fall: Tick,
        release: Tick,
        window: PresenceWindow,
    ) -> Result<State, Cut> {
        let absent = LinkEvent::Reset { presence: false };
        let last_fall = release.offset(window.fall_until);
        let Some(answer) = self.edge_before(last_fall.next()).await? else {
            let span = TickRange::new(fall, last_fall.next());
            self.emit(span, last_fall, absent).await;
            return Ok(State::Idle);
        };
        if release.until(answer) < window.fall_from {
            // Too early for a presence pulse.
            self.emit(TickRange::new(fall, answer), answer, absent)
                .await;
            return Ok(State::Low {
                fall: answer,
                slot: false,
            });
        }
        let last_release = release.offset(window.release_until);
        match self.edge_before(last_release.next()).await? {
            Some(end) if answer.until(end) >= window.low_min => {
                let span = TickRange::new(fall, end.next());
                self.emit(span, end, LinkEvent::Reset { presence: true })
                    .await;
                Ok(State::Idle)
            }
            // Too short for a presence pulse.
            Some(end) => {
                self.emit(TickRange::new(fall, answer), end, absent).await;
                let low_ticks = answer.until(end);
                let span = TickRange::new(answer, end.next());
                self.emit(span, end, LinkEvent::BadSlot { low_ticks }).await;
                Ok(State::Idle)
            }
            // Still low when a presence pulse must have ended.
            None => {
                self.emit(TickRange::new(fall, answer), last_release, absent)
                    .await;
                Ok(State::Low {
                    fall: answer,
                    slot: false,
                })
            }
        }
    }
}

async fn run(mut link: Link) {
    // Each round of this loop starts where the line's level is unknown: at the first sample
    // and after every gap.
    'resync: loop {
        // A pulse cut off by a gap was dropped: nothing is under way until the line is read.
        link.under_way = None;
        let mut state = loop {
            match link.signal.sample().await {
                Sample::At { tick, levels } => {
                    break if levels.high(link.data) {
                        State::Idle
                    } else {
                        State::Unseen(tick)
                    };
                }
                Sample::Gap(range) => link.out.gap(range).await,
                Sample::Progress(tick) => link.progress(tick).await,
                Sample::End(tick) => {
                    link.out.end(tick).await;
                    return;
                }
            }
        };
        loop {
            state = match link.step(state).await {
                Ok(next) => next,
                Err(Cut::Gap(range)) => {
                    link.out.gap(range).await;
                    continue 'resync;
                }
                Err(Cut::End(tick)) => {
                    link.out.end(tick).await;
                    return;
                }
            };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_round_toward_the_bound() {
        // 1 MHz: one tick per microsecond, every bound exact.
        let standard = Windows::new(Speed::Standard, 1_000_000);
        assert_eq!((standard.sample, standard.slot_max), (15, 120));
        assert_eq!(standard.standard_reset, 480);
        assert_eq!(
            standard.standard_presence,
            PresenceWindow {
                fall_from: 15,
                fall_until: 60,
                low_min: 60,
                release_until: 300
            }
        );
        // 3.3 MHz: an upper bound rounds down, a lower bound up.
        let odd = Windows::new(Speed::Standard, 3_300_000);
        assert_eq!(odd.sample, 49); // 49.5 ticks
        assert_eq!(odd.standard_reset, 1584); // exactly 1584
        assert_eq!(odd.standard_presence.fall_from, 50); // 49.5 ticks
        let overdrive = Windows::new(Speed::Overdrive, 10_000_000);
        assert_eq!((overdrive.sample, overdrive.slot_max), (20, 160));
        assert_eq!(
            overdrive.overdrive_reset,
            Some(OverdriveReset {
                shortest: 480,
                longest: 800,
                presence: PresenceWindow {
                    fall_from: 20,
                    fall_until: 60,
                    low_min: 80,
                    release_until: 300
                }
            })
        );
    }

    #[test]
    fn pulses_are_classified_by_their_length() {
        let standard = Windows::new(Speed::Standard, 1_000_000);
        assert_eq!(standard.classify(1), Pulse::One);
        assert_eq!(standard.classify(15), Pulse::One);
        assert_eq!(standard.classify(16), Pulse::Zero);
        assert_eq!(standard.classify(120), Pulse::Zero);
        assert_eq!(standard.classify(121), Pulse::Bad);
        assert_eq!(standard.classify(479), Pulse::Bad);
        assert!(matches!(standard.classify(480), Pulse::Reset(_)));
        let overdrive = Windows::new(Speed::Overdrive, 10_000_000);
        assert_eq!(overdrive.classify(20), Pulse::One);
        assert_eq!(overdrive.classify(160), Pulse::Zero);
        assert_eq!(overdrive.classify(479), Pulse::Bad);
        assert!(matches!(overdrive.classify(480), Pulse::Reset(window) if window.fall_from == 20));
        assert!(matches!(overdrive.classify(800), Pulse::Reset(_)));
        assert_eq!(overdrive.classify(801), Pulse::Bad);
        assert!(
            matches!(overdrive.classify(4800), Pulse::Reset(window) if window.fall_from == 150)
        );
    }

    #[test]
    fn configurations_are_checked() {
        assert_eq!(LinkConfig::default().validate(), Ok(()));
        let bad = LinkConfig {
            data: 64,
            ..LinkConfig::default()
        };
        assert_eq!(bad.validate(), Err(LinkConfigError::Channel(64)));
        assert!(OneWireLink::new(bad).is_err());
    }

    #[test]
    fn ports_declare_the_speed_and_require_a_fine_enough_rate() {
        let config = LinkConfig {
            data: 3,
            speed: Speed::Overdrive,
        };
        assert_eq!(config.props().get("speed"), Some(&"overdrive".into()));
        let requires = config.requires();
        let source =
            |channels: i64, rate: i64| Props::new().with("channels", channels).with("rateHz", rate);
        assert!(requires.unmet(&source(4, 10_000_000)).is_none());
        assert_eq!(
            requires.unmet(&source(3, 10_000_000)).map(|(key, _)| key),
            Some("channels")
        );
        assert_eq!(
            requires.unmet(&source(4, 4_000_000)).map(|(key, _)| key),
            Some("rateHz")
        );
        assert!(
            LinkConfig::default()
                .requires()
                .unmet(&source(1, 1_000_000))
                .is_none()
        );
    }
}
