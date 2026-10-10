//! Decoding the open capture: the session's decoders run in the background and their
//! presentations answer the UI's lane and data stream queries (DESIGN.md §7.4).

use std::sync::Arc;

use hsync_capture::LogicCapture;
use hsync_decode::{Bus, DecoderView, Presentation, Spec, StreamCounts, StreamMode, StreamRow};
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
    /// No native decoder for this bus yet.
    Unsupported,
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

#[derive(Debug)]
enum State {
    Pending(Spec),
    Running,
    Done(Arc<Presentation>),
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
}

impl DecodeSet {
    /// Entries for DSView's `decoders` list, not run yet.
    pub fn new(decoders: Option<&Value>) -> Arc<Self> {
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
                match Spec::from_dsview(entry) {
                    Some(spec) => Entry {
                        id,
                        bus: Some(spec.bus),
                        state: State::Pending(spec),
                    },
                    None => Entry {
                        id,
                        bus: None,
                        state: State::Unsupported,
                    },
                }
            })
            .collect();
        Arc::new(Self {
            entries: RwLock::new(entries),
        })
    }

    /// Decodes every pending entry on its own thread; `done` hears of each one finished.
    pub fn run(
        self: &Arc<Self>,
        capture: &Arc<RwLock<LogicCapture>>,
        done: Arc<dyn Fn(u32) + Send + Sync>,
    ) {
        let mut entries = self.entries.write();
        for (index, entry) in entries.iter_mut().enumerate() {
            let State::Pending(spec) = std::mem::replace(&mut entry.state, State::Running) else {
                continue;
            };
            let set = Arc::clone(self);
            let capture = Arc::clone(capture);
            let done = Arc::clone(&done);
            let spawned = std::thread::Builder::new()
                .name(format!("decode-{}", spec.id))
                .spawn(move || {
                    let began = std::time::Instant::now();
                    let decoded = {
                        let capture = capture.read();
                        spec.run(&capture, capture.len())
                    };
                    log::debug!(
                        "decoded {} in {:.1} ms",
                        spec.id,
                        began.elapsed().as_secs_f64() * 1e3
                    );
                    let state = match decoded {
                        Some(decoded) => State::Done(Arc::new(Presentation::new(&decoded))),
                        None => State::MissingChannels,
                    };
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
                    State::Done(_) => DecodeState::Done,
                    State::MissingChannels => DecodeState::MissingChannels,
                    State::Unsupported => DecodeState::Unsupported,
                },
                counts: match &entry.state {
                    State::Done(presentation) => Some(presentation.counts()),
                    _ => None,
                },
            })
            .collect()
    }

    fn presentation(&self, index: u32) -> Option<Arc<Presentation>> {
        match &self.entries.read().get(index as usize)?.state {
            State::Done(presentation) => Some(Arc::clone(presentation)),
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
                State::Done(presentation) => Some((index as u32, Arc::clone(presentation))),
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
