// SPDX-License-Identifier: Apache-2.0

//! Running a built graph: feeding sources, polling the nodes until nothing can move, and
//! reading the exposed outputs.
//!
//! Everything runs on the caller's thread, in a fixed order, so the same input through the
//! same graph always yields the same events, however it is cut into blocks and however often
//! the caller stops to read the outputs.

use std::any::Any;
use std::cell::RefCell;
use std::fmt;
use std::rc::Rc;
use std::sync::Arc;
use std::task::{Context, Waker};

use crate::graph::{ExposedRef, Exposure, GraphConfig, Owner, PortKey, SourceId, SourceSpec};
use crate::node::{Io, Node, NodeCx, NodeFuture, NodeSpec, SharedQueue, Wait};
use crate::schema::Props;
use crate::signal::{LogicBlock, SignalItem};
use crate::stream::{Broadcast, Event, Item, QueueControl, QueueHandles, ReaderId};
use crate::time::{Tick, TimeBase};

/// Why a source did not take an item.
#[derive(Debug, thiserror::Error)]
pub enum PushError {
    /// The source holds as many blocks as it may; run the pipeline and read its outputs, then
    /// push the item again. If the samples cannot wait, push a gap for them instead.
    #[error("the source is full")]
    Full(SignalItem),
    /// The item does not follow the previous one; lost samples must be announced as a gap.
    #[error("the item starts at tick {found} but the source stands at {expected}")]
    Discontinuity {
        /// The tick the source expects next.
        expected: Tick,
        /// The tick the item starts at.
        found: Tick,
    },
    /// The block has another channel count than the source.
    #[error("the block has {found} channels but the source has {expected}")]
    Channels {
        /// The source's channels.
        expected: u8,
        /// The block's channels.
        found: u8,
    },
    /// The source has ended.
    #[error("the source has ended")]
    Ended,
    /// No such source.
    #[error("no source {0} in the pipeline")]
    UnknownSource(usize),
}

struct SourceState {
    name: String,
    channels: u8,
    time_base: TimeBase,
    queue: SharedQueue<Arc<LogicBlock>>,
    handles: QueueHandles,
    /// The tick the next item starts at; the first item decides where the stream starts.
    next: Option<Tick>,
    ended: bool,
}

struct Running {
    name: String,
    cx: Rc<NodeCx>,
    /// `None` once the node has finished.
    future: Option<NodeFuture>,
}

struct QueueEntry {
    name: String,
    control: Rc<dyn QueueControl>,
}

struct ExposedEntry {
    name: String,
    queue: Rc<dyn Any>,
    /// The reader, until the caller takes it.
    reader: Option<ReaderId>,
}

/// What one run of the pipeline did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RunReport {
    /// Operations the nodes completed.
    pub ops: u64,
    /// Times a node was polled.
    pub polls: u64,
    /// Passes over the nodes.
    pub rounds: u32,
    /// What each unfinished node waits for now, by name.
    pub waiting: Vec<(String, Wait)>,
}

/// The state of one queue.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QueueStat {
    /// The queue, as `owner.port`.
    pub name: String,
    /// Items held.
    pub len: usize,
    /// Items it holds at most.
    pub capacity: usize,
    /// The most it held at any time.
    pub high_water: usize,
    /// Whether its stream has ended.
    pub ended: bool,
}

/// The state of every queue of a pipeline.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    /// The source queues, then the node outputs in node order.
    pub queues: Vec<QueueStat>,
}

impl Stats {
    /// The most items any queue held at one time.
    pub fn high_water(&self) -> usize {
        self.queues
            .iter()
            .map(|queue| queue.high_water)
            .max()
            .unwrap_or(0)
    }

    /// Whether every queue stayed within its capacity (which the queues enforce; this checks
    /// the bookkeeping).
    pub fn within_capacity(&self) -> bool {
        self.queues
            .iter()
            .all(|queue| queue.high_water <= queue.capacity)
    }
}

/// A built graph, ready to take samples and run.
pub struct Pipeline {
    budget: u32,
    sources: Vec<SourceState>,
    /// In an order where every node comes after the ones it reads.
    nodes: Vec<Running>,
    queues: Vec<QueueEntry>,
    exposed: Vec<ExposedEntry>,
}

impl fmt::Debug for Pipeline {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Pipeline")
            .field("sources", &self.sources.len())
            .field("nodes", &self.nodes.len())
            .field("exposed", &self.exposed.len())
            .finish()
    }
}

