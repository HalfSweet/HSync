//! Capture sessions: the capture the user is looking at and the queries the UI runs on it
//! (DESIGN.md §3.1, §5.3, §5.4).
//!
//! Every query names the capture it was computed for; a capture replaced in the meantime makes
//! the query fail with [`SessionError::StaleCapture`] instead of answering for the wrong data.

#![forbid(unsafe_code)]

mod bins;
mod decoding;
mod error;
mod files;
mod instrument;

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Arc, OnceLock};

use hsync_capture::{CaptureInfo, LogicCapture};
use hsync_core::{ChannelIndex, SampleIndex};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

pub use bins::{BINS_FORMAT_VERSION, BinsRequest, encode_bins};
pub use decoding::{
    DecodeSet, DecodeState, DecoderLanes, DecoderStatus, RANGE_KEY, SEARCH_LIMIT, SearchHit,
    SearchResult, StreamPage,
};
pub use error::SessionError;
pub use files::{ExportKind, ExportRequest, ExportSource, read_text, write_text};
pub use hsync_capture::{PatternCondition, PatternLevel};
pub use instrument::{
    CaptureRequest, DeviceState, EndKind, EventSink, Instrument, LivePhase, LiveStatus,
    SessionEvent,
};

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
    /// A capture taken with a device.
    Device {
        /// Driver of the device.
        driver: String,
        /// Model of the device.
        model: String,
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

/// Edges of a channel within a segment, as DSView's cursor measurement counts them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
pub struct ChannelEdges {
    pub channel: ChannelIndex,
    pub rising: u64,
    pub falling: u64,
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
    decoding: Arc<DecodeSet>,
    /// Driver that took the capture, as `.dsl` headers record it.
    driver: String,
    /// The `session` document of a capture read from a file, kept when saving it again.
    session: Option<serde_json::Value>,
    /// For a capture taken behind a held one: whether it replaces it, once it has ended.
    verdict: Option<Verdict>,
}

impl OpenCapture {
    /// Whether this capture is shown rather than the capture held in front of it.
    fn replaces_held(&self) -> bool {
        self.verdict
            .as_ref()
            .is_none_or(|verdict| verdict.get() == Some(&true))
    }
}

/// Whether a capture taken behind a held one replaces it; set when the capture ends.
pub type Verdict = Arc<OnceLock<bool>>;

/// The captures of one window.
#[derive(Default)]
pub struct Workspace {
    current: Option<OpenCapture>,
    /// The capture shown while `current` is taken behind it, as repeated captures are, until
    /// `current` ends and replaces it (or is dropped).
    held: Option<OpenCapture>,
    last_id: u64,
    events: Option<EventSink>,
    /// Runs the decoders without a native implementation, and those stacked on native ones.
    decoders: Option<Arc<hsync_srd::DecoderHost>>,
}

impl std::fmt::Debug for Workspace {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Workspace")
            .field("current", &self.current)
            .field("held", &self.held)
            .field("last_id", &self.last_id)
            .finish_non_exhaustive()
    }
}

/// A capture a device is filling, made the open capture of the workspace.
pub struct LiveCapture {
    pub summary: CaptureSummary,
    /// The samples to append to.
    pub samples: Arc<RwLock<LogicCapture>>,
    /// Decodes the capture once it is complete.
    pub decode: Box<dyn FnOnce() + Send>,
    /// For a capture taken behind the shown one: to be told whether it replaces it.
    pub verdict: Option<Verdict>,
}

impl std::fmt::Debug for LiveCapture {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LiveCapture")
            .field("summary", &self.summary)
            .finish_non_exhaustive()
    }
}

/// Tells `events` that decoder `index` of capture `capture_id` finished.
fn decoded_events(events: Option<EventSink>, capture_id: u64) -> Arc<dyn Fn(u32) + Send + Sync> {
    Arc::new(move |index| {
        if let Some(events) = &events {
            events(SessionEvent::Decoded { capture_id, index });
        }
    })
}

impl Workspace {
    /// Creates a workspace without a capture.
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a workspace that reports finished decoders to `events`.
    pub fn with_events(events: EventSink) -> Self {
        Self {
            events: Some(events),
            ..Self::default()
        }
    }

    /// Lets the Python decoders of `host` decode what no native decoder does.
    pub fn set_decoder_host(&mut self, host: Arc<hsync_srd::DecoderHost>) {
        self.decoders = Some(host);
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
        let capture = Arc::new(RwLock::new(document.capture));
        for open in [self.current.take(), self.held.take()]
            .into_iter()
            .flatten()
        {
            open.decoding.cancel();
        }
        let decoding = DecodeSet::new(document.decoders.as_ref(), self.decoders.clone());
        decoding.run(&capture, decoded_events(self.events.clone(), self.last_id));
        self.current = Some(OpenCapture {
            id: self.last_id,
            source,
            capture,
            channel_names,
            decoders: document.decoders,
            decoding,
            driver: document.header.driver,
            session: document.session,
            verdict: None,
        });
        self.summary().ok_or(SessionError::NoCapture)
    }

