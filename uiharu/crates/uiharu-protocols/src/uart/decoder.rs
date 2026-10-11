// SPDX-License-Identifier: Apache-2.0

//! The UART decoder: the characters on one line of an asynchronous serial link, each timed from
//! its start edge.
//!
//! Implemented from the asynchronous start-stop character format of TIA/EIA-232-F and the
//! receivers of 16550-class UARTs: the line idles at mark; a character is a start bit at space,
//! 5 to 9 data bits, an optional parity bit and 1, 1½ or 2 stop bits at mark; a line held at
//! space through a whole character is a break.
//!
//! The rules:
//!
//! - mark, the idle level, is high, or low on an inverted line; space is the other level. A mark
//!   bit is a 1, a space bit a 0;
//! - a character begins at an edge from mark to space; an edge back to mark begins nothing;
//! - the start bit is read at its sample point: if the line is back at mark there, the edge was a
//!   glitch and the search for a start edge goes on from that point;
//! - every later bit is read at its sample point and nowhere else: a character is timed from its
//!   start edge, and edges inside it are just its bits;
//! - bit `n` of a character, the start bit being bit 0, is read at
//!   `start + ⌊rate × (100 n + p) / (100 × baud)⌋`, where `rate` is the source's samples per
//!   second and `p` the sample point in percent of a bit; the half stop bit of 1½ stop bits is
//!   read at `p` percent of its half bit. Each position is computed from the start edge and
//!   rounded down, so rounding never accumulates: every sample point lies less than a tick
//!   before its exact time;
//! - the data bits make the value least or most significant bit first, as configured; the parity
//!   bit is checked against them (even and odd parity count the 1 bits, mark and space parity
//!   want a fixed bit); every stop bit must read mark, or the character has a framing error;
//! - a character whose every bit reads space, its parity and stop bits included, is a break: the
//!   decoder waits for the line to return to mark and reports how long it was held at space;
//! - after a character the search for the next start edge begins at its last stop bit's sample
//!   point, so a sender somewhat faster than the configured rate is followed;
//! - a character spans from its start edge to the computed end of its stop bits,
//!   `start + ⌊rate × bits / baud⌋` with `bits` its start, data, parity and stop bits (1½ counting
//!   as such), and is settled at its last stop bit's sample point; a break spans from its start
//!   edge to the first tick back at mark, where it is settled;
//! - the first sample of the stream and the first after a gap are never an edge: a line found at
//!   space there is a character under way, which is skipped, and the decoder starts at the next
//!   edge to space once the line is back at mark. Such an edge can lie inside a character whose
//!   start was not seen, so the first characters may be misaligned until the line idles;
//! - a character or break cut off by a gap or by the end of the samples is not reported: the
//!   stream has no frame to abort. After a gap the search for a start edge resumes at the gap's
//!   end, or at the sample point the decoder was seeking when it met the gap, whichever is later;
//! - progress is passed on, but never past the start edge of a character or break still being
//!   read; before a gap or the end goes out, progress is announced up to the gap's start or the
//!   end, or up to the start of the character they cut off. A reader can therefore tell from
//!   progress how long the line has been seen idle, as protocols framed by silences need;
//! - with fewer than two ticks per bit, bits do not get a sample of their own and cannot be told
//!   apart, so the input requires the source's `rateHz` to be at least twice the baud rate and
//!   the graph refuses a slower source. Should a decoder start on one all the same, it reports no
//!   characters and only passes gaps, progress and the end on.
//!
//! Limits: one line per decoder (a link with RX and TX takes two); 5 to 9 data bits; the sample
//! point is a single sample per bit, without majority voting; nothing is reported about the idle
//! line itself.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use uiharu_core::{
    Change, ChannelMask, InputRef, InputSpec, Io, Levels, LogicBlock, MAX_CHANNELS, Node,
    NodeFuture, NodeId, NodeSpec, Outbox, OutputRef, OutputSpec, Props, Provenance, Requires,
    Sample, SignalCursor, Tick, TickRange,
};

use super::events::{ParityCheck, UartEvent};

/// The parity bit a character carries.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Parity {
    /// No parity bit.
    #[default]
    None,
    /// A parity bit that makes the number of 1 bits, data and parity together, even.
    Even,
    /// A parity bit that makes the number of 1 bits odd.
    Odd,
    /// A parity bit that is always 1 (mark).
    Mark,
    /// A parity bit that is always 0 (space).
    Space,
}

