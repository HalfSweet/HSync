// SPDX-License-Identifier: Apache-2.0

//! The parallel bus decoder: up to 64 data lines read as one word, on the edges of a clock
//! line or whenever a data line changes.
//!
//! A parallel bus follows no published specification; the rules are these:
//!
//! - data lines are listed least significant first: the `i`-th line is bit `i` of the word;
//! - with a clock, a word is the data lines' levels at the configured edge of the clock —
//!   rising, falling or either — read at the first sample with the clock's new level;
//! - without a clock, a word is reported whenever a data line changes, with the levels after
//!   the change; lines that change at the same tick make one word, because the signal cursor
//!   reports them together, while lines that change a tick apart make a word each, so skew
//!   between the lines shows up as words in between;
//! - the first sample of the stream and the first after a gap are never edges or changes.
//!   Without a clock, the levels there are reported once as a word marked `initial`, so that
//!   the bus's value is known from the first sample on; with a clock, no word is read there;
//! - every word is decided at the tick it is read at; other channels, a clock's other edge
//!   and data lines between the clock's edges make no word.
//!
//! Limits: 1 to 64 data lines, read at one tick each: there is no settling time and no
//! glitch filter.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use uiharu_core::{
    Change, ChannelMask, InputRef, InputSpec, Io, Levels, LogicBlock, MAX_CHANNELS, Node,
    NodeFuture, NodeId, NodeSpec, Outbox, OutputRef, OutputSpec, Props, Provenance, Requires,
    Sample, SignalCursor,
};

use super::events::ParallelEvent;

/// The clock edges that read a word.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ClockEdge {
    /// The rising edge.
    #[default]
    Rising,
    /// The falling edge.
    Falling,
    /// Both edges, as on a double data rate bus.
    Either,
}

impl ClockEdge {
    /// Whether a clock that has just changed to the level `high` reads a word.
    pub const fn reads(self, high: bool) -> bool {
        match self {
            ClockEdge::Rising => high,
            ClockEdge::Falling => !high,
            ClockEdge::Either => true,
        }
    }
}

/// The clock line of a bus and the edges on which it reads a word.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Clock {
    /// The clock's channel.
    pub channel: u8,
    /// The edges that read a word.
    pub edge: ClockEdge,
}

/// How the bus is wired.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ParallelConfig {
    /// The data lines' channels, least significant bit first: 1 to 64 of them.
    pub data: Vec<u8>,
    /// The clock, if the bus has one; without one, every change of the data lines is a word.
    pub clock: Option<Clock>,
}

impl Default for ParallelConfig {
    fn default() -> Self {
        Self {
            data: (0..8).collect(),
            clock: None,
        }
    }
}

impl ParallelConfig {
    /// The width of a word: one bit per data line.
    pub fn bits(&self) -> u8 {
        u8::try_from(self.data.len()).unwrap_or(u8::MAX)
    }

    /// The channels the decoder reads.
    pub fn channels(&self) -> impl Iterator<Item = u8> + '_ {
        self.data
            .iter()
            .copied()
            .chain(self.clock.map(|clock| clock.channel))
    }

    /// The word the data lines make at `levels`.
    pub fn word(&self, levels: Levels) -> u64 {
        self.data
            .iter()
            .enumerate()
            .fold(0, |word, (bit, &channel)| {
                word | (u64::from(levels.high(channel)) << bit)
            })
    }

    /// Checks the configuration.
    pub fn validate(&self) -> Result<(), ParallelConfigError> {
        if self.data.is_empty() {
            return Err(ParallelConfigError::NoData);
        }
        if self.data.len() > usize::from(MAX_CHANNELS) {
            return Err(ParallelConfigError::TooManyLines(self.data.len()));
        }
        let mut seen = ChannelMask::EMPTY;
        for channel in self.channels() {
            if channel >= MAX_CHANNELS {
                return Err(ParallelConfigError::Channel(channel));
            }
            if seen.has(channel) {
                return Err(ParallelConfigError::DuplicateChannel(channel));
            }
            seen = seen.with(channel);
        }
        Ok(())
    }

    /// What the decoder's output port declares.
    pub fn props(&self) -> Props {
        Props::new()
            .with("bits", i64::from(self.bits()))
            .with("clocked", self.clock.is_some())
    }
}

