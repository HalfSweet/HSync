// SPDX-License-Identifier: Apache-2.0

//! The PS/2 decoder: the frames a keyboard or mouse and its host exchange over the clock and
//! data lines, in both directions.
//!
//! Implemented from the IBM PS/2 keyboard and auxiliary device interface, as IBM's Personal
//! System/2 hardware technical references describe it and the widely reproduced application
//! note "The PS/2 Mouse/Keyboard Protocol" (Adam Chapweske) restates it: both lines are open
//! collector and idle high; the device generates the clock, high and low for 30 to 50 µs each;
//! a frame is a start bit (0), eight data bits least significant first, an odd parity bit and a
//! stop bit (1). The device sends by clocking eleven pulses, the host reading each bit while the
//! clock is low. The host sends by inhibiting — holding the clock low for at least 100 µs —,
//! pulling the data line low (request to send) and releasing the clock; the device then clocks
//! the data bits, parity and stop bit in on rising edges and acknowledges by pulling the data
//! line low for one last clock pulse. The device must begin clocking within 15 ms and the
//! packet takes at most 2 ms.
//!
//! The rules:
//!
//! - every level is read at a clock edge, from the samples of that tick: the decoder follows
//!   edges, and the sample rate only bounds its timing;
//! - a clock low of at least 100 µs is the host inhibiting the bus, wherever it comes; released
//!   with the data line low, it is a request to send and begins a host-to-device frame, its
//!   start bit, spanning from the inhibit's fall; released with the data line high, it was an
//!   inhibit alone and nothing is reported;
//! - a device-to-host frame begins at the first clock fall on an idle bus that the clock rises
//!   from within 100 µs; the start bit and the ten bits after it are read on the falling edges,
//!   and the frame is reported at the eleventh, spanning its first falling edge through its
//!   last. A start bit of 1 is an `error { reason: start }` at the first rise;
//! - in a host-to-device frame the device's first ten clock pulses carry the data bits, parity
//!   and stop bit, read on their rising edges; the acknowledge is the data line's level on the
//!   eleventh pulse's falling edge, the host reading what the device drives as it reads a
//!   device's bits. The frame is reported there, spanning the inhibit's fall through it. When
//!   no eleventh pulse comes within the pause below, the frame is reported unacknowledged once
//!   the pause has passed;
//! - parity is odd over the eight data bits and the parity bit; the stop bit must be 1. A
//!   frame with a wrong parity or stop bit is still reported, with `parityOk` or `stopOk` false;
//! - a frame is aborted, `error { reason: aborted }`, when 2 ms pass after one of its clock
//!   edges without the next (a device sends a whole packet within 2 ms), when the host holds a
//!   clock low for the inhibit time before the frame's last pulse, when the device does not
//!   begin clocking within 15 ms of the host releasing the clock after a request to send, and
//!   when a gap or the end of the samples cuts it off. The error spans the frame from its start
//!   through its last clock edge and is settled where the abort was decided: at the computed
//!   tick for a time limit, at the gap's start or the end of the samples otherwise. An inhibit
//!   that aborts a frame can itself be a request to send;
//! - the decoder knows nothing of what came before the first sample, or before the first sample
//!   after a gap: a frame may be under way there. It begins reading frames once both lines have
//!   been high for 100 µs — twice the longest half period of the clock — and skips the clock
//!   pulses before that; it does the same after a start error or a frame whose stop bit was 0,
//!   whose bits may be misaligned. A host inhibit is recognised meanwhile;
//! - progress is passed on, but never past the start of a frame still being read, nor past the
//!   fall of a clock low that may turn out to be an inhibit.
//!
//! Every time limit is turned into ticks of the source: a bound of "at least `t` µs" is
//! ⌈`t` · rate / 10⁶⌉ ticks, one of "at most `t` µs" ⌊`t` · rate / 10⁶⌋ ticks. The input requires
//! a `rateHz` of at least 100 kHz, three samples in the shortest half period of the clock. The
//! decoder reports bytes: what they mean — scan codes, mouse packets, commands — is the next
//! layer's business. Clock frequencies and setup times are not checked.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use uiharu_core::{
    Change, ChannelMask, InputRef, InputSpec, Io, Levels, LogicBlock, MAX_CHANNELS, Node,
    NodeFuture, NodeId, NodeSpec, Outbox, OutputRef, OutputSpec, Props, Provenance, Requires,
    Sample, SignalCursor, Tick, TickRange,
};

