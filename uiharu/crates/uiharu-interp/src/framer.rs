// SPDX-License-Identifier: Apache-2.0

//! The framing of a bus decoder's lower stream, as the module declares it: what a cursor over
//! the stream's events needs to tell a frame's beginning, its items and its end apart.

use std::fmt;
use std::sync::Arc;

use serde_json::Value as Json;
use uiharu_core::dsl::{Frame, Framer};

use crate::event::DynEvent;
use crate::json;
use crate::program::{FramingData, Program};
use crate::value::Value;

/// The framing of a contract read at run time, for [`uiharu_core::dsl::FramedBus`]: frames
/// are named by a `u64`, items are the values of the item type the module declares for the
/// contract.
#[derive(Clone)]
pub struct DynFramer {
    program: Arc<Program>,
}

impl fmt::Debug for DynFramer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DynFramer")
            .field("program", &self.program.name())
            .finish()
    }
}

impl DynFramer {
    /// The framing of the lower stream of `program`, which must be a bus decoder.
    pub(crate) fn new(program: Arc<Program>) -> Option<Self> {
        match &program.input {
            crate::program::Input::Bus { .. } => Some(Self { program }),
            _ => None,
        }
    }

    fn framing(&self) -> Option<&FramingData> {
        match &self.program.input {
            crate::program::Input::Bus { framing, .. } => Some(framing),
            _ => None,
        }
    }
}

/// An event that belongs to no frame the cursor could read: skipped outside a frame, and
/// inside one it ends the frame as cut off.
fn foreign() -> Frame<u64, Value> {
    Frame::Item {
        id: u64::MAX,
        item: Value::Unit,
    }
}

impl Framer<DynEvent> for DynFramer {
    type Id = u64;
    type Item = Value;

    fn frame(&self, event: &DynEvent) -> Frame<Self::Id, Self::Item> {
        let Some(framing) = self.framing() else {
            return foreign();
        };
        let Some(object) = event.0.as_object() else {
            return foreign();
        };
        let kind = object.get("kind").and_then(Json::as_str).unwrap_or("");
        let id = object.get(&framing.id).and_then(Json::as_u64).unwrap_or(0);
        if kind == framing.begin {
            let partial = framing
                .partial
                .as_ref()
                .and_then(|field| object.get(field))
                .and_then(Json::as_bool)
                .unwrap_or(false);
            return Frame::Begin { id, partial };
        }
        if kind == framing.end {
            let reason = object
                .get(&framing.reason)
                .and_then(Json::as_str)
                .unwrap_or("");
            return Frame::End {
                id,
                normal: framing.normal.iter().any(|normal| normal == reason),
            };
        }
        if !framing.items.iter().any(|item| item == kind) {
            return foreign();
        }
        match json::item_from_event(&self.program.types, &framing.item, &event.0) {
            Ok(item) => Frame::Item { id, item },
            Err(_) => foreign(),
        }
    }

    fn unframed(&self) -> Option<Self::Id> {
        self.framing().and_then(|framing| framing.unframed)
    }
}
