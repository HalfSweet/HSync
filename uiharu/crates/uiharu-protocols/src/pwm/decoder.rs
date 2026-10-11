// SPDX-License-Identifier: Apache-2.0

//! The PWM decoder: the cycles of one pulse-width modulated line, each measured in ticks from
//! one leading edge to the next.
//!
//! Pulse-width modulation follows no published specification; the rules are these:
//!
//! - the line is active high or active low as configured: its leading edge is the change from
//!   inactive to active, its trailing edge the change back;
//! - a cycle runs from one leading edge to the next: its period is the ticks between them, its
//!   active time the ticks from the leading edge to the trailing edge in between;
//! - a cycle is reported once the next leading edge is seen; it covers the ticks from its own
//!   leading edge up to that one and is settled there; duty cycle and frequency are the
//!   reader's business, the stream carries ticks;
//! - progress is passed on, held back to the leading edge of the cycle under way, whose period
//!   is not known yet;
//! - the first sample of the stream and the first after a gap are never edges, so the first
//!   cycle begins at the first leading edge seen after them;
//! - a cycle cut short by a gap or by the end of the samples is not reported, and a line that
//!   stays at one level has no cycles.
//!
//! Limits: one line, and no glitch filter: every change of level is an edge, so a glitch
//! makes cycles of its own.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use uiharu_core::{
    Change, ChannelMask, InputRef, InputSpec, Io, LogicBlock, MAX_CHANNELS, Node, NodeFuture,
    NodeId, NodeSpec, Outbox, OutputRef, OutputSpec, Props, Provenance, Requires, SignalCursor,
    Tick, TickRange,
};

use super::events::PwmEvent;

/// Which level of the line is active.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Polarity {
    /// High is active: cycles begin on rising edges.
    #[default]
    ActiveHigh,
    /// Low is active: cycles begin on falling edges.
    ActiveLow,
}

impl Polarity {
    /// The name the port properties use.
    pub const fn name(self) -> &'static str {
        match self {
            Polarity::ActiveHigh => "active-high",
            Polarity::ActiveLow => "active-low",
        }
    }

    /// Whether the line is active when it is high (`high`) or low.
    pub const fn is_active(self, high: bool) -> bool {
        high == matches!(self, Polarity::ActiveHigh)
    }
}

/// Which line carries the signal and which of its levels is active.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PwmConfig {
    /// The line's channel.
    pub channel: u8,
    /// Which level is active.
    pub polarity: Polarity,
}

impl PwmConfig {
    /// Checks the configuration.
    pub fn validate(&self) -> Result<(), PwmConfigError> {
        if self.channel >= MAX_CHANNELS {
            return Err(PwmConfigError::Channel(self.channel));
        }
        Ok(())
    }

    /// What the decoder's output port declares.
    pub fn props(&self) -> Props {
        Props::new().with("polarity", self.polarity.name())
    }
}

/// Why a configuration is rejected.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum PwmConfigError {
    /// The channel number is out of range.
    #[error("channel {0} is out of range; channels are 0 to {last}", last = MAX_CHANNELS - 1)]
    Channel(u8),
}

/// The PWM decoder node: logic samples in, `pwm.events` out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pwm {
    config: PwmConfig,
}

impl Pwm {
    /// A decoder for `config`.
    pub fn new(config: PwmConfig) -> Result<Self, PwmConfigError> {
        config.validate()?;
        Ok(Self { config })
    }

    /// The configuration.
    pub fn config(&self) -> &PwmConfig {
        &self.config
    }

    /// The logic input of the decoder added as `node`.
    pub fn input(node: NodeId) -> InputRef<Arc<LogicBlock>> {
        InputRef::of_node(node, 0)
    }

    /// The event output of the decoder added as `node`.
    pub fn events(node: NodeId) -> OutputRef<PwmEvent> {
        OutputRef::of_node(node, 0)
    }
}

impl Node for Pwm {
    fn spec(&self) -> NodeSpec {
        NodeSpec {
            name: "pwm".into(),
            inputs: vec![InputSpec::new::<Arc<LogicBlock>>(
                "logic",
                Requires::new().at_least("channels", i64::from(self.config.channel) + 1),
            )],
            outputs: vec![OutputSpec::new::<PwmEvent>("events", self.config.props())],
        }
    }

    fn start(self: Box<Self>, mut io: Io) -> NodeFuture {
        Box::pin(run(io.signal(0), io.output(0), self.config))
    }
}

async fn run(mut signal: SignalCursor, mut out: Outbox<PwmEvent>, config: PwmConfig) {
    let watch = ChannelMask::one(config.channel);
    // The leading edge of the cycle under way, and its trailing edge once seen.
    let mut leading: Option<Tick> = None;
    let mut trailing: Option<Tick> = None;
    loop {
        match signal.next_change(watch).await {
            Change::At { tick, levels, .. } => {
                if !config.polarity.is_active(levels.high(config.channel)) {
                    if leading.is_some() {
                        trailing = Some(tick);
                    }
                    continue;
                }
                if let (Some(start), Some(end)) = (leading, trailing) {
                    let cycle = PwmEvent::Cycle {
                        period_ticks: start.until(tick),
                        active_ticks: start.until(end),
                    };
                    let at = Provenance::over(TickRange::new(start, tick), tick);
                    out.emit(at, cycle).await;
                }
                leading = Some(tick);
                trailing = None;
            }
            Change::Gap(range) => {
                leading = None;
                trailing = None;
                out.gap(range).await;
            }
            // Without a deadline the wait is never quiet.
            Change::Quiet(_) => {}
            // The cycle under way will span from its leading edge.
            Change::Progress(tick) => {
                let held = leading.map_or(tick, |start| start.min(tick));
                out.progress(held).await;
            }
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
    use crate::pwm::PWM_EVENTS;

    #[test]
    fn polarity_names_the_active_level() {
        assert!(Polarity::ActiveHigh.is_active(true));
        assert!(!Polarity::ActiveHigh.is_active(false));
        assert!(Polarity::ActiveLow.is_active(false));
        assert!(!Polarity::ActiveLow.is_active(true));
    }

    #[test]
    fn configurations_are_checked() {
        assert_eq!(PwmConfig::default().validate(), Ok(()));
        assert_eq!(
            Pwm::new(PwmConfig {
                channel: 70,
                ..PwmConfig::default()
            }),
            Err(PwmConfigError::Channel(70))
        );
    }

    #[test]
    fn props_and_requirements_describe_the_line() {
        let config = PwmConfig {
            channel: 3,
            polarity: Polarity::ActiveLow,
        };
        assert_eq!(config.props().get("polarity"), Some(&"active-low".into()));
        let spec = Pwm::new(config).expect("valid").spec();
        let requires = &spec.inputs[0].requires;
        assert!(requires.unmet(&Props::new().with("channels", 4)).is_none());
        assert!(requires.unmet(&Props::new().with("channels", 3)).is_some());
        assert_eq!(spec.outputs[0].schema, PWM_EVENTS);
    }
}
