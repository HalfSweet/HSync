// SPDX-License-Identifier: Apache-2.0

//! The WS281x decoder: the frames of LED data on the data line of a chain of WS2812B-class
//! LEDs, each bit read from the length of its high pulse.
//!
//! Implemented from the WS2812B datasheet (Worldsemi) and the SK6812 and SK6812RGBW datasheets
//! (OPSCO Optoelectronics): one data line at 800 kbit/s; every bit begins with a rising edge and
//! lasts 1.25 µs ± 600 ns, a 0 high for 0.4 µs then low for 0.85 µs, a 1 high for 0.8 µs then low
//! for 0.45 µs, each ± 150 ns; a low of 50 µs or more (80 µs or 280 µs on newer parts) resets the
//! chain, which latches the data it received; each LED keeps the first 24 bits it receives —
//! green, red and blue, most significant bit first — or 32 on RGBW parts, white last, and passes
//! the rest of the frame on.
//!
//! The rules:
//!
//! - a bit begins at a rising edge; its value is the length of the high that follows: a high of
//!   `h` ticks is a 1 when `h × 10⁹ > threshold_ns × rate`, that is when it lasts longer than the
//!   threshold, and a 0 otherwise, `rate` being the source's samples per second. How long the line
//!   then stays low does not matter, as long as it rises again before the reset time;
//! - a low of at least `reset_us`, `⌈reset_us × rate / 10⁶⌉` ticks counted from the falling
//!   edge, is a reset. It ends the frame under way at the computed tick where the low reached
//!   that length, whatever told the decoder — the samples up to that tick, a block used up right
//!   there, or progress — so that the result does not depend on how the samples were cut; a
//!   rising edge at that very tick already comes after the reset;
//! - the first rising edge after a reset begins a frame; bits make LEDs of `bits_per_led` bits in
//!   the order they come, and an LED's bytes are reported in wire order, the first bit received
//!   being the most significant bit of the first byte;
//! - bits left over when the reset comes, fewer than an LED takes, are reported as a
//!   `partial-led` error with their count, just before the frame's end;
//! - the first sample of the stream and the first after a gap are never an edge, and what the
//!   line carries there may be the middle of a frame: the decoder only decodes once it has seen a
//!   reset, a low of the reset time measured from the first sample or from a falling edge after
//!   it, so a frame under way at the start or after a gap is skipped;
//! - a gap or the end of the samples before the reset ends the frame `aborted`, placed at the
//!   gap's start or at the end; the bits of an LED it cuts are dropped. A reset whose tick the
//!   samples reached before the gap or the end ends the frame as usual;
//! - an LED spans its first bit's rising edge to the computed end of its last bit — that bit's
//!   rising edge plus the LED's shortest bit time, the least ticks from one of its bits' rising
//!   edge to the next's, which a pause between bits does not stretch, and never before the tick
//!   after the last bit's falling edge — and is settled at its last bit's falling edge, where
//!   that bit's value became certain. A frame begins on its first rising edge (one tick,
//!   settled there) and ends with an empty span at the reset tick, or at the gap's start or the
//!   end; a `partial-led` error spans the leftover bits as an LED does and is settled at the
//!   reset tick;
//! - progress is passed on, held back to the first rising edge of an LED under way, so that
//!   nothing reported later begins before it;
//! - a threshold shorter than two ticks cannot tell the highs of a 0 and a 1 apart, so the input
//!   requires the source's `rateHz` to be at least `⌈2 × 10⁹ / threshold_ns⌉` and the graph
//!   refuses a slower source.
//!
//! Limits: one data line, the chain's input; no glitch filter, so every high pulse is a bit;
//! highs, lows and bit times are not checked against the datasheets' tolerances, only the
//! threshold and the reset time decide; which byte is which colour is the reader's business; the
//! 400 kbit/s mode of WS2811 drivers needs a threshold of its own.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use uiharu_core::{
    Change, ChannelMask, InputRef, InputSpec, Io, LogicBlock, MAX_CHANNELS, Node, NodeFuture,
    NodeId, NodeSpec, Outbox, OutputRef, OutputSpec, Props, Provenance, Requires, Sample,
    SignalCursor, Tick, TickRange,
};

