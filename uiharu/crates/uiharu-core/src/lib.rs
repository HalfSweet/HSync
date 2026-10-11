// SPDX-License-Identifier: Apache-2.0

//! uiharu, a streaming protocol decode engine: decoders run as resumable state machines over
//! blocks of logic samples, exchange typed events through bounded streams with backpressure,
//! and are wired into a checked decode graph.

#![forbid(unsafe_code)]

pub mod node;
pub mod schema;
pub mod signal;
pub mod stream;
pub mod time;

pub use node::{Inbox, Io, Node, NodeCx, NodeFuture, NodeSpec, Outbox, Provenance, Wait};
pub use schema::{
    EventSchema, InputSpec, OutputSpec, PropValue, Props, Requirement, Requires, SchemaId,
};
pub use signal::{
    BlockError, Change, ChannelMask, LOGIC_SAMPLES, Levels, LogicBlock, LogicBlockBuilder,
    MAX_CHANNELS, Sample, SignalCursor, SignalItem,
};
pub use stream::{Broadcast, Event, Item, ReaderId};
pub use time::{Tick, TickRange, TimeBase};
