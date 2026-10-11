// SPDX-License-Identifier: Apache-2.0

//! The AM230x decoder: the readings of an AM2302/DHT22 or DHT11 humidity and temperature sensor
//! on its single data line, each asked for by a start signal from the host.
//!
//! Implemented from the Aosong AM2302 (DHT22) and DHT11 datasheets, which describe the
//! single-bus exchange: the open-drain line idles high; the host pulls it low for at least 1 ms
//! (18 ms for the DHT11) and releases it; 20 to 40 µs later the sensor answers with 80 µs low
//! and 80 µs high, then sends 40 bits, each a 50 µs low followed by a high of 26 to 28 µs for a
//! 0 or 70 µs for a 1, and ends with a 50 µs low before releasing the line. The 40 bits are five
//! bytes, most significant bit first: two of humidity, two of temperature and a checksum, the
//! sum of the four modulo 256.
//!
//! The rules:
//!
//! - every pulse is measured from the edge that began it to the edge that ended it, and must
//!   lie in its window: the datasheet's time widened by the configured `tolerance`, in percent,
//!   the shortest bound rounded up to whole ticks and the longest down
//!   (⌈`t` · (100 − tolerance) · rate / 10⁸⌉ and ⌊`t` · (100 + tolerance) · rate / 10⁸⌋). The
//!   start signal has a shortest bound only, the model's 1 ms or 18 ms less the tolerance;
//! - a low at least as long as a start signal begins a reading, wherever it comes; a shorter
//!   low on an idle line is a pulse without its start signal, `error { reason: start }` at its
//!   release;
//! - after the start signal come, each in its window, the host's release, the sensor's
//!   response low and response high, the 40 bits and the final low; a pulse outside its window
//!   ends the reading — `response` for the release and the response, `bit` for a bit or the
//!   final low —, decided at the edge that ended the pulse when it was too short, and at the
//!   window's last tick when it outlasted it. A bit's high between the windows of a 0 and a 1
//!   is a `bit` error; one that outlasts the window of a 1 means that the sensor stopped
//!   sending, `count`, as fewer than 40 bits came;
//! - a reading spans the start signal's fall through the sensor's final release and is settled
//!   there; its bytes are reported raw, with the checksum checked and the first two and next
//!   two bytes read as the 16-bit humidity and temperature registers. What they mean depends on
//!   the model and is left to the reader: in tenths of a percent and of a degree, bit 15 of the
//!   temperature the sign, on an AM2302; integral and decimal parts in the high and low bytes on
//!   a DHT11;
//! - after an error the pulses are skipped up to the next start signal; an error spans the
//!   exchange from its start signal (or the pulse without one) through the tick it was decided;
//! - a low found at the first sample, or at the first after a gap, began unseen: it is a start
//!   signal only if what was seen of it is already long enough, and the reading then spans from
//!   the first sample seen; otherwise the decoder waits for the next start signal;
//! - a reading cut off by a gap or the end of the samples is not reported: the stream has no
//!   frame to abort. The gap or the end is passed on, and what the cut-off reading — or a low
//!   under way at the gap — leaves on the line after the gap is skipped up to the next start
//!   signal; a line that was idle when the gap began is idle after it;
//! - progress is passed on, but never past the start signal of a reading under way, nor past
//!   the fall of a low that may yet turn out to be a start signal.
//!
//! The windows hold the datasheets' typical values; a sensor that strays further needs a wider
//! tolerance. The tolerance is at most 40 %, which keeps the windows of a 0 and a 1 apart. The
//! input requires a `rateHz` of at least 1 MHz, a sample per microsecond.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use uiharu_core::{
    Change, ChannelMask, InputRef, InputSpec, Io, LogicBlock, MAX_CHANNELS, Node, NodeFuture,
    NodeId, NodeSpec, Outbox, OutputRef, OutputSpec, Props, Provenance, Requires, Sample,
    SignalCursor, Tick, TickRange,
};

use super::events::{Am230xEvent, ErrorReason};

/// The lowest sample rate the decoder takes.
pub const MIN_RATE_HZ: u64 = 1_000_000;
/// The widest tolerance the decoder takes, in percent.
pub const MAX_TOLERANCE: u8 = 40;

/// The bits of a reading.
const BITS: u32 = 40;

