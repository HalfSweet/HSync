//! Capture sessions: the capture the user is looking at and the queries the UI runs on it
//! (DESIGN.md §3.1, §5.3, §5.4).
//!
//! Every query names the capture it was computed for; a capture replaced in the meantime makes
//! the query fail with [`SessionError::StaleCapture`] instead of answering for the wrong data.

#![forbid(unsafe_code)]

mod bins;
mod error;

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use hsync_capture::{CaptureInfo, LogicCapture};
use hsync_core::{ChannelIndex, SampleIndex};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

pub use bins::{BINS_FORMAT_VERSION, BinsRequest, encode_bins};
pub use error::SessionError;

/// Where the open capture came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase")]
#[ts(export)]
pub enum CaptureSource {
    /// The bundled demo capture.
    Demo,
    /// A capture file.
    File {
        /// Path of the file.
        path: String,
    },
}

/// A channel of the open capture.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
pub struct ChannelSummary {
    /// Hardware channel number.
    pub index: ChannelIndex,
    /// Name given to the channel, e.g. `SDA`; DSView names unnamed channels by their number.
    pub name: String,
}

/// Everything the UI needs to present an open capture.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CaptureSummary {
    /// Identifies this capture in later queries.
    pub id: u64,
    /// Where the capture came from.
    pub source: CaptureSource,
    /// Timing and channels.
    pub info: CaptureInfo,
    /// Names of the channels, ascending by index.
    pub channels: Vec<ChannelSummary>,
    /// Decoder stacks stored with the capture, in DSView's `decoders` format.
    #[ts(type = "unknown")]
    pub decoders: Option<serde_json::Value>,
}

/// The run of equal samples around a position, plus the end of the run after it, which tells
/// whether the signal looks like a clock.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PulseInfo {
    /// Level of the run.
    pub high: bool,
    /// First sample of the run.
    pub start: SampleIndex,
    /// First sample after the run.
    pub end: SampleIndex,
    /// Whether `start` is an edge rather than the start of the capture.
    pub start_is_edge: bool,
    /// Whether `end` is an edge rather than the end of the capture.
    pub end_is_edge: bool,
    /// First sample after the following run, when that run ends at an edge.
    pub next_end: Option<SampleIndex>,
}

/// Direction of an edge search.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum Direction {
    /// Towards later samples.
    Next,
    /// Towards earlier samples.
    Previous,
}

#[derive(Debug)]
struct OpenCapture {
    id: u64,
    source: CaptureSource,
    capture: Arc<RwLock<LogicCapture>>,
    channel_names: BTreeMap<ChannelIndex, String>,
    decoders: Option<serde_json::Value>,
}

/// The captures of one window.
#[derive(Debug, Default)]
pub struct Workspace {
    current: Option<OpenCapture>,
    last_id: u64,
}

impl Workspace {
    /// Creates a workspace without a capture.
    pub fn new() -> Self {
        Self::default()
    }

    /// Opens a `.dsl` file.
    pub fn open_file(&mut self, path: &Path) -> Result<CaptureSummary, SessionError> {
        let source = CaptureSource::File {
            path: path.to_string_lossy().into_owned(),
        };
        self.load(path, source)
    }

    /// Opens the bundled demo capture.
    pub fn open_demo(&mut self, path: &Path) -> Result<CaptureSummary, SessionError> {
        self.load(path, CaptureSource::Demo)
    }

    fn load(&mut self, path: &Path, source: CaptureSource) -> Result<CaptureSummary, SessionError> {
        let document = hsync_fileio::read_dsl_file(path)?;
        let channel_names = document.header.probes.iter().cloned().collect();
        self.last_id += 1;
        self.current = Some(OpenCapture {
            id: self.last_id,
            source,
            capture: Arc::new(RwLock::new(document.capture)),
            channel_names,
            decoders: document.decoders,
        });
        self.summary().ok_or(SessionError::NoCapture)
    }

    /// Summary of the open capture.
    pub fn summary(&self) -> Option<CaptureSummary> {
        let open = self.current.as_ref()?;
        let info = open.capture.read().info();
        let channels = info
            .channels
            .iter()
            .map(|&index| ChannelSummary {
                index,
                name: open
                    .channel_names
                    .get(&index)
                    .cloned()
                    .unwrap_or_else(|| index.to_string()),
            })
            .collect();
        Some(CaptureSummary {
            id: open.id,
            source: open.source.clone(),
            info,
            channels,
            decoders: open.decoders.clone(),
        })
    }

    /// The samples of capture `id`, shared so long queries do not hold the workspace.
    pub fn capture(&self, id: u64) -> Result<Arc<RwLock<LogicCapture>>, SessionError> {
        match &self.current {
            Some(open) if open.id == id => Ok(Arc::clone(&open.capture)),
            Some(_) => Err(SessionError::StaleCapture),
            None => Err(SessionError::NoCapture),
        }
    }
}

/// The pulse of `channel` around `sample`.
pub fn pulse(
    capture: &LogicCapture,
    channel: ChannelIndex,
    sample: SampleIndex,
) -> Result<Option<PulseInfo>, SessionError> {
    let data = capture
        .channel(channel)
        .ok_or(SessionError::UnknownChannel(channel))?;
    let Some(pulse) = data.pulse(sample) else {
        return Ok(None);
    };
    let next_end = if pulse.end_is_edge {
        data.next_change(pulse.end)
    } else {
        None
    };
    Ok(Some(PulseInfo {
        high: pulse.high,
        start: pulse.start,
        end: pulse.end,
        start_is_edge: pulse.start_is_edge,
        end_is_edge: pulse.end_is_edge,
        next_end,
    }))
}

/// The nearest edge of `channel` after or before `sample`: the first sample of the new level.
pub fn edge(
    capture: &LogicCapture,
    channel: ChannelIndex,
    sample: SampleIndex,
    direction: Direction,
) -> Result<Option<SampleIndex>, SessionError> {
    let data = capture
        .channel(channel)
        .ok_or(SessionError::UnknownChannel(channel))?;
    Ok(match direction {
        Direction::Next => data.next_change(sample),
        Direction::Previous => data.prev_change(sample).map(|last| last + 1),
    })
}