    /// The capture the user sees: a held capture until the one behind it replaces it.
    fn shown(&self) -> Option<&OpenCapture> {
        match (&self.current, &self.held) {
            (Some(current), Some(held)) if !current.replaces_held() => Some(held),
            (Some(current), _) => Some(current),
            (None, held) => held.as_ref(),
        }
    }

    /// Takes the shown capture out of the workspace and drops the other one.
    fn take_shown(&mut self) -> Option<OpenCapture> {
        let (shown, dropped) = match (self.current.take(), self.held.take()) {
            (Some(current), Some(held)) if !current.replaces_held() => (Some(held), Some(current)),
            (Some(current), held) => (Some(current), held),
            (None, held) => (held, None),
        };
        if let Some(dropped) = dropped {
            dropped.decoding.cancel();
        }
        shown
    }

    /// Capture `id`, shown or not.
    fn find(&self, id: u64) -> Result<&OpenCapture, SessionError> {
        let any = self.current.is_some() || self.held.is_some();
        [&self.current, &self.held]
            .into_iter()
            .flatten()
            .find(|open| open.id == id)
            .ok_or(if any {
                SessionError::StaleCapture
            } else {
                SessionError::NoCapture
            })
    }

    fn find_mut(&mut self, id: u64) -> Result<&mut OpenCapture, SessionError> {
        let any = self.current.is_some() || self.held.is_some();
        [&mut self.current, &mut self.held]
            .into_iter()
            .flatten()
            .find(|open| open.id == id)
            .ok_or(if any {
                SessionError::StaleCapture
            } else {
                SessionError::NoCapture
            })
    }

    /// Makes `capture`, which a device is about to fill, the open capture. It keeps the channel
    /// names and decoders of the capture it replaces, as DSView keeps them across captures.
    ///
    /// With `hold`, the shown capture stays shown while `capture` fills behind it, as DSView's
    /// repeated captures fill a back buffer; [`LiveCapture::verdict`] then decides, once the
    /// capture has ended, whether it replaces the held one.
    pub fn begin_live(
        &mut self,
        source: CaptureSource,
        capture: LogicCapture,
        hold: bool,
    ) -> LiveCapture {
        self.last_id += 1;
        let samples = Arc::new(RwLock::new(capture));
        let shown = self.take_shown();
        let (channel_names, decoders) = shown
            .as_ref()
            .map(|open| (open.channel_names.clone(), open.decoders.clone()))
            .unwrap_or_default();
        let verdict = (hold && shown.is_some()).then(Verdict::default);
        match shown {
            Some(open) if verdict.is_some() => self.held = Some(open),
            Some(open) => open.decoding.cancel(),
            None => {}
        }
        let decoding = DecodeSet::new(decoders.as_ref(), self.decoders.clone());
        let driver = match &source {
            CaptureSource::Device { driver, .. } => driver.clone(),
            _ => String::new(),
        };
        let open = OpenCapture {
            id: self.last_id,
            source,
            capture: Arc::clone(&samples),
            channel_names,
            decoders,
            decoding: Arc::clone(&decoding),
            driver,
            session: None,
            verdict: verdict.clone(),
        };
        let summary = summarize(&open);
        self.current = Some(open);
        let finished = Arc::clone(&samples);
        let done = decoded_events(self.events.clone(), self.last_id);
        LiveCapture {
            summary,
            samples,
            decode: Box::new(move || decoding.run(&finished, done)),
            verdict,
        }
    }

    /// Replaces the decoders of capture `id` with `decoders` (DSView's `decoders` list) and
    /// decodes again. A capture still being taken decodes once complete instead.
    pub fn set_decoders(
        &mut self,
        id: u64,
        decoders: serde_json::Value,
        capturing: bool,
    ) -> Result<CaptureSummary, SessionError> {
        let events = self.events.clone();
        let host = self.decoders.clone();
        let open = self.find_mut(id)?;
        if capturing {
            return Err(SessionError::Busy);
        }
        let decoders = (!decoders.as_array().is_some_and(Vec::is_empty)).then_some(decoders);
        // Only how results are shown changed (rows hidden, labels): keep the results.
        if decoding_part(decoders.as_ref()) == decoding_part(open.decoders.as_ref()) {
            open.decoders = decoders;
            return Ok(summarize(open));
        }
        let decoding = DecodeSet::new(decoders.as_ref(), host);
        decoding.run(&open.capture, decoded_events(events, id));
        open.decoding.cancel();
        open.decoders = decoders;
        open.decoding = decoding;
        Ok(summarize(open))
    }

    /// Renames channel `channel` of capture `id`; an empty name goes back to the channel number.
    pub fn rename_channel(
        &mut self,
        id: u64,
        channel: ChannelIndex,
        name: &str,
    ) -> Result<CaptureSummary, SessionError> {
        let open = self.find_mut(id)?;
        if open.capture.read().channel(channel).is_none() {
            return Err(SessionError::UnknownChannel(channel));
        }
        let name = name.trim();
        if name.is_empty() {
            open.channel_names.remove(&channel);
        } else {
            open.channel_names.insert(channel, name.to_owned());
        }
        Ok(summarize(open))
    }

