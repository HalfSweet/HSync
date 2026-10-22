//! Capture sessions: the capture the user is looking at and the queries the UI runs on it
//!.
//!
//! Every query names the capture it was computed for; a capture replaced in the meantime makes
//! the query fail with [`SessionError::StaleCapture`] instead of answering for the wrong data.

#![forbid(unsafe_code)]

mod bins;
mod decoding;
mod dsview;
mod entry;
mod error;
mod files;
mod instrument;

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Arc, OnceLock};

use hsync_capture::{CaptureInfo, LogicCapture};
use hsync_core::{ChannelIndex, SampleIndex};
use hsync_driver::CapturePreset;
use hsync_fileio::ImportedCapture;
use indexmap::IndexMap;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uiharu_registry::Catalog;

pub use bins::{BINS_FORMAT_VERSION, BinsRequest, encode_bins};
pub use decoding::{
    DecodeSet, DecodeState, DecoderLanes, DecoderStatus, LANE_LIMIT, SEARCH_LIMIT, SearchHit,
    SearchResult, StreamLanes, StreamPage, StreamStatus,
};
pub use entry::{
    CatalogEntry, CatalogEvent, CatalogField, CatalogFieldKind, CatalogLocale, CatalogPort,
    DecoderEntry, ModuleInfo, catalog,
};
pub use error::SessionError;
pub use files::{ExportKind, ExportRequest, ExportSource, read_text, write_text};
pub use hsync_capture::{PatternCondition, PatternLevel};
pub use hsync_fileio::{
    ImportRequest, Waveform, WaveformFormat, WaveformItem, WaveformItemKind, WaveformSummary,
    read_waveform_file,
};
pub use hsync_uiharu::{
    EventRole, EventRow, LaneBlock, Position, RolePart, StreamProgress, StreamView,
};
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
    /// The decoders stored with the capture: HSync's entries (see [`DecoderEntry`]).
    #[ts(type = "unknown")]
    pub decoders: Option<serde_json::Value>,
    /// The interface's session document stored in an `.hsync` file, for the interface to
    /// apply; `None` for captures from elsewhere.
    #[ts(type = "unknown")]
    pub session: Option<serde_json::Value>,
}

/// The run of equal samples around a position, plus where the run after it ends: the two make
/// the period the hover read-out shows.
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
    /// DSView's `session` document of a capture read from a `.dsl` file, kept when exporting
    /// one again.
    session: Option<serde_json::Value>,
    /// The interface's session document, from an `.hsync` file or the last save.
    hsync_session: Option<serde_json::Value>,
    /// The device the capture came from, as the files record it.
    model: String,
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

/// A capture read from a file, about to become the open capture.
struct Loaded {
    source: CaptureSource,
    capture: LogicCapture,
    channel_names: BTreeMap<ChannelIndex, String>,
    decoders: Option<serde_json::Value>,
    driver: String,
    /// DSView's session document, from a `.dsl` file, written back when exporting one.
    session: Option<serde_json::Value>,
    /// The interface's session document, from an `.hsync` file.
    hsync_session: Option<serde_json::Value>,
    /// The device model the file records.
    model: String,
    /// The decoder modules the file carries, by name: each one's JSON.
    modules: BTreeMap<String, String>,
}

/// A decoder module loaded into the workspace: what the interface is told of it, and its
/// JSON as read, for the capture files that carry it.
struct LoadedModule {
    info: ModuleInfo,
    json: Arc<str>,
}

/// The captures of one window.
#[derive(Default)]
pub struct Workspace {
    current: Option<OpenCapture>,
    /// The capture shown while `current` is taken behind it, as repeated captures are, until
    /// `current` ends and replaces it (or is dropped).
    held: Option<OpenCapture>,
    last_id: u64,
    events: Option<EventSink>,
    /// The node types decoders may name: the engine's and the modules loaded.
    catalog: Catalog,
    /// The modules loaded, by name, in the order they were: on request, or brought by a
    /// capture file.
    modules: IndexMap<String, LoadedModule>,
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
    /// Tells the decoders where the capture has got, after every append and at the end.
    pub advance: Box<dyn Fn(Position) + Send + Sync>,
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

