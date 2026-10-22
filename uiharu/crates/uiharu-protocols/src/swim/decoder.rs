// SPDX-License-Identifier: Apache-2.0

//! ST UM0470 Rev 4 §§3.2–3.6: falling edges start bits; low widths classify them.
//! The receiver thresholds are eight clocks in low speed and four in high speed.
//! All comparisons use integer cross-products, without rounded clock periods.
//! A packet settles at its ACK's rising edge. No idle timeout is invented: UM0470
//! specifies no maximum inter-bit high time. A long low clears command context.
//! Entry recognition accepts nominal half-periods within 25%; it does not claim
//! the target entered active mode. Reset/sync pulses cannot be attributed on one wire.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use uiharu_core::{
    Change, ChannelMask, InputRef, InputSpec, Io, LogicBlock, MAX_CHANNELS, Node, NodeFuture,
    NodeId, NodeSpec, Outbox, OutputRef, OutputSpec, Props, Provenance, Requires, Sample,
    SignalCursor, Tick, TickRange,
};

use super::{Direction, ErrorReason, Field, SwimEvent};

/// The receiver's bit-width threshold. This is the initial speed; reset selects low speed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Speed {
    /// Twenty-two SWIM clocks per nominal bit, threshold eight.
    #[default]
    Low,
    /// Ten SWIM clocks per nominal bit, threshold four.
    High,
}

impl Speed {
    /// The nominal number of SWIM clock periods per bit.
    pub fn clocks(self) -> u32 {
        match self {
            Self::Low => 22,
            Self::High => 10,
        }
    }
}

/// SWIM wiring, target clock and the state at the start of the capture.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SwimConfig {
    /// SWIM data channel, from 0 through 63.
    pub data: u8,
    /// SWIM clock (not HSI): commonly HSI/2, in Hz; 1 through 100 MHz.
    pub clock_hz: u32,
    /// Speed before any observed communication reset.
    pub speed: Speed,
    /// Trust an initial high line as a command boundary; otherwise await a reset.
    pub start_active: bool,
}

impl Default for SwimConfig {
    fn default() -> Self {
        Self {
            data: 0,
            clock_hz: 8_000_000,
            speed: Speed::Low,
            start_active: true,
        }
    }
}

impl SwimConfig {
    /// Validates the channel and clock.
    pub fn validate(&self) -> Result<(), SwimConfigError> {
        if self.data >= MAX_CHANNELS {
            return Err(SwimConfigError::Channel(self.data));
        }
        if self.clock_hz == 0 || self.clock_hz > 100_000_000 {
            return Err(SwimConfigError::Clock(self.clock_hz));
        }
        Ok(())
    }
    /// The output's clock and MSB-first payload convention.
    pub fn props(&self) -> Props {
        Props::new()
            .with("clockHz", i64::from(self.clock_hz))
            .with("bitOrder", "msb-first")
    }
    /// At least two samples in the shortest nominal low pulse (two SWIM clocks).
    pub fn requires(&self) -> Requires {
        Requires::new()
            .at_least("channels", i64::from(self.data) + 1)
            .at_least("rateHz", i64::from(self.clock_hz))
    }
}

/// Invalid SWIM configuration.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum SwimConfigError {
    /// Channel outside the logic mask.
    #[error("decode.config.channel")]
    Channel(u8),
    /// Zero or implausibly large SWIM clock.
    #[error("decode.config.clockHz")]
    Clock(u32),
}

/// Samples-to-SWIM packet decoder.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Swim {
    config: SwimConfig,
}

impl Swim {
    /// Builds a decoder with checked configuration.
    pub fn new(config: SwimConfig) -> Result<Self, SwimConfigError> {
        config.validate()?;
        Ok(Self { config })
    }
    /// The configuration.
    pub fn config(&self) -> &SwimConfig {
        &self.config
    }
    /// Logic input.
    pub fn input(node: NodeId) -> InputRef<Arc<LogicBlock>> {
        InputRef::of_node(node, 0)
    }
    /// SWIM events output.
    pub fn events(node: NodeId) -> OutputRef<SwimEvent> {
        OutputRef::of_node(node, 0)
    }
}