use super::events::{EndReason, ErrorReason, FrameId, Ws281xEvent};

/// The bits of an RGB LED.
const RGB_BITS: u8 = 24;
/// The bits of an RGBW LED.
const RGBW_BITS: u8 = 32;

/// How the line is wired and what its LEDs take.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Ws281xConfig {
    /// The data line's channel.
    pub data: u8,
    /// The bits each LED takes: 24 for RGB parts, 32 for RGBW parts such as the SK6812RGBW.
    pub bits_per_led: u8,
    /// The high time, in nanoseconds, that tells a 0 from a 1: a longer high is a 1. The default,
    /// 625 ns, lies halfway through the 1.25 µs bit and between the 0.4 µs and 0.8 µs highs.
    pub threshold_ns: u32,
    /// The low time, in microseconds, that resets the chain and ends a frame.
    pub reset_us: u32,
}

impl Default for Ws281xConfig {
    fn default() -> Self {
        Self {
            data: 0,
            bits_per_led: RGB_BITS,
            threshold_ns: 625,
            reset_us: 50,
        }
    }
}

impl Ws281xConfig {
    /// Checks the configuration.
    pub fn validate(&self) -> Result<(), Ws281xConfigError> {
        if self.data >= MAX_CHANNELS {
            return Err(Ws281xConfigError::Channel(self.data));
        }
        if self.bits_per_led != RGB_BITS && self.bits_per_led != RGBW_BITS {
            return Err(Ws281xConfigError::BitsPerLed(self.bits_per_led));
        }
        if self.threshold_ns == 0 {
            return Err(Ws281xConfigError::Threshold);
        }
        if self.reset_us == 0 {
            return Err(Ws281xConfigError::Reset);
        }
        Ok(())
    }

    /// What the decoder's output port declares.
    pub fn props(&self) -> Props {
        Props::new().with("bitsPerLed", i64::from(self.bits_per_led))
    }

    /// The lowest sample rate the decoder accepts: the one at which the threshold spans two
    /// ticks.
    pub fn min_rate_hz(&self) -> u64 {
        2_000_000_000u64.div_ceil(u64::from(self.threshold_ns.max(1)))
    }
}

/// Why a configuration is rejected.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Ws281xConfigError {
    /// The data line's channel number is out of range.
    #[error("channel {0} is out of range; channels are 0 to {last}", last = MAX_CHANNELS - 1)]
    Channel(u8),
    /// The LED width is neither 24 nor 32 bits.
    #[error("{0} bits per LED; 24 (RGB) and 32 (RGBW) are supported")]
    BitsPerLed(u8),
    /// The threshold is zero.
    #[error("the threshold must be above zero")]
    Threshold,
    /// The reset time is zero.
    #[error("the reset time must be above zero")]
    Reset,
}

/// The WS281x decoder node: logic samples in, `ws281x.events` out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ws281x {
    config: Ws281xConfig,
}

impl Ws281x {
    /// A decoder for `config`.
    pub fn new(config: Ws281xConfig) -> Result<Self, Ws281xConfigError> {
        config.validate()?;
        Ok(Self { config })
    }

    /// The configuration.
    pub fn config(&self) -> &Ws281xConfig {
        &self.config
    }

    /// The logic input of the decoder added as `node`.
    pub fn input(node: NodeId) -> InputRef<Arc<LogicBlock>> {
        InputRef::of_node(node, 0)
    }

    /// The event output of the decoder added as `node`.
    pub fn events(node: NodeId) -> OutputRef<Ws281xEvent> {
        OutputRef::of_node(node, 0)
    }
}

