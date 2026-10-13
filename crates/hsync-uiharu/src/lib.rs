//! The uiharu decode engine on HSync's captures.
//!
//! A capture's samples are fed into decode pipelines as they arrive ([`Feeder`] cuts the
//! bit-packed channels of a [`hsync_capture::LogicCapture`] into the engine's blocks), and
//! the events every pipeline puts out are kept in a [`Store`] per stream, which answers the
//! interface's questions (lanes, rows, searches).

#![forbid(unsafe_code)]

mod feed;
mod search;
mod store;

pub use feed::{FeedError, Feeder};
pub use search::Needle;
pub use store::{LaneItem, Page, Row, Store, View};
