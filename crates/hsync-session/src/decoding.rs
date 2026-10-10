//! Decoding the open capture: the session's decoders run in the background and their
//! presentations answer the UI's lane and data stream queries (DESIGN.md §7.4).

use std::sync::Arc;

use hsync_capture::LogicCapture;
use std::sync::atomic::{AtomicBool, Ordering};

use hsync_decode::{
    Annotation, Bus, DataLine, DecoderView, Needle, Presentation, Spec, StreamCounts, StreamMode,
    StreamRow,
};
use hsync_srd::{DecoderHost, LayerResult, Stack, StackOutcome};
use parking_lot::RwLock;
use serde::Serialize;
use serde_json::Value;
use ts_rs::TS;

/// Where a decoder is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum DecodeState {
    /// The capture is still being taken, or decoding has not finished.
    Running,
    Done,
    /// A channel the decoder needs is not in the capture.
    MissingChannels,
    /// Neither a native nor a Python decoder can run it.
    Unsupported,
    /// A Python decoder raised an error; what it put out before stays.
    Failed,
}

/// A lane of annotations from a decoder without a native presentation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
pub struct AnnotationLane {
    /// The lane's name in views: `decoder id/row id`.
    pub lane: String,
    /// The row's description, in English as the decoder has it.
    pub desc: String,
}

/// A decoder of the open capture.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[ts(export)]
pub struct DecoderStatus {
    /// Position in the capture's decoder list.
    pub index: u32,
    /// DSView's decoder id, e.g. `1:i2c`.
    pub id: String,
    pub bus: Option<Bus>,
    pub state: DecodeState,
    pub counts: Option<StreamCounts>,
    /// Lanes of annotations, below any native lanes.
    pub lanes: Vec<AnnotationLane>,
    /// Why a Python decoder failed.
    pub error: Option<String>,
}

/// A decoder's lanes in a viewport.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
pub struct DecoderLanes {
    pub index: u32,
    pub view: DecoderView,
}

/// Part of a decoder's data stream.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
pub struct StreamPage {
    /// Rows in the requested scope.
    pub total: u32,
    /// Position of the first row returned within the scope.
    pub offset: u32,
    pub rows: Vec<StreamRow>,
    pub counts: StreamCounts,
    /// With a segment: bytes (or frames) entirely inside it.
    pub inside: Option<u32>,
}

/// Something the search box found in a decoder's results.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
pub struct SearchHit {
    /// Position of the decoder in the capture's decoder list.
    pub decoder: u32,
    pub start: u64,
    pub end: u64,
    /// The SPI line it was found on.
    pub line: Option<DataLine>,
    /// The row of the decoder's data stream that lists it.
    pub row: Option<u32>,
}

/// What a search found, in time order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
pub struct SearchResult {
    pub hits: Vec<SearchHit>,
    /// More were found than `hits` holds.
    pub truncated: bool,
}

/// Hits a search returns at most.
pub const SEARCH_LIMIT: usize = 10_000;

/// How an entry is decoded.
#[derive(Debug)]
enum Job {
    Native(Spec),
    /// A native decoder with Python decoders stacked on it: the native one draws the bus, the
    /// whole stack runs in Python for the decoders above.
    NativeStacked(Spec, Stack),
    Python(Stack),
}

#[derive(Debug)]
enum State {
    Pending(Job),
    Running,
    Done(Arc<Presentation>, Vec<AnnotationLane>),
    Failed(Arc<Presentation>, Vec<AnnotationLane>, String),
    MissingChannels,
    Unsupported,
}

#[derive(Debug)]
struct Entry {
    id: String,
    bus: Option<Bus>,
    state: State,
}

/// The decoders of one capture and their results.
#[derive(Debug, Default)]
pub struct DecodeSet {
    entries: RwLock<Vec<Entry>>,
    host: Option<Arc<DecoderHost>>,
    /// Stops the Python decoders of a set that was replaced.
    cancel: Arc<AtomicBool>,
}

/// Lanes for the annotations of `layers`, named `decoder id/row id`, and the annotations with
/// their rows counted across all of them.
fn annotation_lanes(layers: &[LayerResult]) -> (Vec<AnnotationLane>, Vec<Annotation>) {
    let mut lanes = Vec::new();
    let mut annotations = Vec::new();
    for layer in layers {
        let offset = lanes.len() as u32;
        lanes.extend(layer.rows.iter().map(|(row, desc)| AnnotationLane {
            lane: format!("{}/{row}", layer.id),
            desc: desc.clone(),
        }));
        annotations.extend(layer.annotations.iter().cloned().map(|mut annotation| {
            annotation.row += offset;
            annotation
        }));
    }
    (lanes, annotations)
}

/// Why a stack fell short: the error that stopped it, or that of a decoder stacked on top.
fn failure(outcome: &StackOutcome) -> Option<String> {
    outcome
        .error
        .as_ref()
        .map(ToString::to_string)
        .or_else(|| outcome.layers.iter().find_map(|layer| layer.error.clone()))
}

