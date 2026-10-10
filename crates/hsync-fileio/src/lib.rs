//! Capture files: DSView `.dsl` reading and writing, HDL waveforms (VCD, FST, GHW) laid onto a
//! sample grid, and exports.

#![forbid(unsafe_code)]

mod dsl;
mod error;
mod export;
pub mod header;
mod progress;
mod waveform;

pub use dsl::{
    DslDocument, DslWriteRequest, read_dsl, read_dsl_file, write_atomically, write_dsl,
    write_dsl_file, write_dsl_with,
};
pub use error::FileError;
pub use export::{ExportFormat, ExportRequest, export, samplerate_text};
pub use header::DslHeader;
pub use progress::Progress;
pub use waveform::{
    ImportRequest, ImportedCapture, MAX_SAMPLES, ReadProgress, Waveform, WaveformFormat,
    WaveformItem, WaveformItemKind, WaveformSummary, read_waveform, read_waveform_file,
};