impl Node for Ws281x {
    fn spec(&self) -> NodeSpec {
        let min_rate = i64::try_from(self.config.min_rate_hz()).unwrap_or(i64::MAX);
        NodeSpec {
            name: "ws281x".into(),
            inputs: vec![InputSpec::new::<Arc<LogicBlock>>(
                "logic",
                Requires::new()
                    .at_least("channels", i64::from(self.config.data) + 1)
                    .at_least("rateHz", min_rate),
            )],
            outputs: vec![OutputSpec::new::<Ws281xEvent>(
                "events",
                self.config.props(),
            )],
        }
    }

    fn start(self: Box<Self>, mut io: Io) -> NodeFuture {
        let rate = io.time_base().map_or(0, |base| base.rate_hz);
        let timing = Timing::new(&self.config, rate);
        let reader = Reader::new(io.signal(0), io.output(0), &self.config, timing);
        Box::pin(run(reader))
    }
}

/// The threshold and the reset time in ticks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Timing {
    /// The shortest high, in ticks, that is a 1.
    one_from: u64,
    /// The ticks of low that make a reset, at least one.
    reset: u64,
}

impl Timing {
    /// The timing of `config` at `rate` samples per second.
    fn new(config: &Ws281xConfig, rate: u64) -> Self {
        let rate = u128::from(rate);
        let threshold = u128::from(config.threshold_ns) * rate / 1_000_000_000;
        let reset = (u128::from(config.reset_us) * rate).div_ceil(1_000_000);
        Self {
            one_from: u64::try_from(threshold + 1).unwrap_or(u64::MAX),
            reset: u64::try_from(reset).unwrap_or(u64::MAX).max(1),
        }
    }
}

/// The bits of the LED under way.
#[derive(Clone, Copy, Debug)]
struct Bits {
    count: u8,
    /// The bits so far, the first one the most significant.
    value: u32,
    /// The first bit's rising edge.
    first: Tick,
    /// The last bit's rising and falling edges.
    last_rise: Tick,
    last_fall: Tick,
    /// The shortest time from one bit's rising edge to the next's.
    shortest: u64,
}

impl Default for Bits {
    fn default() -> Self {
        Self {
            count: 0,
            value: 0,
            first: Tick::ZERO,
            last_rise: Tick::ZERO,
            last_fall: Tick::ZERO,
            shortest: u64::MAX,
        }
    }
}

impl Bits {
    fn push(&mut self, one: bool, rise: Tick, fall: Tick) {
        if self.count == 0 {
            self.first = rise;
        } else {
            self.shortest = self.shortest.min(self.last_rise.until(rise));
        }
        self.value = (self.value << 1) | u32::from(one);
        self.count += 1;
        self.last_rise = rise;
        self.last_fall = fall;
    }

    /// Where the bits under way began, if there are any.
    fn start(&self) -> Option<Tick> {
        (self.count > 0).then_some(self.first)
    }

    /// From the first bit's rising edge to the computed end of the last bit: its rising edge
    /// plus the shortest bit time, which a pause between bits does not stretch, and never
    /// before the tick after its falling edge.
    fn span(&self) -> TickRange {
        let after_fall = self.last_fall.next();
        let end = if self.count > 1 {
            self.last_rise.offset(self.shortest).max(after_fall)
        } else {
            after_fall
        };
        TickRange::new(self.first, end)
    }

    /// The bits as bytes, the first bit the most significant of the first byte; whole bytes
    /// only, which a whole LED always is.
    fn bytes(&self) -> Vec<u8> {
        let bytes = usize::from(self.count / 8);
        self.value.to_be_bytes()[4 - bytes.min(4)..].to_vec()
    }
}

/// Why the samples stopped.
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

/// The decoder's view of the line and its writer.
#[derive(Debug)]
struct Reader {
    signal: SignalCursor,
    out: Outbox<Ws281xEvent>,
    channel: u8,
    mask: ChannelMask,
    bits_per_led: u8,
    timing: Timing,
    next_frame: u64,
    /// A gap or end the cursor reported along with a deadline's passing, for the next wait.
    stopped: Option<Stop>,
}

