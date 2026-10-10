//! Capture files: DSView `.dsl` reading and writing (DESIGN.md §8).

#![forbid(unsafe_code)]

mod dsl;
mod error;
pub mod header;

pub use dsl::{DslDocument, DslWriteRequest, read_dsl, read_dsl_file, write_dsl, write_dsl_file};
pub use error::FileError;
pub use header::DslHeader;
