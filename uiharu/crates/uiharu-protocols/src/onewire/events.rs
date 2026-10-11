// SPDX-License-Identifier: Apache-2.0

//! The `onewire.link` contract: the time slots the link layer reads from the line.

use serde::{Deserialize, Serialize};
use uiharu_core::{EventSchema, SchemaId};

/// The schema of 1-Wire link-layer streams: resets, bits and bad slots.
pub const ONEWIRE_LINK: SchemaId = SchemaId::new("onewire.link", 1);

/// What travels on a 1-Wire link-layer stream: one event per low pulse the master started.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum LinkEvent {
    /// A reset pulse, and whether a slave answered it.
    Reset {
        /// Whether a presence pulse followed the reset.
        presence: bool,
    },
    /// A time slot. Read and write slots look alike on the line, so this is the bit the line
    /// carried, whichever side drove it.
    Bit {
        /// The bit: `true` for a 1.
        value: bool,
    },
    /// A low pulse that fits no window: longer than a slot and shorter than a reset, a pulse
    /// in a presence window that is no presence pulse, or a slot whose sampling point fell
    /// into the next pulse.
    BadSlot {
        /// How long the line was low, in ticks.
        low_ticks: u64,
    },
}

impl EventSchema for LinkEvent {
    const SCHEMA: SchemaId = ONEWIRE_LINK;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn json<T: Serialize>(value: &T) -> String {
        serde_json::to_string(value).expect("serializes")
    }

    #[test]
    fn link_events_serialize_as_the_contract_spells_them() {
        assert_eq!(
            json(&LinkEvent::Reset { presence: true }),
            r#"{"kind":"reset","presence":true}"#
        );
        assert_eq!(
            json(&LinkEvent::Bit { value: false }),
            r#"{"kind":"bit","value":false}"#
        );
        assert_eq!(
            json(&LinkEvent::BadSlot { low_ticks: 1500 }),
            r#"{"kind":"bad-slot","low_ticks":1500}"#
        );
    }
}
