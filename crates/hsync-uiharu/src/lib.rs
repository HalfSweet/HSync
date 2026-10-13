//! The uiharu decode engine on HSync's captures.
//!
//! A capture's samples are fed into decode pipelines as they arrive ([`Feeder`] cuts the
//! bit-packed channels of a [`hsync_capture::LogicCapture`] into the engine's blocks), the
//! events every pipeline puts out are kept in a [`Store`] per stream, which answers the
//! interface's questions (lanes, rows, searches), and a [`Decoder`] runs all of it on a
//! thread of its own so that decoding keeps up with a capture in progress.

#![forbid(unsafe_code)]

mod feed;
mod run;
mod search;
mod store;

pub use feed::{FeedError, Feeder};
pub use run::{DecodeError, Decoder, Position, Stream, Update, absolute, channels_read};
pub use search::Needle;
pub use store::{LaneItem, Page, Row, Store, View};