    /// Opens a capture file: HSync's own `.hsync`, or DSView's `.dsl`.
    pub fn open_file(&mut self, path: &Path) -> Result<CaptureSummary, SessionError> {
        let source = CaptureSource::File {
            path: path.to_string_lossy().into_owned(),
        };
        if hsync_fileio::is_hsync_path(path) {
            self.load_hsync(path, source)
        } else {
            self.load(path, source)
        }
    }

    /// Opens the bundled demo capture.
    pub fn open_demo(&mut self, path: &Path) -> Result<CaptureSummary, SessionError> {
        self.load(path, CaptureSource::Demo)
    }

    fn load(&mut self, path: &Path, source: CaptureSource) -> Result<CaptureSummary, SessionError> {
        let document = hsync_fileio::read_dsl_file(path)?;
        let channel_names = document.header.probes.iter().cloned().collect();
        self.install(Loaded {
            source,
            capture: document.capture,
            channel_names,
            // HSync's own entries, written beside DSView's list, which is translated otherwise.
            decoders: document.hsync_decoders.or(document.decoders),
            driver: document.header.driver,
            session: document.session,
            hsync_session: None,
            model: String::new(),
            modules: BTreeMap::new(),
        })
    }

    fn load_hsync(
        &mut self,
        path: &Path,
        source: CaptureSource,
    ) -> Result<CaptureSummary, SessionError> {
        let document = hsync_fileio::read_hsync_file(path)?;
        let channel_names = document
            .manifest
            .channels
            .iter()
            .map(|channel| (channel.index, channel.name.clone()))
            .collect();
        let (driver, model) = document
            .manifest
            .source
            .map(|origin| (origin.driver, origin.model))
            .unwrap_or_default();
        self.install(Loaded {
            source,
            capture: document.capture,
            channel_names,
            decoders: document.decoders,
            driver,
            session: None,
            hsync_session: document.session,
            model,
            modules: document.modules,
        })
    }

    /// Makes a capture imported from the waveform file at `path` the open capture. It comes
    /// without decoders, a device or DSView's session document.
    pub fn open_imported(
        &mut self,
        path: &Path,
        imported: ImportedCapture,
    ) -> Result<CaptureSummary, SessionError> {
        self.install(Loaded {
            source: CaptureSource::File {
                path: path.to_string_lossy().into_owned(),
            },
            capture: imported.capture,
            channel_names: imported.channel_names,
            decoders: None,
            driver: String::new(),
            session: None,
            hsync_session: None,
            model: String::new(),
            modules: BTreeMap::new(),
        })
    }