impl Parity {
    /// The name the port properties use.
    pub const fn name(self) -> &'static str {
        match self {
            Parity::None => "none",
            Parity::Even => "even",
            Parity::Odd => "odd",
            Parity::Mark => "mark",
            Parity::Space => "space",
        }
    }

    /// The parity bit the data bits `value` call for; `None` without a parity bit.
    pub const fn bit(self, value: u16) -> Option<bool> {
        let odd_ones = value.count_ones() % 2 == 1;
        match self {
            Parity::None => None,
            Parity::Even => Some(odd_ones),
            Parity::Odd => Some(!odd_ones),
            Parity::Mark => Some(true),
            Parity::Space => Some(false),
        }
    }
}

/// How long the stop period of a character is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum StopBits {
    /// One bit time.
    #[default]
    One,
    /// One and a half bit times.
    OneAndAHalf,
    /// Two bit times.
    Two,
}

impl StopBits {
    /// The name the port properties use.
    pub const fn name(self) -> &'static str {
        match self {
            StopBits::One => "one",
            StopBits::OneAndAHalf => "one-and-a-half",
            StopBits::Two => "two",
        }
    }

    /// The stop period in half bits.
    pub const fn half_bits(self) -> u8 {
        match self {
            StopBits::One => 2,
            StopBits::OneAndAHalf => 3,
            StopBits::Two => 4,
        }
    }
}

/// Which data bit is sent first.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BitOrder {
    /// The least significant bit first, as asynchronous serial links send.
    #[default]
    LsbFirst,
    /// The most significant bit first.
    MsbFirst,
}

impl BitOrder {
    /// The name the port properties use.
    pub const fn name(self) -> &'static str {
        match self {
            BitOrder::LsbFirst => "lsb-first",
            BitOrder::MsbFirst => "msb-first",
        }
    }
}

/// How the line is wired and what its characters look like.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct UartConfig {
    /// The channel the line is on.
    pub line: u8,
    /// Bits per second, above zero.
    pub baud: u32,
    /// Data bits per character, 5 to 9.
    pub data_bits: u8,
    /// The parity bit.
    pub parity: Parity,
    /// The stop period.
    pub stop_bits: StopBits,
    /// Which data bit comes first.
    pub bit_order: BitOrder,
    /// Whether the line idles low: mark is low and space high.
    pub inverted: bool,
    /// Where in each bit the line is read, in percent of the bit time, 1 to 99.
    pub sample_point: u8,
}

impl Default for UartConfig {
    fn default() -> Self {
        Self {
            line: 0,
            baud: 115_200,
            data_bits: 8,
            parity: Parity::None,
            stop_bits: StopBits::One,
            bit_order: BitOrder::LsbFirst,
            inverted: false,
            sample_point: 50,
        }
    }
}

impl UartConfig {
    /// Checks the configuration.
    pub fn validate(&self) -> Result<(), UartConfigError> {
        if self.baud == 0 {
            return Err(UartConfigError::Baud);
        }
        if !(5..=9).contains(&self.data_bits) {
            return Err(UartConfigError::DataBits(self.data_bits));
        }
        if !(1..=99).contains(&self.sample_point) {
            return Err(UartConfigError::SamplePoint(self.sample_point));
        }
        if self.line >= MAX_CHANNELS {
            return Err(UartConfigError::Channel(self.line));
        }
        Ok(())
    }

    /// What the decoder's output port declares. The bit time in ticks is not among them: it
    /// depends on the source's rate, which a reader learns from its own time base.
    pub fn props(&self) -> Props {
        Props::new()
            .with("baud", i64::from(self.baud))
            .with("dataBits", i64::from(self.data_bits))
            .with("parity", self.parity.name())
            .with("stopBits", self.stop_bits.name())
            .with("bitOrder", self.bit_order.name())
    }

    /// A character's length, start, data, parity and stop bits, in two-hundredths of a bit.
    fn length(&self) -> u64 {
        let whole = 1 + u64::from(self.data_bits) + u64::from(self.parity != Parity::None);
        200 * whole + 100 * u64::from(self.stop_bits.half_bits())
    }
}

