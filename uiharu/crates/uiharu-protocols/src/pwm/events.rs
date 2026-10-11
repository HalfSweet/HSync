// SPDX-License-Identifier: Apache-2.0

//! The `pwm.events` contract: the cycles of a pulse-width modulated line.

use serde::{Deserialize, Serialize};
use uiharu_core::{EventSchema, SchemaId};

/// The schema of PWM event streams.
pub const PWM_EVENTS: SchemaId = SchemaId::new("pwm.events", 1);

/// What travels on a PWM event stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum PwmEvent {
    /// One cycle, from a leading edge, where the line turned active, to the next one.
    Cycle {
        /// Ticks from the leading edge to the next one.
        period_ticks: u64,
        /// Ticks the line was active: from the leading edge to the trailing edge, where it
        /// turned inactive again.
        active_ticks: u64,
    },
}

impl EventSchema for PwmEvent {
    const SCHEMA: SchemaId = PWM_EVENTS;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_as_the_contract_spells_it() {
        let json = serde_json::to_string(&PwmEvent::Cycle {
            period_ticks: 1000,
            active_ticks: 250,
        })
        .expect("serializes");
        assert_eq!(
            json,
            r#"{"kind":"cycle","periodTicks":1000,"activeTicks":250}"#
        );
    }
}
