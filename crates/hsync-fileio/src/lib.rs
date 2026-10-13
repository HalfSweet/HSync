//! Capture files: DSView `.dsl` reading and writing, HDL waveforms (VCD, FST, GHW) laid onto a
//! sample grid, and exports.

#![forbid(unsafe_code)]

mod dsl;
mod error;
mod export;
pub mod header;
mod hsync;
mod progress;
mod waveform;

pub use dsl::{
    DslDocument, DslWriteRequest, read_dsl, read_dsl_file, write_atomically, write_dsl,
    write_dsl_file, write_dsl_with,
};
pub use error::FileError;
pub use export::{ExportFormat, ExportRequest, export, samplerate_text};
pub use header::DslHeader;
pub use hsync::{
    HSYNC_EXTENSION, HSYNC_FORMAT, HSYNC_VERSION, HsyncChannel, HsyncDocument, HsyncManifest,
    HsyncSource, HsyncTrigger, HsyncWriteRequest, is_hsync_path, read_hsync, read_hsync_file,
    write_hsync, write_hsync_file, write_hsync_with,
};
pub use progress::Progress;
pub use waveform::{
    ImportRequest, ImportedCapture, MAX_SAMPLES, ReadProgress, Waveform, WaveformFormat,
    WaveformItem, WaveformItemKind, WaveformSummary, read_waveform, read_waveform_file,
};