impl DecodeSet {
    /// Entries for DSView's `decoders` list, not run yet. Without `host` only the native
    /// decoders run, and stacked decoders are left out.
    pub fn new(decoders: Option<&Value>, host: Option<Arc<DecoderHost>>) -> Arc<Self> {
        let entries = decoders
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .map(|entry| {
                let id = entry
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned();
                let stack = host
                    .as_ref()
                    .and_then(|_| Stack::from_dsview(entry))
                    .filter(|stack| !stack.layers.is_empty());
                let (bus, state) = match (Spec::from_dsview(entry), stack) {
                    (Some(spec), Some(stack)) if stack.layers.len() > 1 => (
                        Some(spec.bus),
                        State::Pending(Job::NativeStacked(spec, stack)),
                    ),
                    (Some(spec), _) => (Some(spec.bus), State::Pending(Job::Native(spec))),
                    (None, Some(stack)) => (Some(Bus::Other), State::Pending(Job::Python(stack))),
                    (None, None) => (None, State::Unsupported),
                };
                Entry { id, bus, state }
            })
            .collect();
        Arc::new(Self {
            entries: RwLock::new(entries),
            host,
            cancel: Arc::new(AtomicBool::new(false)),
        })
    }

    /// Stops the Python decoders still running; their results are not needed any more.
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    /// Runs one job over the capture.
    fn decode(
        host: Option<&DecoderHost>,
        job: Job,
        capture: &Arc<RwLock<LogicCapture>>,
        cancel: &Arc<AtomicBool>,
    ) -> State {
        let native = |spec: &Spec| {
            let capture = capture.read();
            spec.run(&capture, 0..capture.len())
        };
        let python = |stack: &Stack| -> Result<StackOutcome, String> {
            let host = host.ok_or("no decoder host")?;
            let end = capture.read().len();
            hsync_srd::run_stack(host, capture, end, stack, cancel)
                .map_err(|error| error.to_string())
        };
        match job {
            Job::Native(spec) => match native(&spec) {
                Some(decoded) => State::Done(Arc::new(Presentation::new(&decoded)), Vec::new()),
                None => State::MissingChannels,
            },
            Job::NativeStacked(spec, stack) => {
                let Some(decoded) = native(&spec) else {
                    return State::MissingChannels;
                };
                let mut presentation = Presentation::new(&decoded);
                match python(&stack) {
                    Ok(outcome) => {
                        let (lanes, annotations) = annotation_lanes(&outcome.layers[1..]);
                        let names: Vec<String> =
                            lanes.iter().map(|lane| lane.lane.clone()).collect();
                        presentation.append_lanes(&names, annotations);
                        match failure(&outcome) {
                            None => State::Done(Arc::new(presentation), lanes),
                            Some(error) => State::Failed(Arc::new(presentation), lanes, error),
                        }
                    }
                    Err(error) => State::Failed(Arc::new(presentation), Vec::new(), error),
                }
            }
            Job::Python(stack) => match python(&stack) {
                Ok(outcome) => {
                    let (lanes, annotations) = annotation_lanes(&outcome.layers);
                    let names: Vec<String> = lanes.iter().map(|lane| lane.lane.clone()).collect();
                    let presentation = Arc::new(Presentation::annotations(&names, annotations));
                    match failure(&outcome) {
                        None => State::Done(presentation, lanes),
                        Some(error) => State::Failed(presentation, lanes, error),
                    }
                }
                Err(error) if error.contains("required channel") => State::MissingChannels,
                Err(error) => State::Failed(
                    Arc::new(Presentation::annotations(&[], Vec::new())),
                    Vec::new(),
                    error,
                ),
            },
        }
    }

    /// Decodes every pending entry on its own thread; `done` hears of each one finished.
    pub fn run(
        self: &Arc<Self>,
        capture: &Arc<RwLock<LogicCapture>>,
        done: Arc<dyn Fn(u32) + Send + Sync>,
    ) {
        let mut entries = self.entries.write();
        for (index, entry) in entries.iter_mut().enumerate() {
            let State::Pending(job) = std::mem::replace(&mut entry.state, State::Running) else {
                continue;
            };
            let set = Arc::clone(self);
            let capture = Arc::clone(capture);
            let done = Arc::clone(&done);
            let id = entry.id.clone();
            let spawned = std::thread::Builder::new()
                .name(format!("decode-{id}"))
                .spawn(move || {
                    let began = std::time::Instant::now();
                    let state = Self::decode(set.host.as_deref(), job, &capture, &set.cancel);
                    log::debug!(
                        "decoded {id} in {:.1} ms",
                        began.elapsed().as_secs_f64() * 1e3
                    );
                    if let Some(entry) = set.entries.write().get_mut(index) {
                        entry.state = state;
                    }
                    done(index as u32);
                });
            if let Err(error) = spawned {
                log::warn!("starting the decoder failed: {error}");
                entry.state = State::MissingChannels;
            }
        }
    }