/// Why a configuration is rejected.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum UartConfigError {
    /// The baud rate is zero.
    #[error("the baud rate must be above zero")]
    Baud,
    /// The character width is not 5 to 9 bits.
    #[error("{0} data bits; 5 to 9 are supported")]
    DataBits(u8),
    /// The sample point is not 1 to 99 percent.
    #[error("a sample point at {0}% of the bit; 1% to 99% are supported")]
    SamplePoint(u8),
    /// The line's channel number is out of range.
    #[error("channel {0} is out of range; channels are 0 to {last}", last = MAX_CHANNELS - 1)]
    Channel(u8),
}

/// The UART decoder node: logic samples in, `uart.events` out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Uart {
    config: UartConfig,
}

impl Uart {
    /// A decoder for `config`.
    pub fn new(config: UartConfig) -> Result<Self, UartConfigError> {
        config.validate()?;
        Ok(Self { config })
    }

    /// The configuration.
    pub fn config(&self) -> &UartConfig {
        &self.config
    }

    /// The logic input of the decoder added as `node`.
    pub fn input(node: NodeId) -> InputRef<Arc<LogicBlock>> {
        InputRef::of_node(node, 0)
    }

    /// The event output of the decoder added as `node`.
    pub fn events(node: NodeId) -> OutputRef<UartEvent> {
        OutputRef::of_node(node, 0)
    }
}

impl Node for Uart {
    fn spec(&self) -> NodeSpec {
        NodeSpec {
            name: "uart".into(),
            inputs: vec![InputSpec::new::<Arc<LogicBlock>>(
                "logic",
                Requires::new()
                    .at_least("channels", i64::from(self.config.line) + 1)
                    .at_least("rateHz", 2 * i64::from(self.config.baud)),
            )],
            outputs: vec![OutputSpec::new::<UartEvent>("events", self.config.props())],
        }
    }

    fn start(self: Box<Self>, mut io: Io) -> NodeFuture {
        let rate = io.time_base().map_or(0, |base| base.rate_hz);
        Box::pin(run(io.signal(0), io.output(0), self.config, rate))
    }
}

/// Where the bits of a character fall, in ticks from its start edge.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Timing {
    rate: u64,
    baud: u64,
    sample_point: u64,
}

impl Timing {
    /// The timing at `rate` samples per second; `None` when a bit lasts less than two ticks.
    fn new(config: &UartConfig, rate: u64) -> Option<Self> {
        let baud = u64::from(config.baud);
        (baud > 0 && rate / baud >= 2).then_some(Self {
            rate,
            baud,
            sample_point: u64::from(config.sample_point),
        })
    }

    /// Ticks from the start edge to `position`, given in two-hundredths of a bit, rounded down.
    fn ticks(&self, position: u64) -> u64 {
        let ticks = u128::from(self.rate) * u128::from(position) / (200 * u128::from(self.baud));
        u64::try_from(ticks).unwrap_or(u64::MAX)
    }

    /// The sample point of bit `bit`, the start bit being bit 0.
    fn bit(&self, bit: u64) -> u64 {
        self.ticks(200 * bit + 2 * self.sample_point)
    }

    /// The sample point of a half bit that begins where bit `bit` would.
    fn half_bit(&self, bit: u64) -> u64 {
        self.ticks(200 * bit + self.sample_point)
    }
}

/// Why a character could not be read to its end.
#[derive(Clone, Copy, Debug)]
enum Cut {
    Gap(TickRange),
    End(Tick),
}

struct Reader {
    signal: SignalCursor,
    out: Outbox<UartEvent>,
    config: UartConfig,
    timing: Timing,
    watch: ChannelMask,
}

impl Reader {
    /// Whether `levels` show the line at mark.
    fn mark(&self, levels: Levels) -> bool {
        levels.high(self.config.line) != self.config.inverted
    }

    /// Reads the line `offset` ticks after the start edge at `start`: whether it is at mark.
    /// Progress met while the sample is awaited is passed on held at `start`, where the
    /// character being read comes from.
    async fn read(&mut self, start: Tick, offset: u64) -> Result<bool, Cut> {
        self.signal.seek(start.offset(offset));
        loop {
            match self.signal.sample().await {
                Sample::At { levels, .. } => return Ok(self.mark(levels)),
                Sample::Progress(_) => self.out.progress(start).await,
                Sample::Gap(range) => return Err(Cut::Gap(range)),
                Sample::End(tick) => return Err(Cut::End(tick)),
            }
        }
    }