/// The datasheets' times, in microseconds: the shortest and the longest.
const RELEASE_US: (u32, u32) = (20, 40);
const RESPONSE_LOW_US: (u32, u32) = (80, 80);
const RESPONSE_HIGH_US: (u32, u32) = (80, 80);
const BIT_LOW_US: (u32, u32) = (50, 50);
const ZERO_HIGH_US: (u32, u32) = (26, 28);
const ONE_HIGH_US: (u32, u32) = (70, 70);
const END_LOW_US: (u32, u32) = (50, 50);

/// Which sensor is on the line.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Model {
    /// The AM2302 (DHT22): a start signal of at least 1 ms; humidity and temperature in tenths,
    /// the temperature's bit 15 its sign.
    #[default]
    Am2302,
    /// The DHT11: a start signal of at least 18 ms; integral and decimal parts in the high and
    /// low bytes.
    Dht11,
}

impl Model {
    /// The name the port properties use.
    pub const fn name(self) -> &'static str {
        match self {
            Model::Am2302 => "am2302",
            Model::Dht11 => "dht11",
        }
    }

    /// The shortest start signal the datasheet asks the host for, in microseconds.
    pub const fn start_us(self) -> u32 {
        match self {
            Model::Am2302 => 1_000,
            Model::Dht11 => 18_000,
        }
    }
}

/// How the line is wired and which sensor is on it. A description may leave fields out: they
/// take the defaults.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Am230xConfig {
    /// The data line's channel.
    pub data: u8,
    /// The sensor.
    pub model: Model,
    /// How far, in percent of the datasheet's times, a pulse may stray; at most
    /// [`MAX_TOLERANCE`].
    pub tolerance: u8,
}

impl Default for Am230xConfig {
    fn default() -> Self {
        Self {
            data: 0,
            model: Model::Am2302,
            tolerance: 30,
        }
    }
}

impl Am230xConfig {
    /// Checks the configuration.
    pub fn validate(&self) -> Result<(), Am230xConfigError> {
        if self.data >= MAX_CHANNELS {
            return Err(Am230xConfigError::Channel(self.data));
        }
        if self.tolerance > MAX_TOLERANCE {
            return Err(Am230xConfigError::Tolerance(self.tolerance));
        }
        Ok(())
    }

    /// What the decoder's output port declares: the sensor model.
    pub fn props(&self) -> Props {
        Props::new().with("model", self.model.name())
    }

    /// What the decoder's input requires of the source: the data channel, and a sample per
    /// microsecond.
    pub fn requires(&self) -> Requires {
        Requires::new()
            .at_least("channels", i64::from(self.data) + 1)
            .at_least("rateHz", i64::try_from(MIN_RATE_HZ).unwrap_or(i64::MAX))
    }
}

/// Why a configuration is rejected.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Am230xConfigError {
    /// The channel number is out of range.
    #[error("channel {0} is out of range; channels are 0 to {last}", last = MAX_CHANNELS - 1)]
    Channel(u8),
    /// The tolerance is too wide.
    #[error("a tolerance of {0}%; at most {MAX_TOLERANCE}% keeps a 0 and a 1 apart")]
    Tolerance(u8),
}

/// The AM230x decoder node: logic samples in, `am230x.events` out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Am230x {
    config: Am230xConfig,
}

impl Am230x {
    /// A decoder for `config`.
    pub fn new(config: Am230xConfig) -> Result<Self, Am230xConfigError> {
        config.validate()?;
        Ok(Self { config })
    }

    /// The configuration.
    pub fn config(&self) -> &Am230xConfig {
        &self.config
    }

    /// The logic input of the decoder added as `node`.
    pub fn input(node: NodeId) -> InputRef<Arc<LogicBlock>> {
        InputRef::of_node(node, 0)
    }

    /// The event output of the decoder added as `node`.
    pub fn events(node: NodeId) -> OutputRef<Am230xEvent> {
        OutputRef::of_node(node, 0)
    }
}

impl Node for Am230x {
    fn spec(&self) -> NodeSpec {
        NodeSpec {
            name: "am230x".into(),
            inputs: vec![InputSpec::new::<Arc<LogicBlock>>(
                "logic",
                self.config.requires(),
            )],
            outputs: vec![OutputSpec::new::<Am230xEvent>(
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
            data: config.data,
            mask: ChannelMask::one(config.data),
            windows: Windows::new(&config, rate_hz),
            high: true,
            stopped: None,
        }))
    }
}