    pub fn statuses(&self) -> Vec<DecoderStatus> {
        self.entries
            .read()
            .iter()
            .enumerate()
            .map(|(index, entry)| DecoderStatus {
                index: index as u32,
                id: entry.id.clone(),
                bus: entry.bus,
                state: match entry.state {
                    State::Pending(_) | State::Running => DecodeState::Running,
                    State::Done(..) => DecodeState::Done,
                    State::Failed(..) => DecodeState::Failed,
                    State::MissingChannels => DecodeState::MissingChannels,
                    State::Unsupported => DecodeState::Unsupported,
                },
                counts: match &entry.state {
                    State::Done(presentation, _) | State::Failed(presentation, ..) => {
                        Some(presentation.counts())
                    }
                    _ => None,
                },
                lanes: match &entry.state {
                    State::Done(_, lanes) | State::Failed(_, lanes, _) => lanes.clone(),
                    _ => Vec::new(),
                },
                error: match &entry.state {
                    State::Failed(_, _, error) => Some(error.clone()),
                    _ => None,
                },
            })
            .collect()
    }

    fn presentation(&self, index: u32) -> Option<Arc<Presentation>> {
        match &self.entries.read().get(index as usize)?.state {
            State::Done(presentation, _) | State::Failed(presentation, ..) => {
                Some(Arc::clone(presentation))
            }
            _ => None,
        }
    }

    /// Lanes of every finished decoder over `start..end`.
    pub fn view(&self, start: u64, end: u64, samples_per_pixel: f64) -> Vec<DecoderLanes> {
        let finished: Vec<(u32, Arc<Presentation>)> = self
            .entries
            .read()
            .iter()
            .enumerate()
            .filter_map(|(index, entry)| match &entry.state {
                State::Done(presentation, _) | State::Failed(presentation, ..) => {
                    Some((index as u32, Arc::clone(presentation)))
                }
                _ => None,
            })
            .collect();
        finished
            .into_iter()
            .map(|(index, presentation)| DecoderLanes {
                index,
                view: presentation.view(start, end, samples_per_pixel),
            })
            .collect()
    }

    /// `count` rows of decoder `index` from `offset`, of all rows or of those overlapping
    /// `segment`.
    pub fn stream(
        &self,
        index: u32,
        mode: StreamMode,
        segment: Option<(u64, u64)>,
        only_segment: bool,
        offset: u32,
        count: u32,
    ) -> Option<StreamPage> {
        let presentation = self.presentation(index)?;
        let rows = presentation.rows(mode);
        // Rows are not in time order (SPI lists MISO before MOSI), so scope them by scanning.
        let scoped: Vec<&StreamRow> = match segment {
            Some((from, to)) if only_segment => rows
                .iter()
                .filter(|row| row.end() >= from && row.start() < to)
                .collect(),
            _ => rows.iter().collect(),
        };
        let begin = (offset as usize).min(scoped.len());
        let finish = (begin + count as usize).min(scoped.len());
        Some(StreamPage {
            total: scoped.len() as u32,
            offset: begin as u32,
            rows: scoped[begin..finish]
                .iter()
                .map(|&row| row.clone())
                .collect(),
            counts: presentation.counts(),
            inside: segment.map(|(from, to)| presentation.count_inside(from, to)),
        })
    }

    /// Values or text matching `query` in every finished decoder, with their rows in `mode`
    /// listings; `None` for a blank query.
    pub fn search(&self, query: &str, mode: StreamMode) -> Option<SearchResult> {
        let needle = Needle::parse(query)?;
        let finished: Vec<(u32, Arc<Presentation>)> = self
            .entries
            .read()
            .iter()
            .enumerate()
            .filter_map(|(index, entry)| match &entry.state {
                State::Done(presentation, _) | State::Failed(presentation, ..) => {
                    Some((index as u32, Arc::clone(presentation)))
                }
                _ => None,
            })
            .collect();
        let mut hits: Vec<SearchHit> = finished
            .iter()
            .flat_map(|(index, presentation)| {
                presentation
                    .search(&needle, mode)
                    .into_iter()
                    .map(|found| SearchHit {
                        decoder: *index,
                        start: found.start,
                        end: found.end,
                        line: found.line,
                        row: found.row,
                    })
            })
            .collect();
        hits.sort_by_key(|hit| (hit.start, hit.decoder));
        let truncated = hits.len() > SEARCH_LIMIT;
        hits.truncate(SEARCH_LIMIT);
        Some(SearchResult { hits, truncated })
    }

    /// Position of the first row (of all rows in `mode`) ending after `sample`, to scroll the
    /// stream to a place.
    pub fn row_at(&self, index: u32, mode: StreamMode, sample: u64) -> Option<u32> {
        let presentation = self.presentation(index)?;
        let rows = presentation.rows(mode);
        Some(
            rows.iter()
                .position(|row| row.end() >= sample)
                .unwrap_or(rows.len()) as u32,
        )
    }
}
