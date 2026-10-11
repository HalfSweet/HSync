// SPDX-License-Identifier: Apache-2.0

//! Helpers for testing decoders: a one-source pipeline driven block by block, with the blocks
//! cut to chosen sizes, gaps announced and the reader pausing between blocks, and the check
//! the acceptance criteria call for — that none of this changes the events.
//!
//! Enabled by the `testing` feature, for the dev-dependencies of protocol crates:
//!
//! ```toml
//! [dev-dependencies]
//! uiharu-core = { path = "../uiharu-core", features = ["testing"] }
//! ```

use std::cell::RefCell;
use std::fmt::Debug;
use std::rc::Rc;
use std::sync::Arc;

use crate::exec::{Exposed, Pipeline, PushError, RunReport};
use crate::graph::{
    ExposedRef, Graph, GraphConfig, GraphError, OutputRef, SourceHandle, SourceId, SourceSpec,
};
use crate::schema::EventSchema;
use crate::signal::{LogicBlock, SignalItem};
use crate::stream::{Event, Item};
use crate::time::{Tick, TickRange, TimeBase};

/// Reads up to `count` items of `exposed` into `into`; the end of the stream is read once and
/// ends the reading.
pub fn drain_some<T: Clone>(exposed: &Exposed<T>, into: &mut Vec<Item<T>>, count: usize) {
    for _ in 0..count {
        if into.last().is_some_and(Item::is_end) {
            return;
        }
        let Some(item) = exposed.next() else { return };
        let end = item.is_end();
        into.push(item);
        if end {
            return;
        }
    }
}

/// The items that are not progress; progress depends on how the samples were cut.
pub fn without_progress<T: Clone>(items: &[Item<T>]) -> Vec<Item<T>> {
    items
        .iter()
        .filter(|item| !matches!(item, Item::Progress(_)))
        .cloned()
        .collect()
}

/// The events among `items`.
pub fn events<T: Clone>(items: &[Item<T>]) -> Vec<Event<T>> {
    items.iter().filter_map(Item::event).cloned().collect()
}

trait Drainer {
    fn drain_some(&mut self, count: usize);
}

struct TapDrainer<U> {
    exposed: Exposed<U>,
    items: Rc<RefCell<Vec<Item<U>>>>,
}

impl<U: Clone> Drainer for TapDrainer<U> {
    fn drain_some(&mut self, count: usize) {
        drain_some(&self.exposed, &mut self.items.borrow_mut(), count);
    }
}

/// Another exposed stream a [`Harness`] reads along with its output; see [`Harness::tap`].
#[derive(Debug)]
pub struct Tap<U> {
    items: Rc<RefCell<Vec<Item<U>>>>,
}

impl<U: Clone> Tap<U> {
    /// Everything read so far.
    pub fn items(&self) -> Vec<Item<U>> {
        self.items.borrow().clone()
    }

    /// Everything read so far but progress.
    pub fn without_progress(&self) -> Vec<Item<U>> {
        without_progress(&self.items.borrow())
    }

    /// The events read so far.
    pub fn events(&self) -> Vec<Event<U>> {
        events(&self.items.borrow())
    }
}

/// A one-source pipeline with one exposed output, driven block by block.
pub struct Harness<T> {
    pipeline: Pipeline,
    source: SourceId,
    output: Exposed<T>,
    taps: Vec<Box<dyn Drainer>>,
    /// Everything read from the output so far.
    pub items: Vec<Item<T>>,
    /// How many items [`Harness::push`] reads from each stream when the source is full; at
    /// least one.
    pub drain_on_full: usize,
}

impl<T> Debug for Harness<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Harness")
            .field("items", &self.items.len())
            .finish_non_exhaustive()
    }
}

