//! The uiharu decode engine on HSync's captures.
//!
//! A capture's samples are fed into decode pipelines as they arrive: [`Feeder`] cuts the
//! bit-packed channels of a [`hsync_capture::LogicCapture`] into the engine's blocks.

#![forbid(unsafe_code)]

mod feed;

pub use feed::{FeedError, Feeder};
