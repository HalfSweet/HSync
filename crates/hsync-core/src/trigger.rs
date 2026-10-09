use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// The condition a channel must meet for a simple (per-channel) trigger to fire.
///
/// These are the six choices of the trigger menu (DESIGN.md §9.7). With conditions on several
/// channels, all of them must hold at the same sample.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum TriggerCondition {
    /// The channel takes no part in triggering.
    #[default]
    None,
    /// Low to high transition.
    Rise,
    /// High to low transition.
    Fall,
    /// Any transition.
    Both,
    /// High level.
    High,
    /// Low level.
    Low,
}

impl TriggerCondition {
    /// All conditions in menu order.
    pub const ALL: [Self; 6] = [
        Self::None,
        Self::Rise,
        Self::Fall,
        Self::Both,
        Self::High,
        Self::Low,
    ];

    /// The character DSView and the DSLogic trigger encoder use for this condition
    /// (`X`, `R`, `F`, `C`, `1`, `0`).
    pub const fn to_dsl_char(self) -> u8 {
        match self {
            Self::None => b'X',
            Self::Rise => b'R',
            Self::Fall => b'F',
            Self::Both => b'C',
            Self::High => b'1',
            Self::Low => b'0',
        }
    }

    /// Parses a DSView trigger character.
    pub const fn from_dsl_char(c: u8) -> Option<Self> {
        match c {
            b'X' | b'x' => Some(Self::None),
            b'R' | b'r' => Some(Self::Rise),
            b'F' | b'f' => Some(Self::Fall),
            b'C' | b'c' => Some(Self::Both),
            b'1' => Some(Self::High),
            b'0' => Some(Self::Low),
            _ => None,
        }
    }

    /// Whether the condition watches for a transition rather than a level.
    pub const fn is_edge(self) -> bool {
        matches!(self, Self::Rise | Self::Fall | Self::Both)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dsl_characters_round_trip() {
        for condition in TriggerCondition::ALL {
            assert_eq!(
                TriggerCondition::from_dsl_char(condition.to_dsl_char()),
                Some(condition)
            );
        }
        assert_eq!(TriggerCondition::from_dsl_char(b'?'), None);
    }

    #[test]
    fn serializes_as_camel_case() {
        assert_eq!(
            serde_json::to_string(&TriggerCondition::Rise)
                .ok()
                .as_deref(),
            Some("\"rise\"")
        );
    }
}