impl<T: Clone + 'static> Harness<T> {
    /// Builds `source → …`, with `wire` adding the nodes to the graph and returning the output
    /// to read; the source has `channels` channels on `time_base`.
    pub fn build(
        channels: u8,
        time_base: TimeBase,
        config: GraphConfig,
        wire: impl FnOnce(&mut Graph, SourceHandle) -> Result<OutputRef<T>, GraphError>,
    ) -> Result<Self, GraphError>
    where
        T: EventSchema,
    {
        let mut graph = Graph::with_config(config);
        let source = graph.source(SourceSpec::new("logic", channels, time_base))?;
        let output = wire(&mut graph, source)?;
        let exposed = graph.expose("harness.output", output)?;
        let mut pipeline = graph.build()?;
        let output = pipeline
            .exposed(exposed)
            .unwrap_or_else(|| unreachable!("exposed once"));
        Ok(Self {
            pipeline,
            source: source.id,
            output,
            taps: Vec::new(),
            items: Vec::new(),
            drain_on_full: usize::MAX,
        })
    }

    /// Also reads the stream exposed as `exposed` (by `wire`, through [`Graph::expose`])
    /// whenever the output is read.
    pub fn tap<U: Clone + 'static>(&mut self, exposed: ExposedRef<U>) -> Tap<U> {
        let reader = self
            .pipeline
            .exposed(exposed)
            .expect("an exposure is read once");
        let items = Rc::new(RefCell::new(Vec::new()));
        self.taps.push(Box::new(TapDrainer {
            exposed: reader,
            items: Rc::clone(&items),
        }));
        Tap { items }
    }

    /// The pipeline.
    pub fn pipeline(&self) -> &Pipeline {
        &self.pipeline
    }

    /// The pipeline, to push into or run by hand.
    pub fn pipeline_mut(&mut self) -> &mut Pipeline {
        &mut self.pipeline
    }

    /// The source.
    pub fn source(&self) -> SourceId {
        self.source
    }

    /// Runs the pipeline without reading.
    pub fn run(&mut self) -> RunReport {
        self.pipeline.run()
    }

    /// Runs the pipeline and reads everything the output and the taps hold.
    pub fn collect(&mut self) {
        self.collect_some(usize::MAX);
    }

    /// Runs the pipeline and reads up to `count` items from the output and from each tap.
    pub fn collect_some(&mut self, count: usize) {
        self.pipeline.run();
        drain_some(&self.output, &mut self.items, count);
        for tap in &mut self.taps {
            tap.drain_some(count);
        }
    }

    /// Pushes `item`, reading [`Harness::drain_on_full`] items first whenever the source is
    /// full.
    ///
    /// # Panics
    ///
    /// When the push fails for another reason: a block out of order or with the wrong
    /// channels, or a source that has ended.
    pub fn push(&mut self, item: SignalItem) {
        let mut item = item;
        loop {
            match self.pipeline.push(self.source, item) {
                Ok(()) => return,
                Err(PushError::Full(back)) => {
                    item = back;
                    let count = self.drain_on_full.max(1);
                    self.collect_some(count);
                }
                Err(other) => panic!("push failed: {other}"),
            }
        }
    }

    /// Feeds `block` cut into blocks whose sizes cycle through `sizes`, leaving out `gaps`
    /// (announced as such) and, after each block, running the pipeline and reading as many
    /// items as the next entry of `pauses` says — none at all between blocks when `pauses` is
    /// empty.
    pub fn feed_cut(
        &mut self,
        block: &LogicBlock,
        gaps: &[TickRange],
        sizes: &[u64],
        pauses: &[usize],
    ) {
        let mut gaps: Vec<TickRange> = gaps.iter().copied().filter(|gap| !gap.is_empty()).collect();
        gaps.sort_by_key(|gap| gap.start);
        let mut at = block.start();
        let mut cut = 0;
        let mut pause = 0;
        let mut gaps = gaps.into_iter().peekable();
        while at < block.end() {
            if let Some(gap) = gaps.peek().copied()
                && gap.start <= at
            {
                gaps.next();
                let end = gap.end.min(block.end()).max(at);
                if end > at {
                    self.push(SignalItem::Gap(TickRange::new(at, end)));
                    at = end;
                }
                continue;
            }
            let mut end = at.offset(
                sizes
                    .get(cut % sizes.len().max(1))
                    .copied()
                    .unwrap_or(u64::MAX)
                    .max(1),
            );
            end = end.min(block.end());
            if let Some(gap) = gaps.peek() {
                end = end.min(gap.start.max(at.next()));
            }
            cut += 1;
            let piece = block
                .slice(TickRange::new(at, end))
                .expect("a slice of the block");
            self.push(SignalItem::Data(Arc::new(piece)));
            if !pauses.is_empty() {
                let count = pauses[pause % pauses.len()];
                pause += 1;
                self.collect_some(count);
            }
            at = end;
        }
    }

    /// Feeds `block` in blocks whose sizes cycle through `sizes`, without reading in between.
    pub fn feed(&mut self, block: &LogicBlock, sizes: &[u64]) {
        self.feed_cut(block, &[], sizes, &[]);
    }

    /// Feeds `block` leaving out `gaps`, which are announced as such.
    pub fn feed_with_gaps(&mut self, block: &LogicBlock, gaps: &[TickRange], sizes: &[u64]) {
        self.feed_cut(block, gaps, sizes, &[]);
    }

    /// Ends the samples where they stand and runs to the end, reading as the nodes write.
    ///
    /// # Panics
    ///
    /// When the pipeline does not finish.
    pub fn end(&mut self) {
        let at = self.pipeline.next_tick(self.source).unwrap_or(Tick::ZERO);
        self.push(SignalItem::End(at));
        for _ in 0..1_000_000 {
            self.collect();
            if self.pipeline.is_finished() {
                return;
            }
        }
        panic!("the pipeline did not finish: {:?}", self.pipeline.run());
    }

    /// Feeds the whole block in one piece and ends.
    pub fn run_all(&mut self, block: &LogicBlock) {
        self.push(SignalItem::Data(Arc::new(block.clone())));
        self.end();
    }

    /// The events read so far.
    pub fn events(&self) -> Vec<Event<T>> {
        events(&self.items)
    }

    /// Everything read so far but progress.
    pub fn without_progress(&self) -> Vec<Item<T>> {
        without_progress(&self.items)
    }
}