/// Why a configuration is rejected.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ParallelConfigError {
    /// No data line is listed.
    #[error("no data line is listed")]
    NoData,
    /// More data lines than a word holds.
    #[error("{0} data lines; at most {MAX_CHANNELS} are supported")]
    TooManyLines(usize),
    /// A channel is used for two lines.
    #[error("channel {0} is used for two lines")]
    DuplicateChannel(u8),
    /// A channel number is out of range.
    #[error("channel {0} is out of range; channels are 0 to {last}", last = MAX_CHANNELS - 1)]
    Channel(u8),
}

/// The parallel bus decoder node: logic samples in, `parallel.events` out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Parallel {
    config: ParallelConfig,
}

impl Parallel {
    /// A decoder for `config`.
    pub fn new(config: ParallelConfig) -> Result<Self, ParallelConfigError> {
        config.validate()?;
        Ok(Self { config })
    }

    /// The configuration.
    pub fn config(&self) -> &ParallelConfig {
        &self.config
    }

    /// The logic input of the decoder added as `node`.
    pub fn input(node: NodeId) -> InputRef<Arc<LogicBlock>> {
        InputRef::of_node(node, 0)
    }

    /// The event output of the decoder added as `node`.
    pub fn events(node: NodeId) -> OutputRef<ParallelEvent> {
        OutputRef::of_node(node, 0)
    }
}

impl Node for Parallel {
    fn spec(&self) -> NodeSpec {
        let channels = self
            .config
            .channels()
            .max()
            .map_or(0, |max| i64::from(max) + 1);
        NodeSpec {
            name: "parallel".into(),
            inputs: vec![InputSpec::new::<Arc<LogicBlock>>(
                "logic",
                Requires::new().at_least("channels", channels),
            )],
            outputs: vec![OutputSpec::new::<ParallelEvent>(
                "events",
                self.config.props(),
            )],
        }
    }

    fn start(self: Box<Self>, mut io: Io) -> NodeFuture {
        Box::pin(run(io.signal(0), io.output(0), self.config))
    }
}

