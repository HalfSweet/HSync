//! Running one pipeline on a capture, on a thread of its own, as the capture is taken.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, Sender, TryRecvError};
use hsync_capture::LogicCapture;
use hsync_core::{ChannelIndex, SampleIndex};
use parking_lot::{Mutex, RwLock, RwLockReadGuard};
use uiharu_core::{PushError, SchemaId, SignalItem, Tick};
use uiharu_registry::{BuildError, Catalog, Description, DynItem, DynReader, EventKind, FieldKind};

use crate::feed::{FeedError, Feeder};
use crate::store::Store;

/// Why a decoder could not start, or stopped.
#[derive(Debug, thiserror::Error)]
pub enum DecodeError {
    /// The description does not build.
    #[error(transparent)]
    Build(#[from] BuildError),
    /// Channels the pipeline reads are not in the capture.
    #[error("channels {0:?} are not in the capture")]
    MissingChannels(Vec<ChannelIndex>),
    /// The source's channels cannot be fed.
    #[error(transparent)]
    Feed(#[from] FeedError),
    /// The pipeline refused an item or could make no progress.
    #[error("{0}")]
    Pipeline(String),
}

/// What the interface is told as decoding goes on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Update {
    /// More events, or a later settled tick, can be read.
    Progress,
    /// Every stream has ended.
    Finished,
    /// Decoding stopped; what was read before stays.
    Failed(String),
}

/// How far the capture has got: told to the decoder at the start and at every change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Position {
    /// Samples the capture holds.
    pub len: SampleIndex,
    /// Samples it dropped from its front before those.
    pub dropped: SampleIndex,
    /// Whether no more samples will come.
    pub finishing: bool,
}

/// Pruning events of a loop capture: not at every drop, which would mean rebuilding the stores
/// all the time, but once this many samples went.
const PRUNE_STEP: u64 = 1 << 20;

/// How often the interface is told of progress at most.
const PROGRESS_INTERVAL: Duration = Duration::from_millis(50);

enum Signal {
    Position(Position),
    Cancel,
}

/// One exposed output of the pipeline, with its events.
pub struct Stream {
    name: String,
    kind: String,
    schema: SchemaId,
    store: RwLock<Store>,
}

impl Stream {
    /// The name the description exposed it under.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The node type whose output it is.
    pub fn kind(&self) -> &str {
        &self.kind
    }

    /// The schema of its events.
    pub fn schema(&self) -> SchemaId {
        self.schema
    }

    /// Its events, for reading.
    pub fn store(&self) -> RwLockReadGuard<'_, Store> {
        self.store.read()
    }
}

impl std::fmt::Debug for Stream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Stream")
            .field("name", &self.name)
            .field("kind", &self.kind)
            .field("events", &self.store.read().len())
            .finish()
    }
}

/// Shared between a decoder and its thread.
struct Shared {
    streams: Vec<Arc<Stream>>,
    dropped: AtomicU64,
    finished: AtomicBool,
    error: Mutex<Option<String>>,
}

/// A pipeline decoding a capture on its own thread.
pub struct Decoder {
    shared: Arc<Shared>,
    control: Sender<Signal>,
    thread: Option<JoinHandle<()>>,
}

impl std::fmt::Debug for Decoder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Decoder")
            .field("streams", &self.shared.streams)
            .field("finished", &self.shared.finished.load(Ordering::Acquire))
            .finish_non_exhaustive()
    }
}

/// The channels the nodes of `description` read, from their configuration and the defaults
/// of their type, among the node types of `catalog`.
pub fn channels_read(description: &Description, catalog: &Catalog) -> BTreeSet<ChannelIndex> {
    let mut channels = BTreeSet::new();
    for node in description.nodes.values() {
        let Some(kind) = catalog.get(&node.kind) else {
            continue;
        };
        let manifest = kind.manifest();
        let defaults = kind.defaults();
        for field in &manifest.config {
            let value = node
                .config
                .get(&field.name)
                .or_else(|| defaults.get(&field.name));
            let Some(value) = value else {
                continue;
            };
            match field.kind {
                FieldKind::Channel => channels.extend(channel_of(value)),
                FieldKind::Channels { .. } => {
                    if let Some(values) = value.as_array() {
                        channels.extend(values.iter().filter_map(channel_of));
                    }
                }
                _ => {}
            }
        }
    }
    channels
}

/// The kinds of event, with their roles, of a stream of node type `kind` carrying `schema`:
/// those of the type's manifest, which lists the events of its first output.
fn events_of(catalog: &Catalog, kind: &str, schema: SchemaId) -> Vec<EventKind> {
    catalog
        .get(kind)
        .map(|kind| kind.manifest())
        .filter(|manifest| {
            manifest
                .outputs
                .first()
                .is_some_and(|output| output.schema == schema.name)
        })
        .map(|manifest| manifest.events.clone())
        .unwrap_or_default()
}