/// The block sizes [`assert_block_invariant`] tries.
pub const CUTS: [&[u64]; 6] = [
    &[u64::MAX],
    &[1],
    &[2, 3, 5, 7],
    &[64],
    &[63, 65, 1],
    &[1000, 1],
];

/// The reading patterns [`assert_block_invariant`] tries: items read after each block.
pub const PAUSES: [&[usize]; 3] = [&[0], &[1], &[3, 0, 7]];

/// The queue sizes and budgets [`assert_block_invariant`] tries.
pub const CONFIGS: [GraphConfig; 3] = [
    GraphConfig {
        queue_capacity: 256,
        budget: 1024,
    },
    GraphConfig {
        queue_capacity: 1,
        budget: 1,
    },
    GraphConfig {
        queue_capacity: 3,
        budget: 7,
    },
];

/// Runs `block`, less `gaps`, through fresh harnesses cut into blocks of every size in
/// [`CUTS`], read with every pattern in [`PAUSES`], through every configuration in
/// [`CONFIGS`], and checks that each run yields the events of one uninterrupted run, progress
/// aside. `build` makes a harness for a configuration.
///
/// # Panics
///
/// When a run differs from the uninterrupted one.
pub fn assert_block_invariant<T>(
    build: impl Fn(GraphConfig) -> Harness<T>,
    block: &LogicBlock,
    gaps: &[TickRange],
) where
    T: Clone + PartialEq + Debug + 'static,
{
    let reference = {
        let mut harness = build(GraphConfig::default());
        harness.feed_with_gaps(block, gaps, &[u64::MAX]);
        harness.end();
        harness.without_progress()
    };
    for config in CONFIGS {
        for cut in CUTS {
            for pauses in PAUSES {
                let mut harness = build(config);
                harness.drain_on_full = 1;
                harness.feed_cut(block, gaps, cut, pauses);
                harness.end();
                assert_eq!(
                    harness.without_progress(),
                    reference,
                    "blocks of {cut:?}, reading {pauses:?} after each, {config:?}"
                );
            }
        }
    }
}