/// The range of whole ticks a pulse may last.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Window {
    min: u64,
    max: u64,
}

impl Window {
    /// The ticks within `us`, widened by `tolerance` percent, at `rate_hz`.
    fn new(us: (u32, u32), tolerance: u8, rate_hz: u64) -> Self {
        let (shortest, longest) = us;
        let tolerance = u128::from(tolerance);
        let rate = u128::from(rate_hz);
        let min = (u128::from(shortest) * (100 - tolerance) * rate).div_ceil(100_000_000);
        let max = u128::from(longest) * (100 + tolerance) * rate / 100_000_000;
        Self {
            min: u64::try_from(min).unwrap_or(u64::MAX),
            max: u64::try_from(max).unwrap_or(u64::MAX),
        }
    }

    fn contains(&self, ticks: u64) -> bool {
        (self.min..=self.max).contains(&ticks)
    }
}

/// The windows of the exchange, in ticks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Windows {
    /// The shortest start signal.
    start: u64,
    release: Window,
    response_low: Window,
    response_high: Window,
    bit_low: Window,
    zero: Window,
    one: Window,
    end_low: Window,
}

impl Windows {
    fn new(config: &Am230xConfig, rate_hz: u64) -> Self {
        let window = |us| Window::new(us, config.tolerance, rate_hz);
        let start_us = config.model.start_us();
        Self {
            start: window((start_us, start_us)).min,
            release: window(RELEASE_US),
            response_low: window(RESPONSE_LOW_US),
            response_high: window(RESPONSE_HIGH_US),
            bit_low: window(BIT_LOW_US),
            zero: window(ZERO_HIGH_US),
            one: window(ONE_HIGH_US),
            end_low: window(END_LOW_US),
        }
    }
}

/// Where the decoder stands between readings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    /// The line is high and idle: a low begins a start signal, or is a pulse without one.
    Idle,
    /// The line is high; pulses are skipped up to the next start signal.
    Skip,
    /// The line has been low since `fall`; `idle` when that began on an idle line.
    Low { fall: Tick, idle: bool },
}

/// Samples ran out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Cut {
    Gap(TickRange),
    End(Tick),
}

/// What a wait found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Waited {
    /// The line changed at this tick.
    Edge(Tick),
    /// It did not change before the deadline.
    Deadline,
}

/// When the edge that ends a pulse came, against its window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Timed {
    /// Inside the window, at this tick.
    InTime(Tick),
    /// Before the window, at this tick.
    Early(Tick),
    /// Not by the window's last tick, this one.
    Late(Tick),
}

struct Reader {
    signal: SignalCursor,
    out: Outbox<Am230xEvent>,
    data: u8,
    mask: ChannelMask,
    windows: Windows,
    /// The line's level as of the last change.
    high: bool,
    /// A gap or the end of the samples found at or past a deadline, for the next wait.
    stopped: Option<Cut>,
}