async fn run(mut signal: SignalCursor, mut out: Outbox<ParallelEvent>, config: ParallelConfig) {
    let bits = config.bits();
    let watch = match config.clock {
        Some(clock) => ChannelMask::one(clock.channel),
        None => config
            .data
            .iter()
            .fold(ChannelMask::EMPTY, |mask, &channel| mask.with(channel)),
    };
    // Each round of this loop starts where levels are unknown: at the first sample and after
    // every gap.
    'resync: loop {
        if config.clock.is_none() {
            let (tick, levels) = loop {
                match signal.sample().await {
                    Sample::At { tick, levels } => break (tick, levels),
                    Sample::Gap(range) => out.gap(range).await,
                    Sample::Progress(tick) => out.progress(tick).await,
                    Sample::End(tick) => {
                        out.end(tick).await;
                        return;
                    }
                }
            };
            let word = ParallelEvent::Word {
                value: config.word(levels),
                bits,
                initial: true,
            };
            out.emit(Provenance::one(tick), word).await;
        }
        loop {
            match signal.next_change(watch).await {
                Change::At { tick, levels, .. } => {
                    if let Some(clock) = config.clock
                        && !clock.edge.reads(levels.high(clock.channel))
                    {
                        continue;
                    }
                    let word = ParallelEvent::Word {
                        value: config.word(levels),
                        bits,
                        initial: false,
                    };
                    out.emit(Provenance::one(tick), word).await;
                }
                Change::Gap(range) => {
                    out.gap(range).await;
                    continue 'resync;
                }
                // Without a deadline the wait is never quiet.
                Change::Quiet(_) => {}
                Change::Progress(tick) => out.progress(tick).await,
                Change::End(tick) => {
                    out.end(tick).await;
                    return;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parallel::PARALLEL_EVENTS;

    fn clocked(channel: u8, edge: ClockEdge) -> Option<Clock> {
        Some(Clock { channel, edge })
    }

    #[test]
    fn clock_edges_pick_the_level_after_the_change() {
        assert!(ClockEdge::Rising.reads(true));
        assert!(!ClockEdge::Rising.reads(false));
        assert!(ClockEdge::Falling.reads(false));
        assert!(!ClockEdge::Falling.reads(true));
        assert!(ClockEdge::Either.reads(true));
        assert!(ClockEdge::Either.reads(false));
    }

    #[test]
    fn words_take_the_lines_least_significant_first() {
        let config = ParallelConfig {
            data: vec![5, 0, 63],
            clock: None,
        };
        assert_eq!(config.word(Levels(1 << 5)), 0b001);
        assert_eq!(config.word(Levels(1)), 0b010);
        assert_eq!(config.word(Levels(1 << 63)), 0b100);
        assert_eq!(config.word(Levels(u64::MAX)), 0b111);
        let wide = ParallelConfig {
            data: (0..64).rev().collect(),
            clock: None,
        };
        assert_eq!(wide.word(Levels(1)), 1 << 63);
        assert_eq!(wide.word(Levels(u64::MAX)), u64::MAX);
    }

    #[test]
    fn configurations_are_checked() {
        assert_eq!(ParallelConfig::default().validate(), Ok(()));
        let bad = |data: Vec<u8>, clock: Option<Clock>| {
            ParallelConfig { data, clock }
                .validate()
                .expect_err("rejected")
        };
        assert_eq!(bad(vec![], None), ParallelConfigError::NoData);
        assert_eq!(
            bad(vec![0, 1, 2, 1], None),
            ParallelConfigError::DuplicateChannel(1)
        );
        assert_eq!(
            bad(vec![0, 1, 2], clocked(2, ClockEdge::Rising)),
            ParallelConfigError::DuplicateChannel(2)
        );
        assert_eq!(bad(vec![0, 64], None), ParallelConfigError::Channel(64));
        assert_eq!(
            bad(vec![0], clocked(99, ClockEdge::Either)),
            ParallelConfigError::Channel(99)
        );
        assert_eq!(
            bad((0..=64).collect(), None),
            ParallelConfigError::TooManyLines(65)
        );
        assert_eq!(
            Parallel::new(ParallelConfig {
                data: vec![3, 3],
                clock: None,
            }),
            Err(ParallelConfigError::DuplicateChannel(3))
        );
        let all = ParallelConfig {
            data: (0..64).collect(),
            clock: None,
        };
        assert_eq!(all.validate(), Ok(()));
        assert_eq!(all.bits(), 64);
    }

    #[test]
    fn props_and_requirements_describe_the_bus() {
        let config = ParallelConfig {
            data: vec![2, 9, 4],
            clock: clocked(11, ClockEdge::Falling),
        };
        let props = config.props();
        assert_eq!(props.get("bits"), Some(&3i64.into()));
        assert_eq!(props.get("clocked"), Some(&true.into()));
        let spec = Parallel::new(config).expect("valid").spec();
        let requires = &spec.inputs[0].requires;
        assert!(requires.unmet(&Props::new().with("channels", 12)).is_none());
        assert!(requires.unmet(&Props::new().with("channels", 11)).is_some());
        assert_eq!(spec.outputs[0].schema, PARALLEL_EVENTS);
        let unclocked = ParallelConfig::default().props();
        assert_eq!(unclocked.get("bits"), Some(&8i64.into()));
        assert_eq!(unclocked.get("clocked"), Some(&false.into()));
    }
}