    /// Reads the character whose start edge is at `start`: its event, or `None` when the start
    /// bit was a glitch.
    async fn character(&mut self, start: Tick) -> Result<Option<(Provenance, UartEvent)>, Cut> {
        let (config, timing) = (self.config, self.timing);
        // With few ticks per bit the start bit's sample point can be the start edge itself,
        // which reads space.
        let start_bit = timing.bit(0);
        if start_bit > 0 && self.read(start, start_bit).await? {
            return Ok(None);
        }
        let data_bits = u64::from(config.data_bits);
        let mut value = 0u16;
        for i in 0..data_bits {
            let one = u16::from(self.read(start, timing.bit(1 + i)).await?);
            value = match config.bit_order {
                BitOrder::LsbFirst => value | (one << i),
                BitOrder::MsbFirst => (value << 1) | one,
            };
        }
        let mut next = 1 + data_bits;
        let mut all_space = value == 0;
        let parity = match config.parity.bit(value) {
            None => None,
            Some(wanted) => {
                let bit = self.read(start, timing.bit(next)).await?;
                next += 1;
                all_space &= !bit;
                Some(if bit == wanted {
                    ParityCheck::Ok
                } else {
                    ParityCheck::Error
                })
            }
        };
        let stop_bits = match config.stop_bits {
            StopBits::One => [Some(timing.bit(next)), None],
            StopBits::OneAndAHalf => [Some(timing.bit(next)), Some(timing.half_bit(next + 1))],
            StopBits::Two => [Some(timing.bit(next)), Some(timing.bit(next + 1))],
        };
        let mut framing_ok = true;
        let mut settled = start;
        for point in stop_bits.into_iter().flatten() {
            let bit = self.read(start, point).await?;
            framing_ok &= bit;
            all_space &= !bit;
            settled = start.offset(point);
        }
        if all_space {
            return self.line_break(start).await.map(Some);
        }
        let end = start.offset(timing.ticks(config.length()));
        let event = UartEvent::Frame {
            value,
            parity,
            framing_ok,
        };
        Ok(Some((
            Provenance::over(TickRange::new(start, end), settled),
            event,
        )))
    }

    /// Waits for the line to return to mark after the break that began at `start`.
    async fn line_break(&mut self, start: Tick) -> Result<(Provenance, UartEvent), Cut> {
        loop {
            match self.signal.next_change(self.watch).await {
                Change::At { tick, levels, .. } if self.mark(levels) => {
                    let at = Provenance::over(TickRange::new(start, tick), tick);
                    let ticks = start.until(tick);
                    return Ok((at, UartEvent::Break { ticks }));
                }
                // One line only changes from space to mark; without a deadline the wait is
                // never quiet.
                Change::At { .. } | Change::Quiet(_) => {}
                Change::Progress(_) => self.out.progress(start).await,
                Change::Gap(range) => return Err(Cut::Gap(range)),
                Change::End(tick) => return Err(Cut::End(tick)),
            }
        }
    }
}

async fn run(signal: SignalCursor, out: Outbox<UartEvent>, config: UartConfig, rate: u64) {
    let Some(timing) = Timing::new(&config, rate) else {
        return without_characters(signal, out).await;
    };
    let watch = ChannelMask::one(config.line);
    let mut reader = Reader {
        signal,
        out,
        config,
        timing,
        watch,
    };
    loop {
        match reader.signal.next_change(watch).await {
            Change::At { tick, levels, .. } => {
                // Only an edge to space begins a character.
                if reader.mark(levels) {
                    continue;
                }
                match reader.character(tick).await {
                    Ok(Some((at, event))) => reader.out.emit(at, event).await,
                    Ok(None) => {}
                    Err(Cut::Gap(range)) => {
                        reader.out.progress(tick).await;
                        reader.out.gap(range).await;
                    }
                    Err(Cut::End(end)) => {
                        reader.out.progress(tick).await;
                        reader.out.end(end).await;
                        return;
                    }
                }
            }
            // Without a deadline the wait is never quiet.
            Change::Quiet(_) => {}
            Change::Progress(tick) => reader.out.progress(tick).await,
            Change::Gap(range) => {
                reader.out.progress(range.start).await;
                reader.out.gap(range).await;
            }
            Change::End(tick) => {
                reader.out.progress(tick).await;
                reader.out.end(tick).await;
                return;
            }
        }
    }
}

