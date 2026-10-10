//! A host for the sigrok-compatible Python protocol decoders DSView ships (DESIGN.md §7.3): CPython
//! embedded through pyo3, with the `sigrokdecode` module they import implemented in Rust on top of
//! `hsync-decode`'s wait engine.

mod catalog;
mod error;
mod host;
mod module;
mod python;
mod run;

pub use catalog::{
    AnnotationClass, AnnotationRow, ChannelSpec, DecoderInfo, OptionSpec, load_catalog,
};
pub use error::SrdError;
pub use host::DecoderHost;
pub use run::{Layer, LayerResult, Stack, StackOutcome, run_stack};