    /// Makes `loaded` the open capture, dropping the captures open so far, and starts its
    /// decoders, with the decoder modules it brought.
    fn install(&mut self, loaded: Loaded) -> Result<CaptureSummary, SessionError> {
        self.adopt(&loaded.source, loaded.modules);
        self.last_id += 1;
        let capture = Arc::new(RwLock::new(loaded.capture));
        for open in [self.current.take(), self.held.take()]
            .into_iter()
            .flatten()
        {
            open.decoding.cancel();
        }
        let decoders = normalize_decoders(loaded.decoders);
        let position = Position {
            len: capture.read().len(),
            dropped: 0,
            finishing: true,
        };
        let decoding = DecodeSet::start(
            decoders.as_ref(),
            &self.catalog,
            &capture,
            position,
            self.last_id,
            self.events.clone(),
        );
        self.current = Some(OpenCapture {
            id: self.last_id,
            source: loaded.source,
            capture,
            channel_names: loaded.channel_names,
            decoders,
            decoding,
            driver: loaded.driver,
            session: loaded.session,
            hsync_session: loaded.hsync_session,
            model: loaded.model,
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
    /// names and decoders of the capture it replaces, as DSView keeps them across captures;
    /// with no capture to replace, it takes those of the device's `preset`.
    ///
    /// With `hold`, the shown capture stays shown while `capture` fills behind it, as DSView's
    /// repeated captures fill a back buffer; [`LiveCapture::verdict`] then decides, once the
    /// capture has ended, whether it replaces the held one.
    pub fn begin_live(
        &mut self,
        source: CaptureSource,
        capture: LogicCapture,
        hold: bool,
        preset: Option<CapturePreset>,
    ) -> LiveCapture {
        self.last_id += 1;
        let samples = Arc::new(RwLock::new(capture));
        let shown = self.take_shown();
        let (channel_names, decoders) = match &shown {
            Some(open) => (open.channel_names.clone(), open.decoders.clone()),
            None => preset
                .map(|preset| (preset.channel_names, normalize_decoders(preset.decoders)))
                .unwrap_or_default(),
        };
        let verdict = (hold && shown.is_some()).then(Verdict::default);
        match shown {
            Some(open) if verdict.is_some() => self.held = Some(open),
            Some(open) => open.decoding.cancel(),
            None => {}
        }
        let decoding = DecodeSet::start(
            decoders.as_ref(),
            &self.catalog,
            &samples,
            Position::default(),
            self.last_id,
            self.events.clone(),
        );
        let (driver, model) = match &source {
            CaptureSource::Device { driver, model } => (driver.clone(), model.clone()),
            _ => (String::new(), String::new()),
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
            hsync_session: None,
            model,
            verdict: verdict.clone(),
        };
        let summary = summarize(&open);
        self.current = Some(open);
        LiveCapture {
            summary,
            samples,
            advance: Box::new(move |position| decoding.advance(position)),
            verdict,
        }
    }

    /// Replaces the decoders of capture `id` with `decoders` (HSync's entries; DSView's are
    /// translated) and decodes again, from where the capture stands — while it is still being
    /// taken too.
    pub fn set_decoders(
        &mut self,
        id: u64,
        decoders: serde_json::Value,
    ) -> Result<CaptureSummary, SessionError> {
        let events = self.events.clone();
        let catalog = self.catalog.clone();
        let open = self.find_mut(id)?;
        let decoders = normalize_decoders(Some(decoders));
        // Only how results are shown changed (labels, display settings): keep the results.
        if decoding_part(decoders.as_ref()) == decoding_part(open.decoders.as_ref()) {
            open.decoders = decoders;
            return Ok(summarize(open));
        }
        let decoding = DecodeSet::start(
            decoders.as_ref(),
            &catalog,
            &open.capture,
            open.decoding.position(),
            id,
            events,
        );
        open.decoding.cancel();
        open.decoders = decoders;
        open.decoding = decoding;
        Ok(summarize(open))
    }

    /// Loads the decoder module at `path` — a decoder written in the DSL, as its IR — under
    /// the module's name, in place of a decoder of that name built in or loaded before; the
    /// captures whose decoders name it decode again with it.
    pub fn load_module(&mut self, path: &Path) -> Result<ModuleInfo, SessionError> {
        let shown = path.to_string_lossy().into_owned();
        let module = |detail: String| SessionError::Module {
            path: shown.clone(),
            detail,
        };
        let json = std::fs::read_to_string(path).map_err(|error| module(error.to_string()))?;
        let loaded = self
            .catalog
            .load_json(&json)
            .map_err(|error| module(error.to_string()))?;
        let info = ModuleInfo {
            name: loaded.name().to_owned(),
            path: shown,
            category: entry::category_name(loaded.manifest().category),
            shadows: loaded.shadows_builtin(),
        };
        self.modules.shift_remove(&info.name);
        self.modules.insert(
            info.name.clone(),
            LoadedModule {
                info: info.clone(),
                json: json.into(),
            },
        );
        self.restart_using(&info.name);
        Ok(info)
    }

    /// Loads the decoder modules a capture file carries, under the file's path, except those
    /// whose name a module loaded already has: what was loaded on request, or brought by a
    /// file before, stays. The interface is told when any was loaded.
    fn adopt(&mut self, source: &CaptureSource, modules: BTreeMap<String, String>) {
        let CaptureSource::File { path } = source else {
            return;
        };
        let mut adopted = false;
        for (entry, json) in modules {
            // The module's own name decides, whatever the entry was called.
            let name = serde_json::from_str::<serde_json::Value>(&json)
                .ok()
                .and_then(|module| module.get("name")?.as_str().map(ToOwned::to_owned))
                .unwrap_or(entry);
            if self.modules.contains_key(&name) {
                continue;
            }
            match self.catalog.load_json(&json) {
                Ok(loaded) => {
                    let info = ModuleInfo {
                        name: loaded.name().to_owned(),
                        path: path.clone(),
                        category: entry::category_name(loaded.manifest().category),
                        shadows: loaded.shadows_builtin(),
                    };
                    self.modules.insert(
                        info.name.clone(),
                        LoadedModule {
                            info,
                            json: json.into(),
                        },
                    );
                    adopted = true;
                }
                Err(error) => {
                    log::warn!(
                        "{path}: the decoder module {name} it carries does not load: {error}"
                    );
                }
            }
        }
        if adopted && let Some(events) = &self.events {
            events(SessionEvent::ModulesChanged);
        }
    }

    /// Forgets the decoder module `name`; the captures whose decoders name it decode again
    /// with the decoder built in, if there is one. Whether there was such a module.
    pub fn unload_module(&mut self, name: &str) -> bool {
        if self.modules.shift_remove(name).is_none() {
            return false;
        }
        self.catalog.unload(name);
        self.restart_using(name);
        true
    }

    /// The decoder modules loaded, in the order they were; one a capture file brought names
    /// the file as its path.
    pub fn modules(&self) -> Vec<ModuleInfo> {
        self.modules
            .values()
            .map(|module| module.info.clone())
            .collect()
    }

    /// Every node type decoders may name, built in or loaded, with its manifest and defaults.
    pub fn catalog(&self) -> Vec<CatalogEntry> {
        entry::catalog_of(&self.catalog, |name| {
            self.modules
                .get(name)
                .map(|module| module.info.path.clone())
        })
    }

    /// Decodes again the captures whose decoders name the node type `kind`, from where they
    /// stand, with the node types as they are now.
    fn restart_using(&mut self, kind: &str) {
        let events = self.events.clone();
        let catalog = self.catalog.clone();
        for open in [self.current.as_mut(), self.held.as_mut()]
            .into_iter()
            .flatten()
        {
            if !open.decoding.uses(kind) {
                continue;
            }
            let decoding = DecodeSet::start(
                open.decoders.as_ref(),
                &catalog,
                &open.capture,
                open.decoding.position(),
                open.id,
                events.clone(),
            );
            open.decoding.cancel();
            open.decoding = decoding;
        }
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
            catalog: self.catalog.clone(),
            modules: self
                .modules
                .iter()
                .map(|(name, module)| (name.clone(), Arc::clone(&module.json)))
                .collect(),
            driver: open.driver.clone(),
            model: open.model.clone(),
            session: open.session.clone(),
        })
    }

    /// Notes that capture `id` was saved to `path` with the interface's `session`: it then
    /// counts as that file.
    pub fn finish_save(
        &mut self,
        id: u64,
        path: &Path,
        session: serde_json::Value,
    ) -> Result<CaptureSummary, SessionError> {
        let open = self.find_mut(id)?;
        open.hsync_session = Some(session);
        open.source = CaptureSource::File {
            path: path.to_string_lossy().into_owned(),
        };
        Ok(summarize(open))
    }

    /// Writes capture `id` to `path` as an `.hsync` file with its channel names, decoders and
    /// the interface's `session`; the capture then counts as that file.
    pub fn save(
        &mut self,
        id: u64,
        path: &Path,
        session: serde_json::Value,
    ) -> Result<CaptureSummary, SessionError> {
        self.export_source(id)?
            .save(path, &session, &mut |_| true)?;
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

/// HSync's decoder entries out of a list that may hold DSView's (from a `.dsl` file, a `.dsc`
/// session or an older HSync session), retaining original imported metadata even when no runnable translation exists.
fn normalize_decoders(decoders: Option<serde_json::Value>) -> Option<serde_json::Value> {
    let entries: Vec<serde_json::Value> = decoders
        .as_ref()
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .map(|entry| {
            if DecoderEntry::is_entry(entry) {
                entry.clone()
            } else {
                dsview::preserve(entry)
            }
        })
        .collect();
    (!entries.is_empty()).then_some(serde_json::Value::Array(entries))
}

/// What of a decoder list decides the results: each entry without the keys that only say how
/// they are shown.
fn decoding_part(decoders: Option<&serde_json::Value>) -> Vec<serde_json::Value> {
    const DISPLAY: [&str; 3] = ["show", "label", "import"];
    decoders
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .map(|entry| {
            let disabled = entry
                .pointer("/import/runnable")
                .and_then(serde_json::Value::as_bool)
                == Some(false);
            let mut entry = entry.clone();
            if let Some(fields) = entry.as_object_mut() {
                if disabled {
                    fields.insert("_importDisabled".into(), true.into());
                }
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
        session: open.hsync_session.clone(),
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
