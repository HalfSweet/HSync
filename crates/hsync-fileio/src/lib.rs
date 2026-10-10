//! Capture files: DSView `.dsl` reading and writing (DESIGN.md §8).

#![forbid(unsafe_code)]

mod dsl;
mod error;
mod export;
pub mod header;
mod progress;

pub use dsl::{
    DslDocument, DslWriteRequest, read_dsl, read_dsl_file, write_atomically, write_dsl,
    write_dsl_file, write_dsl_with,
};
pub use error::FileError;
pub use export::{ExportFormat, ExportRequest, export, samplerate_text};
pub use header::DslHeader;
pub use progress::Progress;
