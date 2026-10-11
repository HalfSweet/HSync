// SPDX-License-Identifier: Apache-2.0

//! Nodes with several inputs: two streams merged in time order, a stream read beside the
//! samples it refers to, and the stall a fan-in reports when the source cannot hold what it
//! has to look back on.

use std::sync::Arc;

use serde::Serialize;
use uiharu_core::{
    Change, ChannelMask, EventSchema, Graph, GraphConfig, Inbox, InputRef, InputSpec, Io, Item,
    Levels, LogicBlock, LogicBlockBuilder, Node, NodeFuture, NodeId, NodeSpec, Outbox, OutputRef,
    OutputSpec, Pipeline, Props, Provenance, PushError, Requires, Sample, SchemaId, SignalCursor,
    SignalItem, SourceId, SourceSpec, Tick, TimeBase, ready_all,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
struct Num(u32);

impl EventSchema for Num {
    const SCHEMA: SchemaId = SchemaId::new("test.num", 1);
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
struct Text(String);

impl EventSchema for Text {
    const SCHEMA: SchemaId = SchemaId::new("test.text", 1);
}

/// Counts the changes of one channel, one event per change.
struct Edges {
    channel: u8,
}

impl Edges {
    fn input(id: NodeId) -> InputRef<Arc<LogicBlock>> {
        InputRef::of_node(id, 0)
    }

    fn output(id: NodeId) -> OutputRef<Num> {
        OutputRef::of_node(id, 0)
    }

    async fn run(mut signal: SignalCursor, mut out: Outbox<Num>, channel: u8) {
        let mut count = 0;
        loop {
            match signal.next_change(ChannelMask::one(channel)).await {
                Change::At { tick, .. } => {
                    count += 1;
                    out.emit(Provenance::one(tick), Num(count)).await;
                }
                Change::Quiet(_) => {}
                Change::Gap(range) => out.gap(range).await,
                Change::Progress(tick) => out.progress(tick).await,
                Change::End(tick) => {
                    out.end(tick).await;
                    return;
                }
            }
        }
    }
}

impl Node for Edges {
    fn spec(&self) -> NodeSpec {
        NodeSpec {
            name: "edges".into(),
            inputs: vec![InputSpec::new::<Arc<LogicBlock>>(
                "logic",
                Requires::new().at_least("channels", i64::from(self.channel) + 1),
            )],
            outputs: vec![OutputSpec::new::<Num>("changes", Props::new())],
        }
    }

    fn start(self: Box<Self>, mut io: Io) -> NodeFuture {
        Box::pin(Self::run(io.signal(0), io.output(0), self.channel))
    }
}

/// Reports every two events as one, spanning both: it holds the first until the second comes,
/// and holds progress back to it meanwhile.
struct Pairer;

impl Pairer {
    fn input(id: NodeId) -> InputRef<Num> {
        InputRef::of_node(id, 0)
    }

    fn output(id: NodeId) -> OutputRef<Num> {
        OutputRef::of_node(id, 0)
    }

    async fn run(mut input: Inbox<Num>, mut out: Outbox<Num>) {
        let mut first: Option<(Tick, Tick)> = None;
        loop {
            match input.recv().await {
                Item::Event(event) => match first.take() {
                    None => first = Some((event.span.start, event.settled)),
                    Some((start, _)) => {
                        let span = uiharu_core::TickRange::new(start, event.span.end);
                        out.emit(Provenance::over(span, event.settled), event.payload)
                            .await;
                    }
                },
                Item::Gap(range) => {
                    first = None;
                    out.gap(range).await;
                }
                Item::Progress(tick) => {
                    let held = first.map_or(tick, |(start, _)| start.min(tick));
                    out.progress(held).await;
                }
                Item::End(tick) => {
                    out.end(tick).await;
                    return;
                }
            }
        }
    }
}

impl Node for Pairer {
    fn spec(&self) -> NodeSpec {
        NodeSpec {
            name: "pairer".into(),
            inputs: vec![InputSpec::new::<Num>("changes", Requires::new())],
            outputs: vec![OutputSpec::new::<Num>("pairs", Props::new())],
        }
    }

    fn start(self: Box<Self>, mut io: Io) -> NodeFuture {
        Box::pin(Self::run(io.input(0), io.output(0)))
    }
}

/// Merges two streams in time order: it waits until both have an item, then takes the
/// earlier one, the left on a tie. Gaps and progress are taken from the left stream.
struct TimeMerge;

impl TimeMerge {
    fn left(id: NodeId) -> InputRef<Num> {
        InputRef::of_node(id, 0)
    }

    fn right(id: NodeId) -> InputRef<Num> {
        InputRef::of_node(id, 1)
    }

    fn output(id: NodeId) -> OutputRef<Text> {
        OutputRef::of_node(id, 0)
    }

    async fn run(mut left: Inbox<Num>, mut right: Inbox<Num>, mut out: Outbox<Text>) {
        let (mut left_ended, mut right_ended) = (None, None);
        loop {
            let take_left = match (left_ended, right_ended) {
                (Some(l), Some(r)) => {
                    out.end(Tick::max(l, r)).await;
                    return;
                }
                (None, Some(_)) => true,
                (Some(_), None) => false,
                (None, None) => {
                    ready_all(&[&left, &right]).await;
                    let (l, r) = (left.head_time(), right.head_time());
                    l.expect("the left input is ready") <= r.expect("the right input is ready")
                }
            };
            let (item, side) = if take_left {
                (left.recv().await, "l")
            } else {
                (right.recv().await, "r")
            };
            match item {
                Item::Event(event) => {
                    out.emit(
                        Provenance::over(event.span, event.settled),
                        Text(format!("{side}:{}", event.payload.0)),
                    )
                    .await;
                }
                Item::Gap(range) if take_left => out.gap(range).await,
                Item::Progress(tick) if take_left => out.progress(tick).await,
                Item::Gap(_) | Item::Progress(_) => {}
                Item::End(tick) => {
                    if take_left {
                        left_ended = Some(tick);
                    } else {
                        right_ended = Some(tick);
                    }
                }
            }
        }
    }
}

impl Node for TimeMerge {
    fn spec(&self) -> NodeSpec {
        NodeSpec {
            name: "merge".into(),
            inputs: vec![
                InputSpec::new::<Num>("left", Requires::new()),
                InputSpec::new::<Num>("right", Requires::new()),
            ],
            outputs: vec![OutputSpec::new::<Text>("merged", Props::new())],
        }
    }

    fn start(self: Box<Self>, mut io: Io) -> NodeFuture {
        Box::pin(Self::run(io.input(0), io.input(1), io.output(0)))
    }
}

/// Reads the samples beside a stream: for every event, the level of channel 1 where the
/// event starts. The samples are moved through as the stream tells: a seek at every event
/// and at every progress, so that the blocks behind are let go.
struct LevelAt;

impl LevelAt {
    fn events(id: NodeId) -> InputRef<Num> {
        InputRef::of_node(id, 0)
    }

    fn logic(id: NodeId) -> InputRef<Arc<LogicBlock>> {
        InputRef::of_node(id, 1)
    }

    fn output(id: NodeId) -> OutputRef<Num> {
        OutputRef::of_node(id, 0)
    }

    async fn run(mut events: Inbox<Num>, mut signal: SignalCursor, mut out: Outbox<Num>) {
        loop {
            match events.recv().await {
                Item::Event(event) => {
                    signal.seek(event.span.start);
                    let level = loop {
                        match signal.sample().await {
                            Sample::At { levels, .. } => break u32::from(levels.0 >> 1 & 1 == 1),
                            Sample::Progress(_) => {}
                            Sample::Gap(range) => {
                                signal.seek(range.end);
                            }
                            Sample::End(tick) => {
                                out.end(tick).await;
                                return;
                            }
                        }
                    };
                    out.emit(Provenance::over(event.span, event.settled), Num(level))
                        .await;
                }
                Item::Gap(range) => {
                    signal.seek(range.end);
                    out.gap(range).await;
                }
                Item::Progress(tick) => {
                    signal.seek(tick);
                    out.progress(tick).await;
                }
                Item::End(tick) => {
                    out.end(tick).await;
                    return;
                }
            }
        }
    }
}

impl Node for LevelAt {
    fn spec(&self) -> NodeSpec {
        NodeSpec {
            name: "level-at".into(),
            inputs: vec![
                InputSpec::new::<Num>("events", Requires::new()),
                InputSpec::new::<Arc<LogicBlock>>("logic", Requires::new().at_least("channels", 2)),
            ],
            outputs: vec![OutputSpec::new::<Num>("levels", Props::new())],
        }
    }

    fn start(self: Box<Self>, mut io: Io) -> NodeFuture {
        Box::pin(Self::run(io.input(0), io.signal(1), io.output(0)))
    }
}

/// Two channels toggling with their own periods.
fn toggling(start: u64, len: u64, periods: [u64; 2]) -> Arc<LogicBlock> {
    let mut builder = LogicBlockBuilder::new(Tick(start), 2);
    for i in 0..len {
        let tick = start + i;
        let bit = |channel: usize| u64::from((tick / periods[channel]) % 2 == 1) << channel;
        builder.push(Levels(bit(0) | bit(1)));
    }
    Arc::new(builder.finish().expect("a block"))
}

fn drain<T: Clone>(exposed: &uiharu_core::Exposed<T>) -> Vec<Item<T>> {
    let mut items = Vec::new();
    while let Some(item) = exposed.next() {
        let end = item.is_end();
        items.push(item);
        if end {
            break;
        }
    }
    items
}

/// Feeds `len` samples cut into blocks of `sizes` (cycling), running and reading whenever
/// the source is full; the items read, or the push error that stopped it.
fn drive<T: Clone>(
    pipeline: &mut Pipeline,
    source: SourceId,
    exposed: &uiharu_core::Exposed<T>,
    len: u64,
    sizes: &[u64],
) -> Result<Vec<Item<T>>, PushError> {
    let mut found = Vec::new();
    let mut at = 0;
    let mut cut = 0;
    let push = |pipeline: &mut Pipeline, mut item: SignalItem, found: &mut Vec<Item<T>>| loop {
        match pipeline.push(source, item) {
            Ok(()) => return Ok(()),
            Err(PushError::Full(back)) => {
                item = back;
                pipeline.run();
                found.extend(drain(exposed));
            }
            Err(error) => return Err(error),
        }
    };
    while at < len {
        let size = sizes[cut % sizes.len()].min(len - at);
        cut += 1;
        push(
            pipeline,
            SignalItem::Data(toggling(at, size, [7, 11])),
            &mut found,
        )?;
        at += size;
    }
    push(pipeline, SignalItem::End(Tick(len)), &mut found)?;
    // Small queues take several rounds of running and reading to finish.
    loop {
        let report = pipeline.run();
        let read = drain(exposed);
        let got = read.len();
        found.extend(read);
        if pipeline.is_finished() || (report.ops == 0 && got == 0) {
            break;
        }
    }
    Ok(found)
}

fn events<T: Clone>(items: &[Item<T>]) -> Vec<(u64, uiharu_core::TickRange, Tick, T)> {
    items
        .iter()
        .filter_map(Item::event)
        .map(|event| (event.seq, event.span, event.settled, event.payload.clone()))
        .collect()
}

const CUTS: [&[u64]; 4] = [&[u64::MAX], &[1], &[7, 64, 3], &[63, 1, 64]];

fn merge_graph(config: GraphConfig) -> (Pipeline, SourceId, uiharu_core::ExposedRef<Text>) {
    let mut graph = Graph::with_config(config);
    let source = graph
        .source(SourceSpec::new("logic", 2, TimeBase::new(1_000)))
        .expect("adds the source");
    let left = graph.add(Edges { channel: 0 });
    let right = graph.add(Edges { channel: 1 });
    let merge = graph.add(TimeMerge);
    graph
        .connect(source.samples, Edges::input(left))
        .expect("connects");
    graph
        .connect(source.samples, Edges::input(right))
        .expect("connects");
    graph
        .connect(Edges::output(left), TimeMerge::left(merge))
        .expect("connects");
    graph
        .connect(Edges::output(right), TimeMerge::right(merge))
        .expect("connects");
    let exposed = graph
        .expose("merged", TimeMerge::output(merge))
        .expect("exposes");
    (graph.build().expect("builds"), source.id, exposed)
}

#[test]
fn two_streams_merge_in_time_order_whatever_the_blocks_and_queues() {
    let mut reference = None;
    for capacity in [256, 3, 1] {
        for cut in CUTS {
            let config = GraphConfig {
                queue_capacity: capacity,
                budget: 16,
            };
            let (mut pipeline, source, exposed) = merge_graph(config);
            let merged = pipeline.exposed(exposed).expect("the exposed reader");
            let found = events(
                &drive(&mut pipeline, source, &merged, 1_000, cut).expect("drives to the end"),
            );
            match &reference {
                None => {
                    // Channel 0 changes every 7 ticks, channel 1 every 11; at 77 and its
                    // multiples both change, and the left comes first.
                    assert_eq!(found.len(), 142 + 90);
                    let mut last = Tick::ZERO;
                    for (index, (_, span, _, Text(text))) in found.iter().enumerate() {
                        assert!(span.start >= last, "{text} at {span} after {last}");
                        last = span.start;
                        if span.start.0 % 77 == 0 && text.starts_with("r:") {
                            let before = &found[index - 1];
                            assert!(
                                before.1.start == span.start && before.3.0.starts_with("l:"),
                                "{text} at {span} after {before:?}"
                            );
                        }
                    }
                    reference = Some(found);
                }
                Some(reference) => {
                    assert_eq!(&found, reference, "capacity {capacity}, blocks of {cut:?}");
                }
            }
        }
    }
}

fn level_graph(
    pairs: bool,
    source_capacity: usize,
) -> (Pipeline, SourceId, uiharu_core::ExposedRef<Num>) {
    let mut graph = Graph::new();
    let source = graph
        .source(SourceSpec::new("logic", 2, TimeBase::new(1_000)).with_capacity(source_capacity))
        .expect("adds the source");
    let edges = graph.add(Edges { channel: 0 });
    let level = graph.add(LevelAt);
    graph
        .connect(source.samples, Edges::input(edges))
        .expect("connects");
    let events = if pairs {
        let pairer = graph.add(Pairer);
        graph
            .connect(Edges::output(edges), Pairer::input(pairer))
            .expect("connects");
        Pairer::output(pairer)
    } else {
        Edges::output(edges)
    };
    graph
        .connect(events, LevelAt::events(level))
        .expect("connects");
    graph
        .connect(source.samples, LevelAt::logic(level))
        .expect("connects");
    let exposed = graph
        .expose("levels", LevelAt::output(level))
        .expect("exposes");
    (graph.build().expect("builds"), source.id, exposed)
}

#[test]
fn a_node_reads_the_samples_beside_a_stream() {
    let mut reference = None;
    for cut in CUTS {
        let (mut pipeline, source, exposed) = level_graph(false, 4);
        let levels = pipeline.exposed(exposed).expect("the exposed reader");
        let found =
            events(&drive(&mut pipeline, source, &levels, 1_000, cut).expect("drives to the end"));
        match &reference {
            None => {
                assert_eq!(found.len(), 142);
                for (_, span, _, Num(level)) in &found {
                    // Channel 0 changes at multiples of 7; channel 1 is high in every other
                    // period of 11.
                    assert_eq!(span.start.0 % 7, 0);
                    assert_eq!(*level, u32::from((span.start.0 / 11) % 2 == 1), "at {span}");
                }
                reference = Some(found);
            }
            Some(reference) => assert_eq!(&found, reference, "blocks of {cut:?}"),
        }
    }
}

#[test]
fn a_fan_in_stalls_when_the_source_cannot_hold_what_it_looks_back_on() {
    // The pairer holds every first change until the second, seven ticks later, so the
    // level reader must keep the samples from there on: a source holding two items (the
    // progress it starts with and one block of one sample) cannot feed the next block until
    // that one is let go, and nothing moves.
    let (mut pipeline, source, exposed) = level_graph(true, 2);
    let levels = pipeline.exposed(exposed).expect("the exposed reader");
    match drive(&mut pipeline, source, &levels, 200, &[1]) {
        Err(PushError::Stalled { waiting }) => {
            assert_eq!(waiting.len(), 3, "{waiting:?}");
            assert!(
                waiting
                    .iter()
                    .all(|(_, wait)| *wait == uiharu_core::Wait::Input),
                "{waiting:?}"
            );
        }
        other => panic!("expected a stall, got {other:?}"),
    }
    // Enough blocks held, the same run completes, and like one uninterrupted block.
    let whole = {
        let (mut pipeline, source, exposed) = level_graph(true, 4);
        let levels = pipeline.exposed(exposed).expect("the exposed reader");
        events(&drive(&mut pipeline, source, &levels, 200, &[u64::MAX]).expect("one block"))
    };
    assert_eq!(whole.len(), 14);
    let (mut pipeline, source, exposed) = level_graph(true, 16);
    let levels = pipeline.exposed(exposed).expect("the exposed reader");
    let found =
        events(&drive(&mut pipeline, source, &levels, 200, &[1]).expect("drives to the end"));
    assert_eq!(found, whole);
}