fn channel_of(value: &serde_json::Value) -> Option<ChannelIndex> {
    value
        .as_u64()
        .and_then(|channel| ChannelIndex::try_from(channel).ok())
}

impl Decoder {
    /// Starts decoding `capture`, which stands at `position`, with the pipeline `description`
    /// describes over the node types of `catalog`; `updates` hears how it goes. Fails when the
    /// description does not build or reads channels the capture lacks.
    pub fn start(
        description: Description,
        catalog: &Catalog,
        capture: Arc<RwLock<LogicCapture>>,
        position: Position,
        updates: Arc<dyn Fn(Update) + Send + Sync>,
    ) -> Result<Self, DecodeError> {
        let missing: Vec<ChannelIndex> = {
            let capture = capture.read();
            channels_read(&description, catalog)
                .into_iter()
                .filter(|&channel| capture.channel(channel).is_none())
                .collect()
        };
        if !missing.is_empty() {
            return Err(DecodeError::MissingChannels(missing));
        }
        // Built once here to check the description and learn its streams; the thread builds
        // its own, as a pipeline stays on the thread that runs it.
        let built = uiharu_registry::build_with(&description, catalog)?;
        Feeder::new(built.channels, Feeder::DEFAULT_BLOCK)?;
        let streams: Vec<Arc<Stream>> = built
            .streams
            .iter()
            .map(|stream| {
                let schema = stream.reader.schema();
                Arc::new(Stream {
                    name: stream.name.clone(),
                    kind: stream.kind.clone(),
                    schema,
                    store: RwLock::new(Store::for_stream(
                        schema,
                        events_of(catalog, stream.kind.as_str(), schema),
                    )),
                })
            })
            .collect();
        drop(built);
        let shared = Arc::new(Shared {
            streams,
            dropped: AtomicU64::new(position.dropped),
            finished: AtomicBool::new(false),
            error: Mutex::new(None),
        });
        let (control, signals) = crossbeam_channel::unbounded();
        let worker = Worker {
            description,
            catalog: catalog.clone(),
            capture,
            shared: Arc::clone(&shared),
            signals,
            updates,
            position,
            pruned_to: position.dropped,
            progress_due: false,
            last_progress: None,
        };
        let thread = std::thread::Builder::new()
            .name("decode-uiharu".into())
            .spawn(move || worker.run())
            .map_err(|error| DecodeError::Pipeline(error.to_string()))?;
        Ok(Self {
            shared,
            control,
            thread: Some(thread),
        })
    }

    /// Tells the decoder where the capture has got.
    pub fn notify(&self, position: Position) {
        self.shared
            .dropped
            .store(position.dropped, Ordering::Release);
        let _ = self.control.send(Signal::Position(position));
    }

    /// Stops decoding; what was decoded stays readable.
    pub fn cancel(&self) {
        let _ = self.control.send(Signal::Cancel);
    }

    /// Waits for the thread to end, after [`Decoder::cancel`] or the end of the capture.
    pub fn join(&mut self) {
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }

    /// The exposed outputs, in the description's order.
    pub fn streams(&self) -> &[Arc<Stream>] {
        &self.shared.streams
    }

    /// Samples the capture dropped from its front, as last told: the stores count ticks
    /// from the first sample the capture ever held, the capture from the first it still has.
    pub fn dropped(&self) -> SampleIndex {
        self.shared.dropped.load(Ordering::Acquire)
    }

    /// Whether every stream has ended, or decoding stopped.
    pub fn is_finished(&self) -> bool {
        self.shared.finished.load(Ordering::Acquire)
    }

    /// Why decoding stopped, if it failed.
    pub fn error(&self) -> Option<String> {
        self.shared.error.lock().clone()
    }

    /// The tick before which every stream has reported all it will.
    pub fn settled(&self) -> Tick {
        self.shared
            .streams
            .iter()
            .map(|stream| stream.store().settled())
            .min()
            .unwrap_or(Tick::ZERO)
    }
}

impl Drop for Decoder {
    fn drop(&mut self) {
        self.cancel();
    }
}

/// The thread's side.
struct Worker {
    description: Description,
    catalog: Catalog,
    capture: Arc<RwLock<LogicCapture>>,
    shared: Arc<Shared>,
    signals: Receiver<Signal>,
    updates: Arc<dyn Fn(Update) + Send + Sync>,
    position: Position,
    pruned_to: SampleIndex,
    progress_due: bool,
    last_progress: Option<Instant>,
}

enum Step {
    Go,
    Stop,
}

impl Worker {
    fn run(mut self) {
        let outcome = self.decode();
        match outcome {
            Ok(()) => {
                self.shared.finished.store(true, Ordering::Release);
                (self.updates)(Update::Finished);
            }
            Err(Some(error)) => {
                let text = error.to_string();
                *self.shared.error.lock() = Some(text.clone());
                self.shared.finished.store(true, Ordering::Release);
                (self.updates)(Update::Failed(text));
            }
            // Cancelled: nobody is listening any more.
            Err(None) => {}
        }
    }