impl Reader {
    fn new(
        signal: SignalCursor,
        out: Outbox<Ws281xEvent>,
        config: &Ws281xConfig,
        timing: Timing,
    ) -> Self {
        Self {
            signal,
            out,
            channel: config.data,
            mask: ChannelMask::one(config.data),
            bits_per_led: config.bits_per_led,
            timing,
            next_frame: 1,
            stopped: None,
        }
    }

    /// The tick at the cursor and whether the line is high there.
    async fn level(&mut self) -> Result<(Tick, bool), Stop> {
        if let Some(stop) = self.stopped.take() {
            return Err(stop);
        }
        loop {
            match self.signal.sample().await {
                Sample::At { tick, levels } => return Ok((tick, levels.high(self.channel))),
                Sample::Progress(tick) => self.out.progress(tick).await,
                Sample::Gap(range) => return Err(Stop::Gap(range)),
                Sample::End(tick) => return Err(Stop::End(tick)),
            }
        }
    }

    /// Waits for the line to change to `high` before `until`: the tick it did, or `None` when
    /// the deadline passed first. Progress is passed on, held back to `hold`.
    ///
    /// The samples reaching the deadline without the change are the deadline passing, however
    /// the cursor tells it: by `Quiet`, by progress at or past it, or by a gap or the end of
    /// the samples from there on — which is then kept for the next wait.
    async fn edge(
        &mut self,
        high: bool,
        until: Option<Tick>,
        hold: Option<Tick>,
    ) -> Result<Option<Tick>, Stop> {
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
                Change::At { tick, levels, .. } => {
                    // One line alternates; the other way is never reported.
                    if levels.high(self.channel) == high {
                        return Ok(Some(tick));
                    }
                }
                Change::Quiet(_) => return Ok(None),
                Change::Progress(tick) if reached(tick) => {
                    if let Some(until) = until {
                        self.signal.seek(until);
                    }
                    return Ok(None);
                }
                Change::Progress(tick) => {
                    let tick = hold.map_or(tick, |hold| hold.min(tick));
                    self.out.progress(tick).await;
                }
                Change::Gap(range) if reached(range.start) => {
                    self.stopped = Some(Stop::Gap(range));
                    return Ok(None);
                }
                Change::End(tick) if reached(tick) => {
                    self.stopped = Some(Stop::End(tick));
                    return Ok(None);
                }
                Change::Gap(range) => return Err(Stop::Gap(range)),
                Change::End(tick) => return Err(Stop::End(tick)),
            }
        }
    }

    /// Waits until a reset has been seen: the line low for the reset time, measured from the
    /// first sample (or the first after a gap) or from a falling edge.
    async fn hunt(&mut self) -> Result<(), Stop> {
        let (tick, high) = self.level().await?;
        let mut low_since = (!high).then_some(tick);
        loop {
            match low_since {
                Some(since) => {
                    let reset = since.offset(self.timing.reset);
                    if self.edge(true, Some(reset), None).await?.is_none() {
                        return Ok(());
                    }
                    low_since = None;
                }
                None => low_since = self.edge(false, None, None).await?,
            }
        }
    }

    /// Waits, after a reset, for the rising edge that begins the next frame.
    async fn rise(&mut self) -> Result<Tick, Stop> {
        loop {
            if let Some(tick) = self.edge(true, None, None).await? {
                return Ok(tick);
            }
        }
    }

    /// Reads the frame whose first bit rose at `first_rise`, up to its reset.
    async fn frame(&mut self, first_rise: Tick) -> Result<(), Stop> {
        let frame = FrameId(self.next_frame);
        self.next_frame += 1;
        self.out
            .emit(
                Provenance::one(first_rise),
                Ws281xEvent::FrameBegin { frame },
            )
            .await;
        let mut bits = Bits::default();
        let mut leds = 0u32;
        let mut rise = first_rise;
        loop {
            // The high: its length is the bit's value.
            let hold = Some(bits.start().unwrap_or(rise));
            let fall = loop {
                match self.edge(false, None, hold).await {
                    Ok(Some(tick)) => break tick,
                    Ok(None) => {}
                    Err(stop) => return self.abort(frame, leds, stop).await,
                }
            };
            bits.push(rise.until(fall) >= self.timing.one_from, rise, fall);
            if bits.count == self.bits_per_led {
                let led = Ws281xEvent::Led {
                    frame,
                    index: leds,
                    bytes: bits.bytes(),
                };
                self.out
                    .emit(Provenance::over(bits.span(), fall), led)
                    .await;
                leds = leds.saturating_add(1);
                bits = Bits::default();
            }
            // The low: the next bit, or the reset.
            let reset = fall.offset(self.timing.reset);
            match self.edge(true, Some(reset), bits.start()).await {
                Ok(Some(tick)) => rise = tick,
                Ok(None) => {
                    if bits.count > 0 {
                        let error = Ws281xEvent::Error {
                            frame,
                            reason: ErrorReason::PartialLed,
                            bits: bits.count,
                        };
                        self.out
                            .emit(Provenance::over(bits.span(), reset), error)
                            .await;
                    }
                    let end = Ws281xEvent::FrameEnd {
                        frame,
                        leds,
                        reason: EndReason::Reset,
                    };
                    self.out.emit(Provenance::at(reset), end).await;
                    return Ok(());
                }
                Err(stop) => return self.abort(frame, leds, stop).await,
            }
        }
    }

    /// Ends `frame` where the samples stopped and hands `stop` back.
    async fn abort(&mut self, frame: FrameId, leds: u32, stop: Stop) -> Result<(), Stop> {
        let end = Ws281xEvent::FrameEnd {
            frame,
            leds,
            reason: EndReason::Aborted,
        };
        self.out.emit(Provenance::at(stop.tick()), end).await;
        Err(stop)
    }

    /// Passes a gap or the end of the samples on; returns whether the samples ended.
    async fn pass(&mut self, stop: Stop) -> bool {
        match stop {
            Stop::Gap(range) => {
                self.out.gap(range).await;
                false
            }
            Stop::End(tick) => {
                self.out.end(tick).await;
                true
            }
        }
    }
}

