//! Decoding the open capture: every decoder entry runs as a uiharu pipeline on a thread of
//! its own, fed as the capture grows, and the stores of its streams answer the UI's lane,
//! row and search queries.

use std::ops::Range;
use std::sync::Arc;

use hsync_capture::LogicCapture;
use hsync_core::{SampleIndex, UiError};
use hsync_uiharu::{
    DecodeError, Decoder, EventRow, Needle, Position, StreamProgress, StreamView, Update,
};
use parking_lot::{Mutex, RwLock};
use serde::Serialize;
use serde_json::Value;
use ts_rs::TS;
use uiharu_registry::Catalog;

use crate::entry::DecoderEntry;
use crate::instrument::{EventSink, SessionEvent};

/// Hits a search returns at most.
pub const SEARCH_LIMIT: usize = 10_000;

/// Blocks a lane returns at most for one viewport.
pub const LANE_LIMIT: usize = 4096;

/// Where a decoder is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum DecodeState {
    /// Decoding, or waiting for the capture to go on.
    Running,
    /// Every stream has ended.
    Done,
    /// Decoding stopped; what was decoded before stays.
    Failed,
    /// A channel the pipeline reads is not in the capture.
    MissingChannels,
    /// The entry does not describe a pipeline that builds.
    Invalid,
}

/// One exposed output of a decoder: a lane, and a listing in the data stream.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct StreamStatus {
    /// Position among the decoder's streams.
    pub index: u32,
    /// The name the entry exposed it under.
    pub name: String,
    /// The node type whose output it is, e.g. `i2c`.
    pub kind: String,
    /// The schema of its events, e.g. `i2c.events`.
    pub schema: String,
    pub progress: StreamProgress,
}

/// A decoder of the open capture.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DecoderStatus {
    /// Position in the capture's decoder list.
    pub index: u32,
    /// The user's label, else the first node's type.
    pub label: String,
    /// The node types of the pipeline, the bus first.
    pub kinds: Vec<String>,
    pub state: DecodeState,
    /// Why it failed or could not start.
    pub error: Option<UiError>,
    pub streams: Vec<StreamStatus>,
}

/// A stream's lane in a viewport.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct StreamLanes {
    pub stream: u32,
    pub view: StreamView,
}

/// A decoder's lanes in a viewport.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DecoderLanes {
    pub index: u32,
    pub streams: Vec<StreamLanes>,
}

/// Part of a stream's listing.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct StreamPage {
    /// Rows in the requested scope.
    pub total: u32,
    /// Position of the first row returned within the scope.
    pub offset: u32,
    pub rows: Vec<EventRow>,
    /// Events of the whole stream.
    pub events: u64,
    /// With a segment: events lying wholly inside it.
    pub inside: Option<u32>,
}

/// An event the search box found.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SearchHit {
    /// Position of the decoder in the capture's decoder list.
    pub decoder: u32,
    /// Position of the stream among the decoder's.
    pub stream: u32,
    /// The event's row in the stream's listing.
    pub row: u32,
    pub start: SampleIndex,
    pub end: SampleIndex,
}

/// What a search found, in time order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SearchResult {
    pub hits: Vec<SearchHit>,
    /// More were found than `hits` holds.
    pub truncated: bool,
}

/// One entry: its decoder, or why it has none.
struct Entry {
    label: String,
    kinds: Vec<String>,
    decoder: Option<Decoder>,
    failure: Option<(DecodeState, UiError)>,
}

/// The decoders of one capture and their results.
pub struct DecodeSet {
    capture_id: u64,
    capture: Arc<RwLock<LogicCapture>>,
    /// The node types the entries may name, as they were when the set started.
    catalog: Catalog,
    entries: RwLock<Vec<Entry>>,
    /// Where the capture has got, as last told.
    position: Mutex<Position>,
    events: Option<EventSink>,
}

impl std::fmt::Debug for DecodeSet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DecodeSet")
            .field("capture_id", &self.capture_id)
            .field("entries", &self.entries.read().len())
            .field("position", &*self.position.lock())
            .finish_non_exhaustive()
    }
}

/// Why an entry could not start, as the interface is told.
fn failure(error: DecodeError) -> (DecodeState, UiError) {
    match error {
        DecodeError::MissingChannels(channels) => (
            DecodeState::MissingChannels,
            UiError::new("error.decode.missingChannels").with(
                "channels",
                channels
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(", "),
            ),
        ),
        DecodeError::Build(error) => (
            DecodeState::Invalid,
            UiError::new("error.decode.invalid").with("detail", error.to_string()),
        ),
        DecodeError::Feed(error) => (
            DecodeState::Failed,
            UiError::new("error.decode.failed").with("detail", error.to_string()),
        ),
        DecodeError::Pipeline(detail) => (
            DecodeState::Failed,
            UiError::new("error.decode.failed").with("detail", detail),
        ),
    }
}