use super::events::{Direction, ErrorReason, Ps2Event};

/// The shortest clock low that is the host inhibiting the bus, in microseconds.
pub const INHIBIT_US: u32 = 100;
/// The longest time between two clock edges of a frame, in microseconds.
pub const PAUSE_US: u32 = 2_000;
/// The longest the device may take to begin clocking after a request to send, in
/// microseconds.
pub const FIRST_CLOCK_US: u32 = 15_000;
/// How long both lines must be seen high before the decoder trusts the bus to be idle, in
/// microseconds.
pub const IDLE_US: u32 = 100;
/// The lowest sample rate the decoder takes.
pub const MIN_RATE_HZ: u64 = 100_000;

/// How the bus is wired. A description may leave fields out: they take the defaults.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Ps2Config {
    /// The clock channel.
    pub clk: u8,
    /// The data channel.
    pub data: u8,
}

impl Default for Ps2Config {
    fn default() -> Self {
        Self { clk: 0, data: 1 }
    }
}

impl Ps2Config {
    /// Checks the configuration.
    pub fn validate(&self) -> Result<(), Ps2ConfigError> {
        for channel in [self.clk, self.data] {
            if channel >= MAX_CHANNELS {
                return Err(Ps2ConfigError::Channel(channel));
            }
        }
        if self.clk == self.data {
            return Err(Ps2ConfigError::DuplicateChannel(self.clk));
        }
        Ok(())
    }

    /// What the decoder's output port declares: bytes of eight bits, least significant first.
    pub fn props(&self) -> Props {
        Props::new()
            .with("wordBits", 8)
            .with("bitOrder", "lsb-first")
    }

    /// What the decoder's input requires of the source: both channels, and a sample rate that
    /// sees every half period of the clock.
    pub fn requires(&self) -> Requires {
        Requires::new()
            .at_least("channels", i64::from(self.clk.max(self.data)) + 1)
            .at_least("rateHz", i64::try_from(MIN_RATE_HZ).unwrap_or(i64::MAX))
    }
}

/// Why a configuration is rejected.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Ps2ConfigError {
    /// A channel number is out of range.
    #[error("channel {0} is out of range; channels are 0 to {last}", last = MAX_CHANNELS - 1)]
    Channel(u8),
    /// The clock and the data line are the same channel.
    #[error("channel {0} is used for both the clock and the data line")]
    DuplicateChannel(u8),
}

/// The PS/2 decoder node: logic samples in, `ps2.events` out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ps2 {
    config: Ps2Config,
}

impl Ps2 {
    /// A decoder for `config`.
    pub fn new(config: Ps2Config) -> Result<Self, Ps2ConfigError> {
        config.validate()?;
        Ok(Self { config })
    }

    /// The configuration.
    pub fn config(&self) -> &Ps2Config {
        &self.config
    }

    /// The logic input of the decoder added as `node`.
    pub fn input(node: NodeId) -> InputRef<Arc<LogicBlock>> {
        InputRef::of_node(node, 0)
    }

    /// The event output of the decoder added as `node`.
    pub fn events(node: NodeId) -> OutputRef<Ps2Event> {
        OutputRef::of_node(node, 0)
    }
}

impl Node for Ps2 {
    fn spec(&self) -> NodeSpec {
        NodeSpec {
            name: "ps2".into(),
            inputs: vec![InputSpec::new::<Arc<LogicBlock>>(
                "logic",
                self.config.requires(),
            )],
            outputs: vec![OutputSpec::new::<Ps2Event>("events", self.config.props())],
        }
    }