impl Pipeline {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn start(
        config: GraphConfig,
        sources: Vec<SourceSpec>,
        nodes: Vec<Box<dyn Node>>,
        specs: Vec<NodeSpec>,
        inputs: Vec<Vec<Option<PortKey>>>,
        order: Vec<usize>,
        domains: Vec<Option<usize>>,
        input_props: Vec<Vec<Props>>,
        exposed: Vec<Exposure>,
    ) -> Result<Self, crate::graph::GraphError> {
        let mut queues = Vec::new();
        let sources: Vec<SourceState> = sources
            .into_iter()
            .enumerate()
            .map(|(index, spec)| {
                let handles = spec.output().make_queue(spec.capacity);
                queues.push(QueueEntry {
                    name: format!("{}#{index}.samples", spec.name),
                    control: Rc::clone(&handles.control),
                });
                let queue = Rc::clone(&handles.any)
                    .downcast::<RefCell<Broadcast<Arc<LogicBlock>>>>()
                    .unwrap_or_else(|_| unreachable!("source queues carry blocks"));
                SourceState {
                    name: spec.name,
                    channels: spec.channels,
                    time_base: spec.time_base,
                    queue,
                    handles,
                    next: None,
                    ended: false,
                }
            })
            .collect();

        // Every output queue first, so that inputs can be bound to them in any order.
        let outputs: Vec<Vec<QueueHandles>> = specs
            .iter()
            .enumerate()
            .map(|(index, spec)| {
                spec.outputs
                    .iter()
                    .map(|output| {
                        let handles = output.make_queue(config.queue_capacity);
                        queues.push(QueueEntry {
                            name: format!("{}#{index}.{}", spec.name, output.name),
                            control: Rc::clone(&handles.control),
                        });
                        handles
                    })
                    .collect()
            })
            .collect();
        let queue_of = |key: PortKey| -> QueueHandles {
            match key.owner {
                Owner::Source(index) => sources[index].handles.clone(),
                Owner::Node(index) => outputs[index][key.index].clone(),
            }
        };

        // Exposed readers attach before any node runs, so they see every item.
        let exposed = exposed
            .into_iter()
            .map(|exposure| {
                let handles = queue_of(exposure.from);
                let reader = handles.control.add_reader();
                ExposedEntry {
                    name: exposure.name,
                    queue: handles.any,
                    reader: Some(reader),
                }
            })
            .collect();

        let mut running: Vec<Option<Running>> = (0..nodes.len()).map(|_| None).collect();
        let mut specs = specs.into_iter().map(Some).collect::<Vec<_>>();
        let mut nodes = nodes.into_iter().map(Some).collect::<Vec<_>>();
        for &index in &order {
            let (Some(node), Some(spec)) = (nodes[index].take(), specs[index].take()) else {
                continue;
            };
            let cx = NodeCx::new();
            let bound_inputs = inputs[index]
                .iter()
                .flatten()
                .map(|key| queue_of(*key).any)
                .collect();
            let bound_outputs = outputs[index]
                .iter()
                .map(|handles| Rc::clone(&handles.any))
                .collect();
            let time_base = domains[index].map(|domain| sources[domain].time_base);
            let io = Io::new(
                Rc::clone(&cx),
                bound_inputs,
                bound_outputs,
                time_base,
                input_props[index].clone(),
            );
            let future = node.start(io);
            running[index] = Some(Running {
                name: format!("{}#{index}", spec.name),
                cx,
                future: Some(future),
            });
        }
        let nodes = order
            .into_iter()
            .filter_map(|index| running[index].take())
            .collect();
        Ok(Self {
            budget: config.budget,
            sources,
            nodes,
            queues,
            exposed,
        })
    }

    /// The names and time bases of the sources, in id order.
    pub fn sources(&self) -> impl Iterator<Item = (SourceId, &str, TimeBase)> {
        self.sources
            .iter()
            .enumerate()
            .map(|(index, source)| (SourceId(index), source.name.as_str(), source.time_base))
    }

    /// Feeds `item` to `source`. The first item decides where the stream starts; after it,
    /// blocks must follow each other or a gap without a hole. The item is handed back when
    /// the source is full.
    pub fn push(&mut self, source: SourceId, item: SignalItem) -> Result<(), PushError> {
        let state = self
            .sources
            .get_mut(source.0)
            .ok_or(PushError::UnknownSource(source.0))?;
        if state.ended {
            return Err(PushError::Ended);
        }
        let expected = state.next.unwrap_or(match &item {
            SignalItem::Data(block) => block.start(),
            SignalItem::Gap(range) => range.start,
            SignalItem::Progress(tick) | SignalItem::End(tick) => *tick,
        });
        let found = match &item {
            SignalItem::Data(block) => {
                if block.channels() != state.channels {
                    return Err(PushError::Channels {
                        expected: state.channels,
                        found: block.channels(),
                    });
                }
                block.start()
            }
            SignalItem::Gap(range) => range.start,
            SignalItem::End(tick) => *tick,
            SignalItem::Progress(tick) => {
                if *tick < expected {
                    return Err(PushError::Discontinuity {
                        expected,
                        found: *tick,
                    });
                }
                expected
            }
        };
        if found != expected {
            return Err(PushError::Discontinuity { expected, found });
        }
        let (queued, next, ended) = match item {
            SignalItem::Data(block) => {
                let end = block.end();
                let last = Tick(end.0 - 1);
                let event = Event {
                    seq: 0,
                    span: block.range(),
                    settled: last,
                    payload: block,
                };
                (Item::Event(event), end, false)
            }
            SignalItem::Gap(range) => (Item::Gap(range), range.end.max(expected), false),
            SignalItem::Progress(tick) => (Item::Progress(tick), expected, false),
            SignalItem::End(tick) => (Item::End(tick), tick, true),
        };
        if let Err(item) = state.queue.borrow_mut().try_push(queued) {
            let item = match item {
                Item::Event(event) => SignalItem::Data(event.payload),
                Item::Gap(range) => SignalItem::Gap(range),
                Item::Progress(tick) => SignalItem::Progress(tick),
                Item::End(tick) => SignalItem::End(tick),
            };
            return Err(PushError::Full(item));
        }
        state.next = Some(next);
        state.ended = ended;
        Ok(())
    }