impl DecodeSet {
    /// Starts decoding `capture`, which stands at `position`, with every entry of `decoders`
    /// (HSync's entries) over the node types of `catalog`; `events` hears of progress and ends
    /// under `capture_id`.
    pub fn start(
        decoders: Option<&Value>,
        catalog: &Catalog,
        capture: &Arc<RwLock<LogicCapture>>,
        position: Position,
        capture_id: u64,
        events: Option<EventSink>,
    ) -> Arc<Self> {
        let set = Self {
            capture_id,
            capture: Arc::clone(capture),
            catalog: catalog.clone(),
            entries: RwLock::new(Vec::new()),
            position: Mutex::new(position),
            events,
        };
        let entries: Vec<Entry> = decoders
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .enumerate()
            .map(|(index, value)| set.start_entry(index as u32, value))
            .collect();
        *set.entries.write() = entries;
        Arc::new(set)
    }

    fn start_entry(&self, index: u32, value: &Value) -> Entry {
        let entry = match DecoderEntry::parse(value) {
            Ok(entry) => entry,
            Err(error) => {
                return Entry {
                    label: value
                        .get("label")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned(),
                    kinds: Vec::new(),
                    decoder: None,
                    failure: Some((
                        DecodeState::Invalid,
                        UiError::new("error.decode.invalid").with("detail", error.to_string()),
                    )),
                };
            }
        };
        let rate = self.capture.read().samplerate();
        let description = entry.description(entry.source_channels(&self.catalog), rate);
        let events = self.events.clone();
        let capture_id = self.capture_id;
        let updates: Arc<dyn Fn(Update) + Send + Sync> = Arc::new(move |update| {
            let Some(events) = &events else {
                return;
            };
            match update {
                Update::Progress => events(SessionEvent::DecodeProgress { capture_id, index }),
                Update::Finished | Update::Failed(_) => {
                    events(SessionEvent::Decoded { capture_id, index });
                }
            }
        });
        let position = *self.position.lock();
        let (decoder, failure) = match Decoder::start(
            description,
            &self.catalog,
            Arc::clone(&self.capture),
            position,
            updates,
        ) {
            Ok(decoder) => (Some(decoder), None),
            Err(error) => {
                log::info!("decoder {index} ({}) cannot start: {error}", entry.label());
                (None, Some(failure(error)))
            }
        };
        Entry {
            label: entry.label(),
            kinds: entry.kinds(),
            decoder,
            failure,
        }
    }

    /// Tells every decoder where the capture has got.
    pub fn advance(&self, position: Position) {
        *self.position.lock() = position;
        for entry in self.entries.read().iter() {
            if let Some(decoder) = &entry.decoder {
                decoder.notify(position);
            }
        }
    }

    /// Where the capture stood when the set last heard of it.
    pub fn position(&self) -> Position {
        *self.position.lock()
    }

    /// Whether an entry names the node type `kind`.
    pub fn uses(&self, kind: &str) -> bool {
        self.entries
            .read()
            .iter()
            .any(|entry| entry.kinds.iter().any(|have| have == kind))
    }

    /// Stops every decoder; what was decoded stays readable.
    pub fn cancel(&self) {
        for entry in self.entries.read().iter() {
            if let Some(decoder) = &entry.decoder {
                decoder.cancel();
            }
        }
    }

    pub fn statuses(&self) -> Vec<DecoderStatus> {
        self.entries
            .read()
            .iter()
            .enumerate()
            .map(|(index, entry)| {
                let (state, error) = match (&entry.failure, &entry.decoder) {
                    (Some((state, error)), _) => (*state, Some(error.clone())),
                    (None, Some(decoder)) => match decoder.error() {
                        Some(detail) => (
                            DecodeState::Failed,
                            Some(UiError::new("error.decode.failed").with("detail", detail)),
                        ),
                        None if decoder.is_finished() => (DecodeState::Done, None),
                        None => (DecodeState::Running, None),
                    },
                    (None, None) => (DecodeState::Invalid, None),
                };
                let streams = entry.decoder.as_ref().map_or_else(Vec::new, |decoder| {
                    decoder
                        .streams()
                        .iter()
                        .enumerate()
                        .map(|(stream, info)| StreamStatus {
                            index: stream as u32,
                            name: info.name().to_owned(),
                            kind: info.kind().to_owned(),
                            schema: info.schema().name.to_owned(),
                            progress: decoder.progress(stream).unwrap_or(StreamProgress {
                                events: 0,
                                settled: 0,
                                ended: false,
                            }),
                        })
                        .collect()
                });
                DecoderStatus {
                    index: index as u32,
                    label: entry.label.clone(),
                    kinds: entry.kinds.clone(),
                    state,
                    error,
                    streams,
                }
            })
            .collect()
    }

