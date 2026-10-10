//! Triggers beyond the simple per-channel one: DSView's multi-stage and serial triggers
//! (DESIGN.md §2.2; `DSView/pv/dock/triggerdock.cpp`).

use hsync_core::{ChannelIndex, TriggerCondition};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Stages a multi-stage trigger has at most.
pub const MAX_TRIGGER_STAGES: usize = 16;
/// Data bits a serial trigger compares at most.
pub const MAX_SERIAL_BITS: u8 = 16;

/// What a device's trigger can do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TriggerCaps {
    /// Largest share of a capture before its trigger, in percent; 1 where the position cannot be
    /// chosen (DSView's Stream mode).
    pub max_position: u8,
    /// Multi-stage and serial triggers are available.
    pub advanced: bool,
}

impl Default for TriggerCaps {
    /// Simple triggers anywhere up to 90 %, DSView's limit.
    fn default() -> Self {
        Self {
            max_position: 90,
            advanced: false,
        }
    }
}

/// Conditions of a trigger pattern by channel; channels left out take no part.
pub type TriggerPattern = Vec<(ChannelIndex, TriggerCondition)>;

/// One stage of a multi-stage trigger: two patterns, combined and matched `count` times before
/// the next stage starts looking.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TriggerStageSpec {
    pub pattern0: TriggerPattern,
    pub pattern1: TriggerPattern,
    /// Both patterns must hold, rather than either.
    pub both: bool,
    /// Negate the first pattern.
    pub invert0: bool,
    /// Negate the second pattern.
    pub invert1: bool,
    /// Times the stage must hold, at least 1.
    pub count: u32,
    /// The times must follow each other without a gap.
    pub contiguous: bool,
}

/// A serial trigger: bits on a data channel, sampled at a clock edge between a start and a stop
/// condition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SerialTriggerSpec {
    pub start: TriggerPattern,
    pub stop: TriggerPattern,
    /// The edge that samples each data bit.
    pub clock: TriggerPattern,
    pub data_channel: ChannelIndex,
    /// Bits compared, 1–16.
    pub bits: u8,
    /// The bits to find.
    pub value: u16,
    /// Bits of `value` that count; the others match anything.
    pub mask: u16,
}

/// A trigger in place of the simple one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "mode", rename_all = "camelCase")]
#[ts(export)]
pub enum AdvancedTrigger {
    /// Stages matched one after the other.
    Stages {
        stages: Vec<TriggerStageSpec>,
    },
    Serial(SerialTriggerSpec),
}

impl AdvancedTrigger {
    /// Why the trigger cannot run on a device with `channels` channels, if it cannot.
    pub fn check(&self, channels: u16) -> Result<(), String> {
        let check_pattern = |pattern: &TriggerPattern| match pattern
            .iter()
            .find(|(channel, _)| *channel >= channels)
        {
            Some((channel, _)) => Err(format!("channel {channel} of {channels}")),
            None => Ok(()),
        };
        match self {
            Self::Stages { stages } => {
                if stages.is_empty() || stages.len() > MAX_TRIGGER_STAGES {
                    return Err(format!("{} trigger stages", stages.len()));
                }
                for stage in stages {
                    if stage.count == 0 {
                        return Err("a stage matched 0 times".into());
                    }
                    check_pattern(&stage.pattern0)?;
                    check_pattern(&stage.pattern1)?;
                }
            }
            Self::Serial(serial) => {
                if !(1..=MAX_SERIAL_BITS).contains(&serial.bits) {
                    return Err(format!("{} serial bits", serial.bits));
                }
                if serial.data_channel >= channels {
                    return Err(format!(
                        "data channel {} of {channels}",
                        serial.data_channel
                    ));
                }
                check_pattern(&serial.start)?;
                check_pattern(&serial.stop)?;
                check_pattern(&serial.clock)?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stage(pattern0: TriggerPattern, count: u32) -> TriggerStageSpec {
        TriggerStageSpec {
            pattern0,
            pattern1: Vec::new(),
            both: true,
            invert0: false,
            invert1: false,
            count,
            contiguous: false,
        }
    }

    #[test]
    fn checks_stages_and_channels() {
        let fine = AdvancedTrigger::Stages {
            stages: vec![stage(vec![(3, TriggerCondition::Rise)], 1)],
        };
        assert_eq!(fine.check(16), Ok(()));
        assert!(fine.check(3).is_err());
        assert!(
            AdvancedTrigger::Stages { stages: Vec::new() }
                .check(16)
                .is_err()
        );
        let never = AdvancedTrigger::Stages {
            stages: vec![stage(Vec::new(), 0)],
        };
        assert!(never.check(16).is_err());
        let serial = AdvancedTrigger::Serial(SerialTriggerSpec {
            start: Vec::new(),
            stop: Vec::new(),
            clock: vec![(0, TriggerCondition::Rise)],
            data_channel: 1,
            bits: 17,
            value: 0,
            mask: 0,
        });
        assert!(serial.check(16).is_err());
    }
}