impl Node for Swim {
    fn spec(&self) -> NodeSpec {
        NodeSpec {
            name: "swim".into(),
            inputs: vec![InputSpec::new::<Arc<LogicBlock>>(
                "logic",
                self.config.requires(),
            )],
            outputs: vec![OutputSpec::new::<SwimEvent>("events", self.config.props())],
        }
    }
    fn start(self: Box<Self>, mut io: Io) -> NodeFuture {
        let rate = io.time_base().map_or(0, |base| base.rate_hz);
        Box::pin(run(io.signal(0), io.output(0), self.config, rate))
    }
}

/// Constant-size command context; a NACK never advances it.
#[derive(Clone, Copy, Debug)]
struct Context {
    command: u8,
    step: u8,
    count: u8,
    address: u32,
    offset: u8,
    next_speed: Option<Speed>,
}
impl Context {
    fn new(command: u8) -> Self {
        Self {
            command,
            step: 0,
            count: 0,
            address: 0,
            offset: 0,
            next_speed: None,
        }
    }
    fn field(self) -> Field {
        match self.step {
            0 => Field::Count,
            1 => Field::AddressExtended,
            2 => Field::AddressHigh,
            3 => Field::AddressLow,
            _ => Field::Data,
        }
    }
    fn advance(&mut self, value: u8) -> bool {
        match self.step {
            0 => {
                self.count = value;
                self.step = 1;
                value == 0
            }
            1..=3 => {
                self.address = (self.address << 8) | u32::from(value);
                self.step += 1;
                false
            }
            _ => {
                self.offset = self.offset.saturating_add(1);
                self.offset >= self.count
            }
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct Packet {
    start: Tick,
    direction: Direction,
    bits: u8,
    seen: u8,
    value: u8,
    parity: bool,
}

#[derive(Clone, Copy, Debug)]
struct Entry {
    armed: bool,
    count: u8,
    start: Tick,
    release: Tick,
}
impl Entry {
    fn new() -> Self {
        Self {
            armed: false,
            count: 0,
            start: Tick::ZERO,
            release: Tick::ZERO,
        }
    }
    fn hold(self) -> Option<Tick> {
        (self.count > 0).then_some(self.start)
    }
    fn width(ticks: u64, us: u64, rate: u64) -> bool {
        let scaled = u128::from(ticks) * 4_000_000;
        let nominal = u128::from(us) * u128::from(rate);
        scaled >= nominal * 3 && scaled <= nominal * 5
    }
    /// Eight low pulses with matching intervening high half-periods.
    fn pulse(&mut self, fall: Tick, rise: Tick, rate: u64) -> bool {
        if !self.armed {
            return false;
        }
        let half = if self.count < 4 { 500 } else { 250 };
        // At the frequency transition the preceding high is the last 1 kHz half-period.
        let high_half = if self.count <= 4 { 500 } else { 250 };
        if !Self::width(fall.until(rise), half, rate)
            || (self.count > 0 && !Self::width(self.release.until(fall), high_half, rate))
        {
            self.armed = false;
            self.count = 0;
            return false;
        }
        if self.count == 0 {
            self.start = fall;
        }
        self.count += 1;
        self.release = rise;
        true
    }
}

struct Decoder {
    out: Outbox<SwimEvent>,
    config: SwimConfig,
    rate: u64,
    speed: Speed,
    active: bool,
    context: Option<Context>,
    packet: Option<Packet>,
    entry: Entry,
}

impl Decoder {
    fn hold(&self, low: Option<(Tick, bool)>) -> Option<Tick> {
        [
            low.map(|(tick, _)| tick),
            self.packet.map(|p| p.start),
            self.entry.hold(),
        ]
        .into_iter()
        .flatten()
        .min()
    }
    async fn error(&mut self, start: Tick, end: Tick, reason: ErrorReason, partial: bool) {
        self.out
            .emit(
                Provenance::over(TickRange::new(start, end.max(start.next())), end),
                SwimEvent::Error { reason, partial },
            )
            .await;
    }
    async fn pulse(&mut self, fall: Tick, rise: Tick, partial: bool) {
        let width = fall.until(rise);
        if self.entry.pulse(fall, rise, self.rate) {
            if self.entry.count == 8 {
                self.out
                    .emit(
                        Provenance::over(TickRange::new(self.entry.start, rise.next()), rise),
                        SwimEvent::Entry {},
                    )
                    .await;
                self.entry = Entry::new();
                self.active = true;
                self.context = None;
                self.speed = Speed::Low;
            }
            return;
        }
        let clocks = u128::from(width) * u128::from(self.config.clock_hz);
        if clocks > u128::from(self.rate) * 64 {
            if let Some(packet) = self.packet.take() {
                self.error(packet.start, rise, ErrorReason::Reset, false)
                    .await;
            }
            self.context = None;
            self.speed = Speed::Low;
            self.active = true;
            self.out
                .emit(
                    Provenance::over(TickRange::new(fall, rise.next()), rise),
                    SwimEvent::Reset {},
                )
                .await;
            // A cut-off low does not prove the entry prelude was at least 16 µs.
            self.entry.armed =
                !partial && u128::from(width) * 1_000_000 >= u128::from(self.rate) * 16;
            return;
        }
        if partial {
            self.error(fall, rise, ErrorReason::Aborted, true).await;
            self.active = false;
            self.context = None;
            return;
        }
        if !self.active {
            return;
        }
        let threshold = if self.speed == Speed::Low { 8 } else { 4 };
        let bit = clocks <= u128::from(self.rate) * threshold;
        let Some(mut packet) = self.packet.take() else {
            let direction = if bit {
                Direction::TargetToHost
            } else {
                Direction::HostToTarget
            };
            // Host data only follows a known ROTF/WOTF parameter or WOTF payload.
            let byte = bit
                || self
                    .context
                    .is_some_and(|ctx| ctx.step < 4 || ctx.command == 2);
            self.packet = Some(Packet {
                start: fall,
                direction,
                bits: if byte { 8 } else { 3 },
                seen: 0,
                value: 0,
                parity: false,
            });
            return;
        };
        if packet.seen < packet.bits {
            packet.value = (packet.value << 1) | u8::from(bit);
        } else if packet.seen == packet.bits {
            packet.parity = bit;
        } else {
            self.complete(packet, bit, rise).await;
            return;
        }
        packet.seen += 1;
        self.packet = Some(packet);
    }
    async fn complete(&mut self, packet: Packet, ack: bool, rise: Tick) {
        let parity_ok = (!packet.value.count_ones().is_multiple_of(2)) == packet.parity;
        let accepted = parity_ok && ack;
        let event = if packet.bits == 3 {
            if accepted {
                self.context = match packet.value {
                    1 | 2 => Some(Context::new(packet.value)),
                    _ => None,
                };
            }
            SwimEvent::Command {
                value: packet.value,
                parity_ok,
                ack,
            }
        } else {
            let context = self.context.filter(|ctx| {
                packet.direction == Direction::HostToTarget && (ctx.step < 4 || ctx.command == 2)
                    || packet.direction == Direction::TargetToHost
                        && ctx.step >= 4
                        && ctx.command == 1
            });
            let field = context.map_or(Field::Unclassified, Context::field);
            let address = context
                .filter(|ctx| ctx.step >= 4)
                .map(|ctx| ctx.address.wrapping_add(u32::from(ctx.offset)) & 0xffffff);
            if accepted && let Some(mut ctx) = context {
                let done = ctx.advance(packet.value);
                // The HS bit takes effect after this write, when the communication becomes idle.
                if field == Field::Data && address == Some(0x7f80) && ctx.command == 2 {
                    ctx.next_speed = Some(if packet.value & 0x10 != 0 {
                        Speed::High
                    } else {
                        Speed::Low
                    });
                }
                if done && let Some(speed) = ctx.next_speed {
                    self.speed = speed;
                }
                self.context = (!done).then_some(ctx);
            }
            SwimEvent::Byte {
                direction: packet.direction,
                field,
                value: packet.value,
                parity_ok,
                ack,
                address,
            }
        };
        self.out
            .emit(
                Provenance::over(TickRange::new(packet.start, rise.next()), rise),
                event,
            )
            .await;
    }
    async fn cut(&mut self, low: Option<(Tick, bool)>, tick: Tick) {
        if let Some(packet) = self.packet.take() {
            self.error(packet.start, tick, ErrorReason::Aborted, false)
                .await;
        } else if let Some((fall, partial)) = low {
            self.error(fall, tick, ErrorReason::Aborted, partial).await;
        } else if self.entry.count > 0 {
            self.error(self.entry.start, tick, ErrorReason::Aborted, false)
                .await;
        }
        self.active = false;
        self.context = None;
        self.entry = Entry::new();
    }
}

async fn run(mut signal: SignalCursor, out: Outbox<SwimEvent>, config: SwimConfig, rate: u64) {
    let mask = ChannelMask::one(config.data);
    let mut decoder = Decoder {
        out,
        config,
        rate,
        speed: config.speed,
        active: config.start_active,
        context: None,
        packet: None,
        entry: Entry::new(),
    };
    'sample: loop {
        let mut low = loop {
            match signal.sample().await {
                Sample::At { tick, levels } => {
                    break (!levels.high(config.data)).then_some((tick, true));
                }
                Sample::Progress(tick) => decoder.out.progress(tick).await,
                Sample::Gap(range) => decoder.out.gap(range).await,
                Sample::End(tick) => {
                    decoder.out.end(tick).await;
                    return;
                }
            }
        };
        if low.is_some() {
            decoder.active = false;
        }
        loop {
            match signal.next_change(mask).await {
                Change::At { tick, levels, .. } => {
                    if levels.high(config.data) {
                        if let Some((fall, partial)) = low.take() {
                            decoder.pulse(fall, tick, partial).await;
                        }
                    } else {
                        low = Some((tick, false));
                    }
                }
                Change::Progress(tick) => {
                    decoder
                        .out
                        .progress(decoder.hold(low).map_or(tick, |hold| hold.min(tick)))
                        .await
                }
                Change::Gap(range) => {
                    decoder.cut(low, range.start).await;
                    decoder.out.gap(range).await;
                    continue 'sample;
                }
                Change::End(tick) => {
                    decoder.cut(low, tick).await;
                    decoder.out.end(tick).await;
                    return;
                }
                Change::Quiet(_) => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn configurations_and_properties_are_checked() {
        assert!(Swim::new(SwimConfig::default()).is_ok());
        assert!(
            Swim::new(SwimConfig {
                data: 64,
                ..SwimConfig::default()
            })
            .is_err()
        );
        for clock_hz in [0, 100_000_001] {
            assert!(
                Swim::new(SwimConfig {
                    clock_hz,
                    ..SwimConfig::default()
                })
                .is_err()
            );
        }
        assert_eq!(
            SwimConfig::default().props().get("bitOrder"),
            Some(&"msb-first".into())
        );
        let config: SwimConfig = serde_json::from_str("{}").expect("defaults");
        assert_eq!(config, SwimConfig::default());
        assert!(serde_json::from_str::<SwimConfig>(r#"{"unknown":true}"#).is_err());
    }
}