/// Lets the samples pass without reading characters, when a bit lasts less than two ticks.
async fn without_characters(mut signal: SignalCursor, mut out: Outbox<UartEvent>) {
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

    #[test]
    fn configurations_are_checked() {
        assert_eq!(UartConfig::default().validate(), Ok(()));
        let bad = |config: UartConfig| config.validate().expect_err("rejected");
        assert_eq!(
            bad(UartConfig {
                baud: 0,
                ..UartConfig::default()
            }),
            UartConfigError::Baud
        );
        for data_bits in [0, 4, 10] {
            assert_eq!(
                bad(UartConfig {
                    data_bits,
                    ..UartConfig::default()
                }),
                UartConfigError::DataBits(data_bits)
            );
        }
        for sample_point in [0, 100] {
            assert_eq!(
                bad(UartConfig {
                    sample_point,
                    ..UartConfig::default()
                }),
                UartConfigError::SamplePoint(sample_point)
            );
        }
        assert_eq!(
            bad(UartConfig {
                line: 64,
                ..UartConfig::default()
            }),
            UartConfigError::Channel(64)
        );
        for data_bits in 5..=9 {
            assert!(
                Uart::new(UartConfig {
                    data_bits,
                    ..UartConfig::default()
                })
                .is_ok()
            );
        }
    }

    #[test]
    fn props_describe_the_line() {
        let props = UartConfig {
            baud: 9600,
            data_bits: 7,
            parity: Parity::Even,
            stop_bits: StopBits::OneAndAHalf,
            bit_order: BitOrder::MsbFirst,
            ..UartConfig::default()
        }
        .props();
        assert_eq!(props.get("baud"), Some(&9600i64.into()));
        assert_eq!(props.get("dataBits"), Some(&7i64.into()));
        assert_eq!(props.get("parity"), Some(&"even".into()));
        assert_eq!(props.get("stopBits"), Some(&"one-and-a-half".into()));
        assert_eq!(props.get("bitOrder"), Some(&"msb-first".into()));
        let uart = Uart::new(UartConfig {
            line: 5,
            ..UartConfig::default()
        })
        .expect("valid");
        let spec = uart.spec();
        let source = |channels: i64, rate: i64| {
            let props = Props::new().with("channels", channels).with("rateHz", rate);
            spec.inputs[0].requires.unmet(&props).map(|(key, _)| key)
        };
        // Six channels for channel 5, and two ticks per bit at 115200 baud.
        assert_eq!(source(6, 230_400), None);
        assert_eq!(source(5, 230_400), Some("channels"));
        assert_eq!(source(6, 230_399), Some("rateHz"));
    }

    #[test]
    fn parity_bits_follow_the_data() {
        assert_eq!(Parity::None.bit(0b1011), None);
        assert_eq!(Parity::Even.bit(0b1011), Some(true));
        assert_eq!(Parity::Even.bit(0b1001), Some(false));
        assert_eq!(Parity::Odd.bit(0b1011), Some(false));
        assert_eq!(Parity::Odd.bit(0), Some(true));
        assert_eq!(Parity::Mark.bit(0), Some(true));
        assert_eq!(Parity::Space.bit(0x1ff), Some(false));
    }

    #[test]
    fn sample_points_are_computed_from_the_start_edge_and_rounded_down() {
        let config = UartConfig::default();
        // 1 MHz at 115200 baud: 8.68 ticks per bit.
        let timing = Timing::new(&config, 1_000_000).expect("enough ticks per bit");
        let points: Vec<u64> = (0..10).map(|bit| timing.bit(bit)).collect();
        assert_eq!(points, vec![4, 13, 21, 30, 39, 47, 56, 65, 73, 82]);
        // Ten bits of 8N1 end 86.8 ticks after the start edge.
        assert_eq!(timing.ticks(config.length()), 86);
        // The half stop bit of 1½ stop bits is read halfway through its half bit.
        let one_and_a_half = UartConfig {
            stop_bits: StopBits::OneAndAHalf,
            ..config
        };
        assert_eq!(one_and_a_half.length(), 2100);
        assert_eq!(timing.half_bit(10), 88);
        // Two ticks per bit is the least that works.
        assert!(Timing::new(&config, 230_400).is_some());
        assert!(Timing::new(&config, 230_399).is_none());
        assert!(Timing::new(&config, 0).is_none());
    }
}