    fn start(self: Box<Self>, mut io: Io) -> NodeFuture {
        // The input's requirement guarantees a rate; without one every limit is zero.
        let rate_hz = io.time_base().map_or(0, |base| base.rate_hz);
        let Ps2Config { clk, data } = self.config;
        Box::pin(run(Reader {
            signal: io.signal(0),
            out: io.output(0),
            clk,
            data,
            mask: ChannelMask::one(clk).with(data),
            times: Times::new(rate_hz),
            levels: Levels::default(),
            high_since: None,
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

/// The protocol's time limits in ticks of the source.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Times {
    /// The shortest clock low that is an inhibit.
    inhibit: u64,
    /// The longest time between two clock edges of a frame.
    pause: u64,
    /// The longest wait for the device's first clock after a request to send.
    first_clock: u64,
    /// How long both lines must be high for the bus to be trusted idle.
    idle: u64,
}

impl Times {
    fn new(rate_hz: u64) -> Self {
        Self {
            inhibit: at_least(INHIBIT_US, rate_hz).max(1),
            pause: at_most(PAUSE_US, rate_hz),
            first_clock: at_most(FIRST_CLOCK_US, rate_hz),
            idle: at_least(IDLE_US, rate_hz),
        }
    }
}

/// The bits of a frame after its start bit: the data bits, parity and stop, the first as bit 0.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Bits {
    value: u16,
    count: u8,
}

impl Bits {
    /// Data, parity and stop.
    const FRAME: u8 = 10;

    fn push(&mut self, bit: bool) {
        self.value |= u16::from(bit) << self.count;
        self.count += 1;
    }

    fn stop_ok(&self) -> bool {
        self.value >> 9 & 1 != 0
    }

    fn event(&self, direction: Direction, ack: Option<bool>) -> Ps2Event {
        let [value, _] = self.value.to_le_bytes();
        let parity = u32::from(self.value >> 8 & 1);
        Ps2Event::Frame {
            direction,
            value,
            parity_ok: !(value.count_ones() + parity).is_multiple_of(2),
            stop_ok: self.stop_ok(),
            ack,
        }
    }
}

/// Where the decoder stands between frames.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    /// Nothing is known of the bus: waiting for both lines to be high for the idle time.
    Resync,
    /// The bus is idle, the clock high.
    Idle,
    /// The clock fell at `fall` on an idle bus, the data line low there when `start_bit`.
    Fall { fall: Tick, start_bit: bool },
    /// The last clock pulse of a frame fell at `fall`; the bus is to be resynchronised after it
    /// when `resync`.
    Tail { fall: Tick, resync: bool },
    /// The host has held the clock low since `fall` for the inhibit time.
    Inhibit { fall: Tick },
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
    /// These lines changed at this tick.
    Edge(Tick, ChannelMask),
    /// Nothing changed before the deadline.
    Deadline,
}

struct Reader {
    signal: SignalCursor,
    out: Outbox<Ps2Event>,
    clk: u8,
    data: u8,
    mask: ChannelMask,
    times: Times,
    /// Both lines' levels as of the last change.
    levels: Levels,
    /// Since when both lines have been high; `None` while one is low.
    high_since: Option<Tick>,
    /// A gap or the end of the samples found at or past a deadline, for the next wait.
    stopped: Option<Cut>,
}

impl Reader {
    /// Waits for a line to change, before `until` if given, and notes the levels; passes
    /// progress on, held at `hold`. Samples that reach the deadline with no change are the
    /// deadline passing, however the cursor learns it: by `Quiet`, by progress at or past it, or
    /// by a gap or the end of the samples that comes no earlier, which is then kept for the next
    /// wait.
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
                Change::At {
                    tick,
                    changed,
                    levels,
                } => {
                    self.levels = levels;
                    let idle = levels.high(self.clk) && levels.high(self.data);
                    self.high_since = idle.then_some(tick);
                    return Ok(Waited::Edge(tick, changed));
                }
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

    /// Waits for the clock to change, before `until` if given; changes of the data line alone
    /// are noted on the way.
    async fn clock(&mut self, until: Option<Tick>, hold: Option<Tick>) -> Result<Waited, Cut> {
        loop {
            match self.wait(until, hold).await? {
                Waited::Edge(tick, changed) if changed.has(self.clk) => {
                    return Ok(Waited::Edge(tick, changed));
                }
                Waited::Edge(..) => {}
                Waited::Deadline => return Ok(Waited::Deadline),
            }
        }
    }

    /// The data line's level as of the last change.
    fn data_high(&self) -> bool {
        self.levels.high(self.data)
    }

    async fn emit(&mut self, start: Tick, end: Tick, settled: Tick, event: Ps2Event) {
        let at = Provenance::over(TickRange::new(start, end), settled);
        self.out.emit(at, event).await;
    }

    /// Reports the frame that began at `start` aborted, its last clock edge at `last`, decided
    /// at `settled`.
    async fn abort(&mut self, start: Tick, last: Tick, settled: Tick) {
        let error = Ps2Event::Error {
            reason: ErrorReason::Aborted,
        };
        self.emit(start, last.next(), settled, error).await;
    }

    /// Waits for both lines to have been high for the idle time; a host inhibit meanwhile is
    /// taken as such.
    async fn resync(&mut self) -> Result<Phase, Cut> {
        loop {
            let until = self.high_since.map(|since| since.offset(self.times.idle));
            let Waited::Edge(tick, changed) = self.wait(until, None).await? else {
                return Ok(Phase::Idle);
            };
            if changed.has(self.clk) && !self.levels.high(self.clk) {
                let inhibit = tick.offset(self.times.inhibit);
                if self.clock(Some(inhibit), Some(tick)).await? == Waited::Deadline {
                    return Ok(Phase::Inhibit { fall: tick });
                }
            }
        }
    }

    /// Waits for the clock to fall on the idle bus.
    async fn idle(&mut self) -> Result<Phase, Cut> {
        match self.clock(None, None).await? {
            Waited::Edge(tick, _) if !self.levels.high(self.clk) => Ok(Phase::Fall {
                fall: tick,
                start_bit: !self.data_high(),
            }),
            Waited::Edge(..) | Waited::Deadline => Ok(Phase::Idle),
        }
    }

    /// The clock fell at `fall` on an idle bus: a device's first clock pulse, or the host
    /// inhibiting.
    async fn fall(&mut self, fall: Tick, start_bit: bool) -> Result<Phase, Cut> {
        let inhibit = fall.offset(self.times.inhibit);
        let Waited::Edge(rise, _) = self.clock(Some(inhibit), Some(fall)).await? else {
            return Ok(Phase::Inhibit { fall });
        };
        if start_bit {
            return self.device_frame(fall, rise).await;
        }
        let error = Ps2Event::Error {
            reason: ErrorReason::Start,
        };
        self.emit(fall, rise.next(), rise, error).await;
        Ok(Phase::Resync)
    }

    /// The last pulse of a frame fell at `fall`: the device releases the clock, or the host
    /// holds it.
    async fn tail(&mut self, fall: Tick, resync: bool) -> Result<Phase, Cut> {
        let inhibit = fall.offset(self.times.inhibit);
        Ok(match self.clock(Some(inhibit), Some(fall)).await? {
            Waited::Deadline => Phase::Inhibit { fall },
            Waited::Edge(..) if resync => Phase::Resync,
            Waited::Edge(..) => Phase::Idle,
        })
    }

    /// The host has inhibited the bus since `fall`: a request to send when it releases the
    /// clock with the data line low.
    async fn inhibit(&mut self, fall: Tick) -> Result<Phase, Cut> {
        match self.clock(None, Some(fall)).await? {
            Waited::Edge(release, _) if !self.data_high() => self.host_frame(fall, release).await,
            Waited::Edge(..) => Ok(Phase::Idle),
            Waited::Deadline => Ok(Phase::Inhibit { fall }),
        }
    }

    /// Reads the frame the device began with the clock pulse that fell at `start` and rose at
    /// `rise`, its start bit 0: ten more pulses, each bit read on the fall.
    async fn device_frame(&mut self, start: Tick, rise: Tick) -> Result<Phase, Cut> {
        let hold = Some(start);
        let times = self.times;
        let mut last = rise;
        let mut bits = Bits::default();
        loop {
            let pause = last.offset(times.pause);
            let fall = match self.clock(Some(pause.next()), hold).await {
                Ok(Waited::Edge(fall, _)) => fall,
                Ok(Waited::Deadline) => {
                    self.abort(start, last, pause).await;
                    return Ok(Phase::Resync);
                }
                Err(cut) => {
                    self.abort(start, last, cut.tick()).await;
                    return Err(cut);
                }
            };
            bits.push(self.data_high());
            if bits.count == Bits::FRAME {
                let event = bits.event(Direction::DeviceToHost, None);
                self.emit(start, fall.next(), fall, event).await;
                let resync = !bits.stop_ok();
                return Ok(Phase::Tail { fall, resync });
            }
            let inhibit = fall.offset(times.inhibit);
            last = match self.clock(Some(inhibit), hold).await {
                Ok(Waited::Edge(rise, _)) => rise,
                Ok(Waited::Deadline) => {
                    // The host holds the clock: the device must give up the frame.
                    self.abort(start, fall, Tick(inhibit.get() - 1)).await;
                    return Ok(Phase::Inhibit { fall });
                }
                Err(cut) => {
                    self.abort(start, fall, cut.tick()).await;
                    return Err(cut);
                }
            };
        }
    }

    /// Reads the frame the host requested to send by inhibiting the bus from `start` and
    /// releasing the clock at `release` with the data line low: the device's ten pulses, each
    /// bit read on the rise, and the acknowledge on the eleventh.
    async fn host_frame(&mut self, start: Tick, release: Tick) -> Result<Phase, Cut> {
        let hold = Some(start);
        let times = self.times;
        let first = release.offset(times.first_clock);
        let mut fall = match self.clock(Some(first.next()), hold).await {
            Ok(Waited::Edge(fall, _)) => fall,
            Ok(Waited::Deadline) => {
                self.abort(start, release, first).await;
                return Ok(Phase::Resync);
            }
            Err(cut) => {
                self.abort(start, release, cut.tick()).await;
                return Err(cut);
            }
        };
        let mut bits = Bits::default();
        loop {
            let inhibit = fall.offset(times.inhibit);
            let rise = match self.clock(Some(inhibit), hold).await {
                Ok(Waited::Edge(rise, _)) => rise,
                Ok(Waited::Deadline) => {
                    // The host takes the clock back.
                    self.abort(start, fall, Tick(inhibit.get() - 1)).await;
                    return Ok(Phase::Inhibit { fall });
                }
                Err(cut) => {
                    self.abort(start, fall, cut.tick()).await;
                    return Err(cut);
                }
            };
            bits.push(self.data_high());
            let pause = rise.offset(times.pause);
            fall = match self.clock(Some(pause.next()), hold).await {
                Ok(Waited::Edge(fall, _)) => fall,
                Ok(Waited::Deadline) if bits.count == Bits::FRAME => {
                    // Every bit is in but the device clocked no acknowledge.
                    let event = bits.event(Direction::HostToDevice, Some(false));
                    self.emit(start, rise.next(), pause, event).await;
                    return Ok(if bits.stop_ok() {
                        Phase::Idle
                    } else {
                        Phase::Resync
                    });
                }
                Ok(Waited::Deadline) => {
                    self.abort(start, rise, pause).await;
                    return Ok(Phase::Resync);
                }
                Err(cut) => {
                    self.abort(start, rise, cut.tick()).await;
                    return Err(cut);
                }
            };
            if bits.count == Bits::FRAME {
                let ack = !self.data_high();
                let event = bits.event(Direction::HostToDevice, Some(ack));
                self.emit(start, fall.next(), fall, event).await;
                let resync = !bits.stop_ok();
                return Ok(Phase::Tail { fall, resync });
            }
        }
    }
}

async fn run(mut reader: Reader) {
    // Each round of this loop starts where nothing is known of the bus: at the first sample and
    // after every gap.
    'resync: loop {
        loop {
            match reader.signal.sample().await {
                Sample::At { tick, levels } => {
                    reader.levels = levels;
                    let idle = levels.high(reader.clk) && levels.high(reader.data);
                    reader.high_since = idle.then_some(tick);
                    break;
                }
                Sample::Gap(range) => reader.out.gap(range).await,
                Sample::Progress(tick) => reader.out.progress(tick).await,
                Sample::End(tick) => {
                    reader.out.end(tick).await;
                    return;
                }
            }
        }
        let mut phase = Phase::Resync;
        loop {
            let next = match phase {
                Phase::Resync => reader.resync().await,
                Phase::Idle => reader.idle().await,
                Phase::Fall { fall, start_bit } => reader.fall(fall, start_bit).await,
                Phase::Tail { fall, resync } => reader.tail(fall, resync).await,
                Phase::Inhibit { fall } => reader.inhibit(fall).await,
            };
            phase = match next {
                Ok(next) => next,
                Err(Cut::Gap(range)) => {
                    reader.out.gap(range).await;
                    continue 'resync;
                }
                Err(Cut::End(tick)) => {
                    reader.out.end(tick).await;
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
    fn configurations_are_checked() {
        assert_eq!(Ps2Config::default().validate(), Ok(()));
        assert_eq!(
            Ps2Config { clk: 2, data: 2 }.validate(),
            Err(Ps2ConfigError::DuplicateChannel(2))
        );
        assert_eq!(
            Ps2Config { clk: 0, data: 64 }.validate(),
            Err(Ps2ConfigError::Channel(64))
        );
        assert!(Ps2::new(Ps2Config { clk: 64, data: 0 }).is_err());
    }

    #[test]
    fn descriptions_may_leave_fields_out() {
        let config: Ps2Config = serde_json::from_str(r#"{"data": 5}"#).expect("parses");
        assert_eq!(config, Ps2Config { clk: 0, data: 5 });
        assert!(serde_json::from_str::<Ps2Config>(r#"{"clock": 0}"#).is_err());
    }

    #[test]
    fn ports_declare_bytes_and_require_a_fine_enough_rate() {
        let props = Ps2Config::default().props();
        assert_eq!(props.get("wordBits"), Some(&8i64.into()));
        assert_eq!(props.get("bitOrder"), Some(&"lsb-first".into()));
        let spec = Ps2::new(Ps2Config { clk: 4, data: 1 })
            .expect("valid")
            .spec();
        let source = |channels: i64, rate: i64| {
            let props = Props::new().with("channels", channels).with("rateHz", rate);
            spec.inputs[0].requires.unmet(&props).map(|(key, _)| key)
        };
        assert_eq!(source(5, 100_000), None);
        assert_eq!(source(4, 100_000), Some("channels"));
        assert_eq!(source(5, 99_999), Some("rateHz"));
    }

    #[test]
    fn time_limits_round_toward_the_bound() {
        assert_eq!(
            Times::new(1_000_000),
            Times {
                inhibit: 100,
                pause: 2_000,
                first_clock: 15_000,
                idle: 100
            }
        );
        // 333 kHz: a lower bound rounds up, an upper bound down.
        assert_eq!(
            Times::new(333_000),
            Times {
                inhibit: 34,
                pause: 666,
                first_clock: 4_995,
                idle: 34
            }
        );
    }

    #[test]
    fn frames_check_odd_parity_and_the_stop_bit() {
        let frame = |value: u8, parity: bool, stop: bool| {
            let mut bits = Bits::default();
            for bit in 0..8 {
                bits.push(value >> bit & 1 != 0);
            }
            bits.push(parity);
            bits.push(stop);
            bits.event(Direction::DeviceToHost, None)
        };
        // 0x1c has three ones: the parity bit is 0.
        assert_eq!(
            frame(0x1c, false, true),
            Ps2Event::Frame {
                direction: Direction::DeviceToHost,
                value: 0x1c,
                parity_ok: true,
                stop_ok: true,
                ack: None
            }
        );
        assert!(matches!(
            frame(0x1c, true, false),
            Ps2Event::Frame {
                parity_ok: false,
                stop_ok: false,
                ..
            }
        ));
        // 0xff has eight: the parity bit is 1.
        assert!(matches!(
            frame(0xff, true, true),
            Ps2Event::Frame {
                parity_ok: true,
                ..
            }
        ));
    }
}
