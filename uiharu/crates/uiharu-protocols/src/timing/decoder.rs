// SPDX-License-Identifier: Apache-2.0

//! The edge timing decoder: the edges of one line, all of them or only the rising or the
//! falling ones, each with the ticks since the previous one reported.
//!
//! Edge timing follows no published specification; the rules are these:
//!
//! - an edge is a change of the line's level from one sample to the next, placed at the first
//!   sample with the new level;
//! - `edges` picks the edges reported: all of them, the rising ones or the falling ones; the
//!   interval is counted from the previous *reported* edge, so with one kind of edge it is the
//!   period of the signal and with both it is the time the line spent at a level;
//! - the first sample of the stream and the first after a gap are never edges, and the first
//!   edge reported after either has no interval (`since_previous: None`): the edge before it
//!   was not seen;
//! - an edge is decided at its tick; frequencies, periods and duty cycles are the reader's
//!   business, the stream carries ticks.
//!
//! Limits: one line, and no glitch filter: a glitch is two edges.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use uiharu_core::{
    Change, ChannelMask, InputRef, InputSpec, Io, LogicBlock, MAX_CHANNELS, Node, NodeFuture,
    NodeId, NodeSpec, Outbox, OutputRef, OutputSpec, Props, Provenance, Requires, SignalCursor,
    Tick,
};

use super::events::{Slope, TimingEvent};

/// Which edges the decoder reports.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Edges {
    /// Rising and falling edges.
    #[default]
    Any,
    /// Rising edges only.
    Rising,
    /// Falling edges only.
    Falling,
}

impl Edges {
    /// The name the port properties use.
    pub const fn name(self) -> &'static str {
        match self {
            Edges::Any => "any",
            Edges::Rising => "rising",
            Edges::Falling => "falling",
        }
    }

    /// Whether an edge of `slope` is reported.
    pub const fn accepts(self, slope: Slope) -> bool {
        matches!(
            (self, slope),
            (Edges::Any, _) | (Edges::Rising, Slope::Rising) | (Edges::Falling, Slope::Falling)
        )
    }
}

/// Which line is timed and which of its edges are reported.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimingConfig {
    /// The line's channel.
    pub channel: u8,
    /// The edges reported.
    pub edges: Edges,
}

impl TimingConfig {
    /// Checks the configuration.
    pub fn validate(&self) -> Result<(), TimingConfigError> {
        if self.channel >= MAX_CHANNELS {
            return Err(TimingConfigError::Channel(self.channel));
        }
        Ok(())
    }

    /// What the decoder's output port declares.
    pub fn props(&self) -> Props {
        Props::new().with("edges", self.edges.name())
    }
}

/// Why a configuration is rejected.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum TimingConfigError {
    /// The channel number is out of range.
    #[error("channel {0} is out of range; channels are 0 to {last}", last = MAX_CHANNELS - 1)]
    Channel(u8),
}

/// The edge timing node: logic samples in, `timing.events` out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Timing {
    config: TimingConfig,
}

impl Timing {
    /// A decoder for `config`.
    pub fn new(config: TimingConfig) -> Result<Self, TimingConfigError> {
        config.validate()?;
        Ok(Self { config })
    }

    /// The configuration.
    pub fn config(&self) -> &TimingConfig {
        &self.config
    }

    /// The logic input of the decoder added as `node`.
    pub fn input(node: NodeId) -> InputRef<Arc<LogicBlock>> {
        InputRef::of_node(node, 0)
    }

    /// The event output of the decoder added as `node`.
    pub fn events(node: NodeId) -> OutputRef<TimingEvent> {
        OutputRef::of_node(node, 0)
    }
}

impl Node for Timing {
    fn spec(&self) -> NodeSpec {
        NodeSpec {
            name: "timing".into(),
            inputs: vec![InputSpec::new::<Arc<LogicBlock>>(
                "logic",
                Requires::new().at_least("channels", i64::from(self.config.channel) + 1),
            )],
            outputs: vec![OutputSpec::new::<TimingEvent>(
                "events",
                self.config.props(),
            )],
        }
    }

    fn start(self: Box<Self>, mut io: Io) -> NodeFuture {
        Box::pin(run(io.signal(0), io.output(0), self.config))
    }
}

async fn run(mut signal: SignalCursor, mut out: Outbox<TimingEvent>, config: TimingConfig) {
    let watch = ChannelMask::one(config.channel);
    // The last edge reported since the samples began or resumed after a gap.
    let mut previous: Option<Tick> = None;
    loop {
        match signal.next_change(watch).await {
            Change::At { tick, levels, .. } => {
                let slope = if levels.high(config.channel) {
                    Slope::Rising
                } else {
                    Slope::Falling
                };
                if !config.edges.accepts(slope) {
                    continue;
                }
                let since_previous = previous.map(|previous| previous.until(tick));
                previous = Some(tick);
                let edge = TimingEvent::Edge {
                    slope,
                    since_previous,
                };
                out.emit(Provenance::one(tick), edge).await;
            }
            Change::Gap(range) => {
                previous = None;
                out.gap(range).await;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::timing::TIMING_EVENTS;

    #[test]
    fn edges_pick_the_slopes() {
        assert!(Edges::Any.accepts(Slope::Rising));
        assert!(Edges::Any.accepts(Slope::Falling));
        assert!(Edges::Rising.accepts(Slope::Rising));
        assert!(!Edges::Rising.accepts(Slope::Falling));
        assert!(Edges::Falling.accepts(Slope::Falling));
        assert!(!Edges::Falling.accepts(Slope::Rising));
    }

    #[test]
    fn configurations_are_checked() {
        assert_eq!(TimingConfig::default().validate(), Ok(()));
        assert_eq!(
            Timing::new(TimingConfig {
                channel: 64,
                ..TimingConfig::default()
            }),
            Err(TimingConfigError::Channel(64))
        );
    }

    #[test]
    fn props_and_requirements_describe_the_line() {
        let config = TimingConfig {
            channel: 9,
            edges: Edges::Falling,
        };
        assert_eq!(config.props().get("edges"), Some(&"falling".into()));
        let spec = Timing::new(config).expect("valid").spec();
        let requires = &spec.inputs[0].requires;
        assert!(requires.unmet(&Props::new().with("channels", 10)).is_none());
        assert!(requires.unmet(&Props::new().with("channels", 9)).is_some());
        assert_eq!(spec.outputs[0].schema, TIMING_EVENTS);
    }
}
