// SPDX-License-Identifier: Apache-2.0

//! The `timing.events` contract: the edges of one line and the ticks between them.

use serde::{Deserialize, Serialize};
use uiharu_core::{EventSchema, SchemaId};

/// The schema of edge timing streams.
pub const TIMING_EVENTS: SchemaId = SchemaId::new("timing.events", 1);

/// Which way a line changed level.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Slope {
    /// From low to high.
    Rising,
    /// From high to low.
    Falling,
}

/// What travels on an edge timing stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum TimingEvent {
    /// The line changed level.
    Edge {
        /// Which way it changed.
        slope: Slope,
        /// Ticks since the previous edge the stream reported; `None` for the first edge after
        /// the start of the samples or after a gap, whose predecessor was not seen.
        since_previous: Option<u64>,
    },
}

impl EventSchema for TimingEvent {
    const SCHEMA: SchemaId = TIMING_EVENTS;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_as_the_contract_spells_it() {
        let json = serde_json::to_string(&TimingEvent::Edge {
            slope: Slope::Rising,
            since_previous: Some(250),
        })
        .expect("serializes");
        assert_eq!(
            json,
            r#"{"kind":"edge","slope":"rising","sincePrevious":250}"#
        );
        let json = serde_json::to_string(&TimingEvent::Edge {
            slope: Slope::Falling,
            since_previous: None,
        })
        .expect("serializes");
        assert_eq!(
            json,
            r#"{"kind":"edge","slope":"falling","sincePrevious":null}"#
        );
    }
}