    fn with_decoder<R>(&self, index: u32, f: impl FnOnce(&Decoder) -> Option<R>) -> Option<R> {
        let entries = self.entries.read();
        let decoder = entries.get(index as usize)?.decoder.as_ref()?;
        f(decoder)
    }

    /// Lanes of every decoder over `window`.
    pub fn view(&self, window: Range<SampleIndex>, samples_per_pixel: f64) -> Vec<DecoderLanes> {
        self.entries
            .read()
            .iter()
            .enumerate()
            .filter_map(|(index, entry)| {
                let decoder = entry.decoder.as_ref()?;
                let streams = (0..decoder.streams().len())
                    .filter_map(|stream| {
                        Some(StreamLanes {
                            stream: stream as u32,
                            view: decoder.view(
                                stream,
                                window.clone(),
                                samples_per_pixel,
                                LANE_LIMIT,
                            )?,
                        })
                    })
                    .collect();
                Some(DecoderLanes {
                    index: index as u32,
                    streams,
                })
            })
            .collect()
    }

    /// `count` rows of stream `stream` of decoder `index` from `offset`, of all rows or of
    /// those overlapping `segment`; `None` for a decoder or stream that is not there.
    pub fn stream(
        &self,
        index: u32,
        stream: u32,
        segment: Option<(SampleIndex, SampleIndex)>,
        only_segment: bool,
        offset: u32,
        count: u32,
    ) -> Option<StreamPage> {
        let ordered = segment.map(|(a, b)| a.min(b)..a.max(b));
        self.with_decoder(index, |decoder| {
            let stream = stream as usize;
            let scope = ordered.clone().filter(|_| only_segment);
            let page = decoder.page(stream, scope, offset as usize, count as usize)?;
            let progress = decoder.progress(stream)?;
            let inside = ordered.map(|range| decoder.inside(stream, range).unwrap_or(0) as u32);
            Some(StreamPage {
                total: page.total,
                offset: page.offset,
                rows: page.rows,
                events: progress.events,
                inside,
            })
        })
    }

    /// Position in stream `stream` of decoder `index` of the first event reaching past
    /// `sample`, to scroll the listing to a place.
    pub fn row_at(&self, index: u32, stream: u32, sample: SampleIndex) -> Option<u32> {
        self.with_decoder(index, |decoder| {
            decoder
                .row_at(stream as usize, sample)
                .map(|row| row as u32)
        })
    }

    /// Events matching `query` in every stream of every decoder, in time order; `None` for
    /// a blank query.
    pub fn search(&self, query: &str) -> Option<SearchResult> {
        let needle = Needle::parse(query)?;
        let mut hits = Vec::new();
        let mut truncated = false;
        for (index, entry) in self.entries.read().iter().enumerate() {
            let Some(decoder) = &entry.decoder else {
                continue;
            };
            for stream in 0..decoder.streams().len() {
                let Some((found, more)) = decoder.search(stream, &needle, SEARCH_LIMIT) else {
                    continue;
                };
                truncated |= more;
                hits.extend(found.into_iter().map(|hit| SearchHit {
                    decoder: index as u32,
                    stream: stream as u32,
                    row: hit.row,
                    start: hit.start,
                    end: hit.end,
                }));
            }
        }
        hits.sort_by_key(|hit| (hit.start, hit.decoder, hit.stream, hit.row));
        if hits.len() > SEARCH_LIMIT {
            hits.truncate(SEARCH_LIMIT);
            truncated = true;
        }
        Some(SearchResult { hits, truncated })
    }

    /// The stretches of `range` where decoder `index` has events on any stream, those
    /// closer than `resolution` samples joined; for the overview strip.
    pub fn coverage(
        &self,
        index: u32,
        range: Range<SampleIndex>,
        resolution: u64,
    ) -> Option<Vec<[SampleIndex; 2]>> {
        self.with_decoder(index, |decoder| {
            let mut stretches: Vec<[SampleIndex; 2]> = (0..decoder.streams().len())
                .filter_map(|stream| decoder.coverage(stream, range.clone(), resolution))
                .flatten()
                .collect();
            stretches.sort_unstable();
            let mut joined: Vec<[SampleIndex; 2]> = Vec::new();
            for [start, end] in stretches {
                match joined.last_mut() {
                    Some(last) if start <= last[1].saturating_add(resolution) => {
                        last[1] = last[1].max(end);
                    }
                    _ => joined.push([start, end]),
                }
            }
            Some(joined)
        })
    }
}
