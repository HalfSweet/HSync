// SPDX-License-Identifier: Apache-2.0

//! The `parallel.events` contract: the words on a parallel bus.

use serde::{Deserialize, Serialize};
use uiharu_core::{EventSchema, SchemaId};

/// The schema of parallel bus event streams.
pub const PARALLEL_EVENTS: SchemaId = SchemaId::new("parallel.events", 1);

/// What travels on a parallel bus event stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum ParallelEvent {
    /// The data lines, read as one word.
    Word {
        /// The word: bit `i` is the level of the `i`-th data line. Written as a hexadecimal
        /// string in JSON (`"0x…"`), as it may be wider than JSON readers keep exactly.
        #[serde(with = "uiharu_core::hex_u64")]
        value: u64,
        /// The data lines, 1 to 64: the width of `value`.
        bits: u8,
        /// The levels found at the first sample, or at the first sample after a gap, of a bus
        /// without a clock: the word was on the bus already and was not seen changing.
        initial: bool,
    },
}

impl EventSchema for ParallelEvent {
    const SCHEMA: SchemaId = PARALLEL_EVENTS;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_as_the_contract_spells_it() {
        let json = serde_json::to_string(&ParallelEvent::Word {
            value: 0xa5,
            bits: 8,
            initial: false,
        })
        .expect("serializes");
        assert_eq!(
            json,
            r#"{"kind":"word","value":"0xa5","bits":8,"initial":false}"#
        );
        let json = serde_json::to_string(&ParallelEvent::Word {
            value: u64::MAX,
            bits: 64,
            initial: true,
        })
        .expect("serializes");
        assert_eq!(
            json,
            r#"{"kind":"word","value":"0xffffffffffffffff","bits":64,"initial":true}"#
        );
    }
}