    /// Decodes until the streams end; `Err(None)` when cancelled.
    fn decode(&mut self) -> Result<(), Option<DecodeError>> {
        let mut built = uiharu_registry::build_with(&self.description, &self.catalog)
            .map_err(DecodeError::from)?;
        let source = built.source;
        let mut readers: Vec<Box<dyn DynReader>> = std::mem::take(&mut built.streams)
            .into_iter()
            .map(|stream| stream.reader)
            .collect();
        let mut feeder =
            Feeder::new(built.channels, Feeder::DEFAULT_BLOCK).map_err(DecodeError::from)?;
        let pipeline = &mut built.pipeline;
        let mut ended = false;
        loop {
            if let Step::Stop = self.take_signals(false)? {
                return Err(None);
            }
            let item = {
                let capture = self.capture.read();
                feeder.next(&capture, self.position.dropped, self.position.finishing)
            };
            let item = match item {
                Some(item) => Some(item),
                None if self.position.finishing && !ended => {
                    ended = true;
                    Some(SignalItem::End(feeder.fed()))
                }
                None => None,
            };
            match item {
                Some(mut item) => loop {
                    match pipeline.push(source, item) {
                        Ok(()) => break,
                        Err(PushError::Full(back)) => {
                            item = back;
                            pipeline.run();
                            self.drain(&mut readers);
                        }
                        Err(error) => return Err(Some(DecodeError::Pipeline(error.to_string()))),
                    }
                },
                None => {
                    let report = pipeline.run();
                    let moved = self.drain(&mut readers);
                    if ended {
                        if pipeline.is_finished() && readers.iter().all(|reader| reader.is_ended())
                        {
                            // A decoder interpreted from its module that faulted stopped
                            // decoding on its own; what it found before is in its store.
                            if let Some(fault) = built.faults().into_iter().next() {
                                return Err(Some(DecodeError::Pipeline(format!(
                                    "{}: {}",
                                    fault.node, fault.message
                                ))));
                            }
                            return Ok(());
                        }
                        if report.ops == 0 && !moved {
                            return Err(Some(DecodeError::Pipeline(
                                "the pipeline cannot finish: a node waits after the end".into(),
                            )));
                        }
                        continue;
                    }
                    // Nothing to feed: wait to be told of more samples.
                    if let Step::Stop = self.take_signals(true)? {
                        return Err(None);
                    }
                }
            }
        }
    }

    /// Takes every signal waiting, keeping the latest position; with `wait`, waits for one
    /// first. `Stop` on a cancel or when the sender is gone.
    fn take_signals(&mut self, wait: bool) -> Result<Step, Option<DecodeError>> {
        if wait && !self.position.finishing {
            self.report_progress(true);
            match self.signals.recv() {
                Ok(Signal::Position(position)) => self.position = position,
                Ok(Signal::Cancel) | Err(_) => return Ok(Step::Stop),
            }
        }
        loop {
            match self.signals.try_recv() {
                Ok(Signal::Position(position)) => self.position = position,
                Ok(Signal::Cancel) | Err(TryRecvError::Disconnected) => return Ok(Step::Stop),
                Err(TryRecvError::Empty) => return Ok(Step::Go),
            }
        }
    }

    /// Moves what the readers have into the stores and tells the interface, now and then;
    /// whether anything moved.
    fn drain(&mut self, readers: &mut [Box<dyn DynReader>]) -> bool {
        let mut moved = false;
        for (reader, stream) in readers.iter_mut().zip(&self.shared.streams) {
            if reader.pending() == 0 {
                continue;
            }
            let mut store = stream.store.write();
            while let Some(item) = reader.next() {
                moved = true;
                match item {
                    DynItem::Event { span, payload, .. } => store.push_event(span, &payload),
                    DynItem::Gap(range) => store.push_gap(range),
                    DynItem::Progress(tick) => store.push_progress(tick),
                    DynItem::End(tick) => store.push_end(tick),
                }
            }
        }
        if self.position.dropped >= self.pruned_to + PRUNE_STEP {
            self.pruned_to = self.position.dropped;
            for stream in &self.shared.streams {
                stream.store.write().prune_before(Tick(self.pruned_to));
            }
        }
        if moved {
            self.progress_due = true;
            self.report_progress(false);
        }
        moved
    }

    /// Tells the interface of progress that is due: at most every [`PROGRESS_INTERVAL`], or
    /// `now`, before waiting for more samples, so that nothing stays unreported.
    fn report_progress(&mut self, now: bool) {
        if self.progress_due
            && (now
                || self
                    .last_progress
                    .is_none_or(|last| last.elapsed() >= PROGRESS_INTERVAL))
        {
            self.progress_due = false;
            self.last_progress = Some(Instant::now());
            (self.updates)(Update::Progress);
        }
    }
}