    /// The tick `source` expects next; `None` before its first item, which may start
    /// anywhere.
    pub fn next_tick(&self, source: SourceId) -> Option<Tick> {
        self.sources.get(source.0).and_then(|state| state.next)
    }

    /// Whether `source` has room for another item.
    pub fn has_room(&self, source: SourceId) -> bool {
        self.sources
            .get(source.0)
            .is_some_and(|state| !state.queue.borrow().is_full())
    }

    /// Polls the nodes, in order, until none of them can go on: every unfinished node waits
    /// for input that has not arrived or for an output the caller has not read.
    pub fn run(&mut self) -> RunReport {
        let mut report = RunReport::default();
        let mut context = Context::from_waker(Waker::noop());
        loop {
            report.rounds += 1;
            let mut moved = false;
            for node in &mut self.nodes {
                let Some(future) = node.future.as_mut() else {
                    continue;
                };
                node.cx.refill(self.budget);
                let before = node.cx.ops();
                report.polls += 1;
                let poll = future.as_mut().poll(&mut context);
                let done = node.cx.ops() - before;
                report.ops += done;
                moved |= done > 0;
                // A node that only ran out of budget goes on in the next round, after the
                // others have had their turn.
                if poll.is_ready() {
                    node.future = None;
                }
            }
            if !moved {
                break;
            }
        }
        report.waiting = self
            .nodes
            .iter()
            .filter(|node| node.future.is_some())
            .map(|node| (node.name.clone(), node.cx.waiting()))
            .collect();
        report
    }

    /// Whether every node has finished: their inputs ended and they wrote all they had.
    pub fn is_finished(&self) -> bool {
        self.nodes.iter().all(|node| node.future.is_none())
    }

    /// The state of every queue.
    pub fn stats(&self) -> Stats {
        Stats {
            queues: self
                .queues
                .iter()
                .map(|entry| QueueStat {
                    name: entry.name.clone(),
                    len: entry.control.len(),
                    capacity: entry.control.capacity(),
                    high_water: entry.control.high_water(),
                    ended: entry.control.is_ended(),
                })
                .collect(),
        }
    }

    /// Takes the reader of an exposed output; `None` once taken.
    pub fn exposed<T: 'static>(&mut self, exposed: ExposedRef<T>) -> Option<Exposed<T>> {
        let entry = self.exposed.get_mut(exposed.index)?;
        let reader = entry.reader.take()?;
        let queue = Rc::clone(&entry.queue)
            .downcast::<RefCell<Broadcast<T>>>()
            .unwrap_or_else(|_| panic!("exposed output {} carries another event type", entry.name));
        Some(Exposed {
            name: entry.name.clone(),
            queue,
            reader,
        })
    }
}

/// The caller's reader on an exposed output.
pub struct Exposed<T> {
    name: String,
    queue: SharedQueue<T>,
    reader: ReaderId,
}

impl<T> fmt::Debug for Exposed<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Exposed")
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}

impl<T> Exposed<T> {
    /// The name the output was exposed under.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Looks at the next item without taking it.
    pub fn with_next<R>(&self, f: impl FnOnce(&Item<T>) -> R) -> Option<R> {
        self.queue.borrow().peek(self.reader).map(f)
    }

    /// Moves past the next item; the end of the stream stays.
    pub fn advance(&self) {
        self.queue.borrow_mut().advance(self.reader);
    }

    /// Takes the next item, if one is waiting. The end of the stream is returned again and
    /// again; use [`Exposed::is_ended`] to stop.
    pub fn next(&self) -> Option<Item<T>>
    where
        T: Clone,
    {
        let item = self.with_next(Item::clone)?;
        self.advance();
        Some(item)
    }

    /// Whether the next item is the end of the stream.
    pub fn is_ended(&self) -> bool {
        self.with_next(Item::is_end).unwrap_or(false)
    }

    /// The items waiting to be read.
    pub fn pending(&self) -> usize {
        self.queue.borrow().pending(self.reader)
    }
}

impl<T> Drop for Exposed<T> {
    fn drop(&mut self) {
        self.queue.borrow_mut().remove_reader(self.reader);
    }
}