    /// What saving or exporting capture `id` needs, taken so the writing can go on without
    /// holding the workspace.
    pub fn export_source(&self, id: u64) -> Result<ExportSource, SessionError> {
        let open = self.find(id)?;
        Ok(ExportSource {
            capture: Arc::clone(&open.capture),
            channel_names: open.channel_names.clone(),
            decoders: open.decoders.clone(),
            driver: open.driver.clone(),
            session: open.session.clone(),
        })
    }

    /// Notes that capture `id` was saved to `path` with `session`: it then counts as that file.
    pub fn finish_save(
        &mut self,
        id: u64,
        path: &Path,
        session: serde_json::Value,
    ) -> Result<CaptureSummary, SessionError> {
        let open = self.find_mut(id)?;
        open.session = Some(session);
        open.source = CaptureSource::File {
            path: path.to_string_lossy().into_owned(),
        };
        Ok(summarize(open))
    }

    /// Writes capture `id` to `path` as a DSView `.dsl` file with its channel names and
    /// decoders; the capture then counts as that file.
    pub fn save(&mut self, id: u64, path: &Path) -> Result<CaptureSummary, SessionError> {
        let session = self.export_source(id)?.save(path, &mut |_| true)?;
        self.finish_save(id, path, session)
    }

    /// The decoders of capture `id`.
    pub fn decoding(&self, id: u64) -> Result<Arc<DecodeSet>, SessionError> {
        self.find(id).map(|open| Arc::clone(&open.decoding))
    }

    /// Summary of the capture shown.
    pub fn summary(&self) -> Option<CaptureSummary> {
        self.shown().map(summarize)
    }

    /// The samples of capture `id`, shared so long queries do not hold the workspace.
    pub fn capture(&self, id: u64) -> Result<Arc<RwLock<LogicCapture>>, SessionError> {
        self.find(id).map(|open| Arc::clone(&open.capture))
    }
}

/// What of a decoder list decides the results: each entry without the keys that only say how
/// they are shown (DSView's `show`, `label`, `view_index`, HSync's number format).
fn decoding_part(decoders: Option<&serde_json::Value>) -> Vec<serde_json::Value> {
    const DISPLAY: [&str; 4] = ["show", "label", "view_index", "number format"];
    decoders
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .map(|entry| {
            let mut entry = entry.clone();
            if let Some(fields) = entry.as_object_mut() {
                for key in DISPLAY {
                    fields.remove(key);
                }
            }
            entry
        })
        .collect()
}

fn summarize(open: &OpenCapture) -> CaptureSummary {
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
    CaptureSummary {
        id: open.id,
        source: open.source.clone(),
        info,
        channels,
        decoders: open.decoders.clone(),
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

/// Rising edges considered by [`clock_period`]; enough for a stable median, bounded in time.
const CLOCK_EDGES: usize = 1 << 16;

/// The median number of samples between consecutive rising edges of `channel`, over up to
/// 65536 edges from the start: the period of the clock the channel carries. Shown next to the
/// I²C and SPI group titles; the median ignores the pauses between bursts.
pub fn clock_period(
    capture: &LogicCapture,
    channel: ChannelIndex,
) -> Result<Option<f64>, SessionError> {
    let data = capture
        .channel(channel)
        .ok_or(SessionError::UnknownChannel(channel))?;
    if data.is_empty() {
        return Ok(None);
    }
    let mut rising = Vec::new();
    let mut at = 0;
    while rising.len() < CLOCK_EDGES {
        let Some(edge) = data.next_change(at) else {
            break;
        };
        if data.sample(edge) {
            rising.push(edge);
        }
        at = edge;
    }
    let mut periods: Vec<u64> = rising.windows(2).map(|pair| pair[1] - pair[0]).collect();
    if periods.is_empty() {
        return Ok(None);
    }
    let middle = periods.len() / 2;
    let (_, median, _) = periods.select_nth_unstable(middle);
    Ok(Some(*median as f64))
}

/// Rising and falling edges of `channels` from sample `from` to sample `to`: the changes whose
/// new level starts after `from` and no later than `to`, as DSView's cursor measurement counts
/// them (`LogicSignal::edges`).
pub fn edges(
    capture: &LogicCapture,
    from: SampleIndex,
    to: SampleIndex,
    channels: &[ChannelIndex],
) -> Result<Vec<ChannelEdges>, SessionError> {
    let (from, to) = (from.min(to), from.max(to));
    channels
        .iter()
        .map(|&channel| {
            let data = capture
                .channel(channel)
                .ok_or(SessionError::UnknownChannel(channel))?;
            let counted = data.count_edges(from, to.saturating_add(1));
            Ok(ChannelEdges {
                channel,
                rising: counted.rising,
                falling: counted.falling,
            })
        })
        .collect()
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