impl Reader {
    /// Waits for the line to change, before `until` if given, and notes its level; passes
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
                Change::At { tick, levels, .. } => {
                    self.high = levels.high(self.data);
                    return Ok(Waited::Edge(tick));
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

    /// Waits for the edge that ends the pulse that began at `from`, against `window`, for the
    /// reading that began at `start`.
    async fn edge_in(&mut self, from: Tick, window: Window, start: Tick) -> Result<Timed, Cut> {
        let last = from.offset(window.max);
        Ok(match self.wait(Some(last.next()), Some(start)).await? {
            Waited::Edge(tick) if from.until(tick) >= window.min => Timed::InTime(tick),
            Waited::Edge(tick) => Timed::Early(tick),
            Waited::Deadline => Timed::Late(last),
        })
    }

    async fn error(&mut self, start: Tick, decided: Tick, reason: ErrorReason) {
        let at = Provenance::over(TickRange::new(start, decided.next()), decided);
        self.out.emit(at, Am230xEvent::Error { reason }).await;
    }

    /// Ends the reading that began at `start` with `reason`, because the pulse that began at
    /// `from` ended outside its window as `timed` says; the line goes on from there.
    async fn fail(&mut self, start: Tick, from: Tick, timed: Timed, reason: ErrorReason) -> Phase {
        let decided = match timed {
            Timed::InTime(tick) | Timed::Early(tick) | Timed::Late(tick) => tick,
        };
        self.error(start, decided, reason).await;
        match timed {
            // The pulse goes on: when it is a low, it may still be a start signal.
            Timed::Late(_) if !self.high => Phase::Low {
                fall: from,
                idle: false,
            },
            Timed::InTime(tick) | Timed::Early(tick) if !self.high => Phase::Low {
                fall: tick,
                idle: false,
            },
            _ => Phase::Skip,
        }
    }

    /// Waits for the line to fall.
    async fn high_line(&mut self, idle: bool) -> Result<Phase, Cut> {
        Ok(match self.wait(None, None).await? {
            Waited::Edge(fall) if !self.high => Phase::Low { fall, idle },
            Waited::Edge(_) | Waited::Deadline if idle => Phase::Idle,
            Waited::Edge(_) | Waited::Deadline => Phase::Skip,
        })
    }

    /// The line has been low since `fall`: a start signal when long enough.
    async fn low(&mut self, fall: Tick, idle: bool) -> Result<Phase, Cut> {
        let Waited::Edge(release) = self.wait(None, Some(fall)).await? else {
            return Ok(Phase::Low { fall, idle });
        };
        if fall.until(release) >= self.windows.start {
            return self.reading(fall, release).await;
        }
        if idle {
            self.error(fall, release, ErrorReason::Start).await;
        }
        Ok(Phase::Skip)
    }

    /// Reads the exchange whose start signal fell at `start` and was released at `release`.
    async fn reading(&mut self, start: Tick, release: Tick) -> Result<Phase, Cut> {
        let windows = self.windows;
        let response = ErrorReason::Response;
        let answer = match self.edge_in(release, windows.release, start).await? {
            Timed::InTime(tick) => tick,
            timed => return Ok(self.fail(start, release, timed, response).await),
        };
        let rise = match self.edge_in(answer, windows.response_low, start).await? {
            Timed::InTime(tick) => tick,
            timed => return Ok(self.fail(start, answer, timed, response).await),
        };
        let mut fall = match self.edge_in(rise, windows.response_high, start).await? {
            Timed::InTime(tick) => tick,
            timed => return Ok(self.fail(start, rise, timed, response).await),
        };
        // Pausing between the bits keeps those read so far: they are the state of this future.
        let mut value = 0u64;
        for _ in 0..BITS {
            let rise = match self.edge_in(fall, windows.bit_low, start).await? {
                Timed::InTime(tick) => tick,
                timed => return Ok(self.fail(start, fall, timed, ErrorReason::Bit).await),
            };
            let next = match self.edge_in(rise, windows.one, start).await? {
                Timed::Late(last) => {
                    self.error(start, last, ErrorReason::Count).await;
                    return Ok(Phase::Skip);
                }
                Timed::InTime(tick) | Timed::Early(tick) => tick,
            };
            let high = rise.until(next);
            let bit = if windows.zero.contains(high) {
                false
            } else if windows.one.contains(high) {
                true
            } else {
                return Ok(self
                    .fail(start, rise, Timed::Early(next), ErrorReason::Bit)
                    .await);
            };
            value = (value << 1) | u64::from(bit);
            fall = next;
        }
        let end = match self.edge_in(fall, windows.end_low, start).await? {
            Timed::InTime(tick) => tick,
            timed => return Ok(self.fail(start, fall, timed, ErrorReason::Bit).await),
        };
        let [_, _, _, bytes @ ..] = value.to_be_bytes();
        let [
            humidity_high,
            humidity_low,
            temperature_high,
            temperature_low,
            checksum,
        ] = bytes;
        let sum = [
            humidity_high,
            humidity_low,
            temperature_high,
            temperature_low,
        ]
        .iter()
        .fold(0u8, |sum, byte| sum.wrapping_add(*byte));
        let event = Am230xEvent::Reading {
            bytes,
            checksum_ok: sum == checksum,
            humidity_raw: u16::from_be_bytes([humidity_high, humidity_low]),
            temperature_raw: u16::from_be_bytes([temperature_high, temperature_low]),
        };
        let at = Provenance::over(TickRange::new(start, end.next()), end);
        self.out.emit(at, event).await;
        Ok(Phase::Idle)
    }
}

async fn run(mut reader: Reader) {
    // Whether the line was idle where the samples resume: at the first sample nothing says
    // otherwise; after a gap, only when nothing was under way where it began.
    let mut idle = true;
    // Each round of this loop starts where nothing is known of the line: at the first sample
    // and after every gap.
    'resync: loop {
        let mut phase = loop {
            match reader.signal.sample().await {
                Sample::At { tick, levels } => {
                    reader.high = levels.high(reader.data);
                    break match (reader.high, idle) {
                        (true, true) => Phase::Idle,
                        (true, false) => Phase::Skip,
                        (false, _) => Phase::Low {
                            fall: tick,
                            idle: false,
                        },
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
            let next = match phase {
                Phase::Idle => reader.high_line(true).await,
                Phase::Skip => reader.high_line(false).await,
                Phase::Low { fall, idle } => reader.low(fall, idle).await,
            };
            phase = match next {
                Ok(next) => next,
                Err(Cut::Gap(range)) => {
                    // What a reading or a low under way left on the line after the gap is
                    // skipped up to the next start signal.
                    idle = phase == Phase::Idle;
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
        assert_eq!(Am230xConfig::default().validate(), Ok(()));
        let bad = |config: Am230xConfig| config.validate().expect_err("rejected");
        assert_eq!(
            bad(Am230xConfig {
                data: 64,
                ..Am230xConfig::default()
            }),
            Am230xConfigError::Channel(64)
        );
        assert_eq!(
            bad(Am230xConfig {
                tolerance: 41,
                ..Am230xConfig::default()
            }),
            Am230xConfigError::Tolerance(41)
        );
        assert!(
            Am230x::new(Am230xConfig {
                tolerance: 40,
                model: Model::Dht11,
                ..Am230xConfig::default()
            })
            .is_ok()
        );
    }

    #[test]
    fn descriptions_may_leave_fields_out() {
        let config: Am230xConfig =
            serde_json::from_str(r#"{"model": "dht11", "data": 3}"#).expect("parses");
        assert_eq!(
            config,
            Am230xConfig {
                data: 3,
                model: Model::Dht11,
                tolerance: 30
            }
        );
        let whole: Am230xConfig = serde_json::from_str("{}").expect("parses");
        assert_eq!(whole, Am230xConfig::default());
        assert!(serde_json::from_str::<Am230xConfig>(r#"{"model": "dht21"}"#).is_err());
        assert!(serde_json::from_str::<Am230xConfig>(r#"{"line": 0}"#).is_err());
    }

    #[test]
    fn ports_declare_the_model_and_require_a_sample_per_microsecond() {
        assert_eq!(
            Am230xConfig::default().props().get("model"),
            Some(&"am2302".into())
        );
        let spec = Am230x::new(Am230xConfig {
            data: 2,
            ..Am230xConfig::default()
        })
        .expect("valid")
        .spec();
        let source = |channels: i64, rate: i64| {
            let props = Props::new().with("channels", channels).with("rateHz", rate);
            spec.inputs[0].requires.unmet(&props).map(|(key, _)| key)
        };
        assert_eq!(source(3, 1_000_000), None);
        assert_eq!(source(2, 1_000_000), Some("channels"));
        assert_eq!(source(3, 999_999), Some("rateHz"));
    }

    #[test]
    fn windows_widen_by_the_tolerance_and_round_inwards() {
        let windows = Windows::new(&Am230xConfig::default(), 1_000_000);
        assert_eq!(windows.start, 700);
        assert_eq!(windows.release, Window { min: 14, max: 52 });
        assert_eq!(windows.response_low, Window { min: 56, max: 104 });
        assert_eq!(windows.bit_low, Window { min: 35, max: 65 });
        // 26 µs less 30 % is 18.2 µs, 28 µs more 30 % is 36.4 µs.
        assert_eq!(windows.zero, Window { min: 19, max: 36 });
        assert_eq!(windows.one, Window { min: 49, max: 91 });
        let dht11 = Windows::new(
            &Am230xConfig {
                model: Model::Dht11,
                tolerance: 0,
                ..Am230xConfig::default()
            },
            1_000_000,
        );
        assert_eq!(dht11.start, 18_000);
        assert_eq!(dht11.zero, Window { min: 26, max: 28 });
        // At the widest tolerance a 0 and a 1 are still apart.
        let wide = Windows::new(
            &Am230xConfig {
                tolerance: MAX_TOLERANCE,
                ..Am230xConfig::default()
            },
            3_000_000,
        );
        assert!(wide.zero.max < wide.one.min, "{wide:?}");
    }
}
