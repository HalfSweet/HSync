//! A host for the sigrok-compatible Python protocol decoders DSView ships (DESIGN.md §7.3): CPython
//! embedded through pyo3, with the `sigrokdecode` module they import implemented in Rust on top of
//! `hsync-decode`'s wait engine.

mod catalog;
mod error;
mod module;

pub use catalog::{
    AnnotationClass, AnnotationRow, ChannelSpec, DecoderInfo, OptionSpec, load_catalog,
};
pub use error::SrdError;
