// SPDX-License-Identifier: Apache-2.0

//! Reading an exposed output without knowing its event type: every event becomes a JSON
//! value, the way the contracts serialize it (`kind` and the fields by name).

use serde::Serialize;
use uiharu_core::{EventSchema, Exposed, Item, SchemaId, Tick, TickRange};

/// An item of a stream with its event, if any, as a JSON value.
#[derive(Clone, Debug, PartialEq)]
pub enum DynItem {
    /// An event: its number in the stream, its provenance and its payload as the contract
    /// serializes it.
    Event {
        seq: u64,
        span: TickRange,
        settled: Tick,
        payload: serde_json::Value,
    },
    /// Samples, and whatever happened in them, are missing for these ticks.
    Gap(TickRange),
    /// Nothing more will be reported before this tick.
    Progress(Tick),
    /// The stream has ended at this tick.
    End(Tick),
}

/// A reader on an exposed output, with the event type erased.
pub trait DynReader {
    /// The name the output was exposed under.
    fn name(&self) -> &str;
    /// The schema of the events.
    fn schema(&self) -> SchemaId;
    /// The next item, if one is waiting. The end is returned once; afterwards nothing.
    fn next(&mut self) -> Option<DynItem>;
    /// Whether the end of the stream was read.
    fn is_ended(&self) -> bool;
    /// The items waiting to be read.
    fn pending(&self) -> usize;
    /// Events read so far.
    fn events(&self) -> u64;
}

/// The reader of an output whose event type is known: [`DynReader`] over [`Exposed`].
#[derive(Debug)]
pub struct TypedReader<T> {
    name: String,
    exposed: Exposed<T>,
    ended: bool,
    events: u64,
}

impl<T> TypedReader<T> {
    /// A reader named `name` over `exposed`.
    pub fn new(name: impl Into<String>, exposed: Exposed<T>) -> Self {
        Self {
            name: name.into(),
            exposed,
            ended: false,
            events: 0,
        }
    }
}

impl<T: Serialize + EventSchema + Clone> DynReader for TypedReader<T> {
    fn name(&self) -> &str {
        &self.name
    }

    fn schema(&self) -> SchemaId {
        T::SCHEMA
    }

    fn next(&mut self) -> Option<DynItem> {
        if self.ended {
            return None;
        }
        let item = self.exposed.with_next(|item| match item {
            Item::Event(event) => DynItem::Event {
                seq: event.seq,
                span: event.span,
                settled: event.settled,
                // A contract's events are plain data; serializing them cannot fail.
                payload: serde_json::to_value(&event.payload).unwrap_or(serde_json::Value::Null),
            },
            Item::Gap(range) => DynItem::Gap(*range),
            Item::Progress(tick) => DynItem::Progress(*tick),
            Item::End(tick) => DynItem::End(*tick),
        })?;
        self.exposed.advance();
        match &item {
            DynItem::Event { .. } => self.events += 1,
            DynItem::End(_) => self.ended = true,
            DynItem::Gap(_) | DynItem::Progress(_) => {}
        }
        Some(item)
    }

    fn is_ended(&self) -> bool {
        self.ended
    }

    fn pending(&self) -> usize {
        if self.ended {
            0
        } else {
            self.exposed.pending()
        }
    }

    fn events(&self) -> u64 {
        self.events
    }
}
