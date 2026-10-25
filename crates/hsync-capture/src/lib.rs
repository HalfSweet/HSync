//! Storage of logic captures: bit-packed samples per channel, multi-level summaries for
//! searches over billions of samples, and the queries behind waveform drawing, hover read-outs
//! and measurements; analog channels keep converter codes beside them.

#![forbid(unsafe_code)]

mod analog;
mod capture;
mod channel;
mod ingest;
mod pattern;
mod summary;
mod words;

pub use analog::AnalogData;
pub use capture::{CaptureInfo, LogicCapture};
pub use channel::{ChannelData, EdgeCount, Pulse, cell};
pub use ingest::CrossDeinterleaver;
pub use pattern::{PatternCondition, PatternLevel};
pub use words::{CHUNK_SAMPLES, CHUNK_WORDS, ChunkView};