async fn run(mut reader: Reader) {
    // Each round of this loop starts where nothing is known of the line: at the first sample
    // and after every gap.
    'hunt: loop {
        if let Err(stop) = reader.hunt().await {
            if reader.pass(stop).await {
                return;
            }
            continue;
        }
        // A reset has been seen: frames follow, one per rising edge after a reset.
        loop {
            let result = match reader.rise().await {
                Ok(rise) => reader.frame(rise).await,
                Err(stop) => Err(stop),
            };
            if let Err(stop) = result {
                if reader.pass(stop).await {
                    return;
                }
                continue 'hunt;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ws281x::WS281X_EVENTS;

    #[test]
    fn configurations_are_checked() {
        assert_eq!(Ws281xConfig::default().validate(), Ok(()));
        let bad = |config: Ws281xConfig| config.validate().expect_err("rejected");
        assert_eq!(
            bad(Ws281xConfig {
                data: 64,
                ..Ws281xConfig::default()
            }),
            Ws281xConfigError::Channel(64)
        );
        for bits_per_led in [0, 8, 23, 25, 31, 33] {
            assert_eq!(
                bad(Ws281xConfig {
                    bits_per_led,
                    ..Ws281xConfig::default()
                }),
                Ws281xConfigError::BitsPerLed(bits_per_led)
            );
        }
        assert_eq!(
            bad(Ws281xConfig {
                threshold_ns: 0,
                ..Ws281xConfig::default()
            }),
            Ws281xConfigError::Threshold
        );
        assert_eq!(
            bad(Ws281xConfig {
                reset_us: 0,
                ..Ws281xConfig::default()
            }),
            Ws281xConfigError::Reset
        );
        let rgbw = Ws281xConfig {
            bits_per_led: 32,
            ..Ws281xConfig::default()
        };
        assert!(Ws281x::new(rgbw).is_ok());
    }

    #[test]
    fn descriptions_may_leave_fields_out() {
        let config: Ws281xConfig =
            serde_json::from_str(r#"{"data": 3, "bits_per_led": 32}"#).expect("parses");
        assert_eq!(
            config,
            Ws281xConfig {
                data: 3,
                bits_per_led: 32,
                ..Ws281xConfig::default()
            }
        );
        let whole: Ws281xConfig = serde_json::from_str("{}").expect("parses");
        assert_eq!(whole, Ws281xConfig::default());
        assert!(serde_json::from_str::<Ws281xConfig>(r#"{"line": 0}"#).is_err());
    }

    #[test]
    fn props_and_requirements_describe_the_line() {
        let config = Ws281xConfig {
            data: 2,
            bits_per_led: 32,
            ..Ws281xConfig::default()
        };
        assert_eq!(config.props().get("bitsPerLed"), Some(&32i64.into()));
        // Two ticks of 312.5 ns within the 625 ns threshold.
        assert_eq!(config.min_rate_hz(), 3_200_000);
        let spec = Ws281x::new(config).expect("valid").spec();
        assert_eq!(spec.outputs[0].schema, WS281X_EVENTS);
        let source = |channels: i64, rate: i64| {
            let props = Props::new().with("channels", channels).with("rateHz", rate);
            spec.inputs[0].requires.unmet(&props).map(|(key, _)| key)
        };
        assert_eq!(source(3, 3_200_000), None);
        assert_eq!(source(2, 3_200_000), Some("channels"));
        assert_eq!(source(3, 3_199_999), Some("rateHz"));
        // A threshold that does not divide evenly rounds the rate up.
        let odd = Ws281xConfig {
            threshold_ns: 600,
            ..Ws281xConfig::default()
        };
        assert_eq!(odd.min_rate_hz(), 3_333_334);
    }

    #[test]
    fn the_threshold_and_the_reset_are_turned_into_ticks() {
        let config = Ws281xConfig::default();
        // 20 MHz: 625 ns are 12.5 ticks, so 13 ticks of high are the shortest 1; 50 µs are
        // 1000 ticks.
        assert_eq!(
            Timing::new(&config, 20_000_000),
            Timing {
                one_from: 13,
                reset: 1000
            }
        );
        // 16 MHz: 625 ns are 10 ticks exactly, which is not longer than the threshold.
        assert_eq!(Timing::new(&config, 16_000_000).one_from, 11);
        // The reset rounds up: 50 µs at 3.33 MHz are 166.67 ticks.
        assert_eq!(Timing::new(&config, 3_333_333).reset, 167);
        // A source too slow for any of it still gets a reset of one tick.
        assert_eq!(Timing::new(&config, 0).reset, 1);
    }

    #[test]
    fn leds_span_their_bits_to_the_computed_end_of_the_last() {
        let mut bits = Bits::default();
        // Bits every 25 ticks from 100, highs of 8 ticks.
        for i in 0..24u64 {
            bits.push(i % 2 == 0, Tick(100 + 25 * i), Tick(108 + 25 * i));
        }
        assert_eq!(bits.count, 24);
        assert_eq!(bits.bytes(), vec![0xaa, 0xaa, 0xaa]);
        assert_eq!(bits.span(), TickRange::new(Tick(100), Tick(700)));
        // A pause between bits does not stretch the last one.
        let mut paused = Bits::default();
        for rise in [0, 25, 325, 350] {
            paused.push(false, Tick(rise), Tick(rise + 8));
        }
        assert_eq!(paused.span(), TickRange::new(Tick(0), Tick(375)));
        // A last bit high for longer than the bit time still ends after its fall.
        let mut long = Bits::default();
        long.push(true, Tick(0), Tick(10));
        long.push(true, Tick(25), Tick(90));
        assert_eq!(long.span(), TickRange::new(Tick(0), Tick(91)));
        // One bit alone ends after its fall.
        let mut one = Bits::default();
        one.push(false, Tick(40), Tick(48));
        assert_eq!(one.span(), TickRange::new(Tick(40), Tick(49)));
        assert_eq!(one.start(), Some(Tick(40)));
        assert_eq!(Bits::default().start(), None);
    }
}
