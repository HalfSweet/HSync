// SPDX-License-Identifier: Apache-2.0

//! The executor through its public API: small nodes wired into graphs, fed in blocks, run and
//! read back.

use std::sync::Arc;

use serde::Serialize;
use uiharu_core::{
    Change, EventSchema, Graph, GraphConfig, GraphError, Inbox, InputRef, InputSpec, Io, Item,
    Levels, LogicBlock, LogicBlockBuilder, Node, NodeFuture, NodeId, NodeSpec, Outbox, OutputRef,
    OutputSpec, Pipeline, Props, Provenance, PushError, Requires, SchemaId, SignalCursor,
    SignalItem, SourceSpec, Tick, TickRange, TimeBase, Wait,
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

/// Numbers the changes of one channel: the n-th change is the event `Num(n)` at its tick.
/// Gaps, progress and the end are passed on.
struct EdgeCounter {
    channel: u8,
}

impl EdgeCounter {
    fn input(id: NodeId) -> InputRef<Arc<LogicBlock>> {
        InputRef::of_node(id, 0)
    }

    fn output(id: NodeId) -> OutputRef<Num> {
        OutputRef::of_node(id, 0)
    }

    async fn run(mut signal: SignalCursor, mut out: Outbox<Num>, channel: u8) {
        let mut count = 0;
        loop {
            match signal
                .next_change(uiharu_core::ChannelMask::one(channel))
                .await
            {
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

impl Node for EdgeCounter {
    fn spec(&self) -> NodeSpec {
        NodeSpec {
            name: "edges".into(),
            inputs: vec![InputSpec::new::<Arc<LogicBlock>>(
                "logic",
                Requires::new().at_least("channels", i64::from(self.channel) + 1),
            )],
            outputs: vec![OutputSpec::new::<Num>(
                "count",
                Props::new().with("channel", i64::from(self.channel)),
            )],
        }
    }

    fn start(self: Box<Self>, mut io: Io) -> NodeFuture {
        Box::pin(Self::run(io.signal(0), io.output(0), self.channel))
    }
}

/// Doubles the numbers it reads and passes everything else on.
struct Doubler {
    requires: Requires,
}

impl Doubler {
    fn new() -> Self {
        Self {
            requires: Requires::new(),
        }
    }

    fn input(id: NodeId) -> InputRef<Num> {
        InputRef::of_node(id, 0)
    }

    fn output(id: NodeId) -> OutputRef<Num> {
        OutputRef::of_node(id, 0)
    }

    async fn run(mut input: Inbox<Num>, mut out: Outbox<Num>) {
        loop {
            match input.recv().await {
                Item::Event(event) => {
                    out.emit(
                        Provenance::over(event.span, event.settled),
                        Num(event.payload.0 * 2),
                    )
                    .await;
                }
                Item::Gap(range) => out.gap(range).await,
                Item::Progress(tick) => out.progress(tick).await,
                Item::End(tick) => {
                    out.end(tick).await;
                    return;
                }
            }
        }
    }
}

impl Node for Doubler {
    fn spec(&self) -> NodeSpec {
        NodeSpec {
            name: "doubler".into(),
            inputs: vec![InputSpec::new::<Num>("numbers", self.requires.clone())],
            outputs: vec![OutputSpec::new::<Num>(
                "doubled",
                Props::new().with("doubled", true),
            )],
        }
    }

    fn start(self: Box<Self>, mut io: Io) -> NodeFuture {
        Box::pin(Self::run(io.input(0), io.output(0)))
    }
}

/// Reads two number streams; only here to have a node with two inputs.
struct Merger;

impl Node for Merger {
    fn spec(&self) -> NodeSpec {
        NodeSpec {
            name: "merger".into(),
            inputs: vec![
                InputSpec::new::<Num>("left", Requires::new()),
                InputSpec::new::<Num>("right", Requires::new()),
            ],
            outputs: vec![OutputSpec::new::<Text>("text", Props::new())],
        }
    }

    fn start(self: Box<Self>, mut io: Io) -> NodeFuture {
        let _left: Inbox<Num> = io.input(0);
        let _right: Inbox<Num> = io.input(1);
        let _out: Outbox<Text> = io.output(0);
        Box::pin(async {})
    }
}

/// Reports what the executor tells it: the sample rate and the channel count it reads.
struct Introspect;

impl Node for Introspect {
    fn spec(&self) -> NodeSpec {
        NodeSpec {
            name: "introspect".into(),
            inputs: vec![InputSpec::new::<Arc<LogicBlock>>("logic", Requires::new())],
            outputs: vec![OutputSpec::new::<Num>("facts", Props::new())],
        }
    }

    fn start(self: Box<Self>, mut io: Io) -> NodeFuture {
        let rate = io.time_base().map_or(0, |base| base.rate_hz) as u32;
        let channels = match io.input_props(0).get("channels") {
            Some(uiharu_core::PropValue::Int(n)) => *n as u32,
            _ => 0,
        };
        let _signal = io.signal(0);
        let mut out: Outbox<Num> = io.output(0);
        Box::pin(async move {
            out.emit(Provenance::at(Tick::ZERO), Num(rate)).await;
            out.emit(Provenance::at(Tick::ZERO), Num(channels)).await;
            out.end(Tick::ZERO).await;
        })
    }
}

#[test]
fn nodes_learn_their_time_base_and_what_they_read() {
    let mut graph = Graph::new();
    let source = graph
        .source(SourceSpec::new("logic", 3, TimeBase::new(25_000_000)))
        .expect("adds the source");
    let node = graph.add(Introspect);
    graph
        .connect(
            source.samples,
            InputRef::<Arc<LogicBlock>>::of_node(node, 0),
        )
        .expect("connects");
    let facts = graph
        .expose("facts", OutputRef::<Num>::of_node(node, 0))
        .expect("exposes");
    let mut pipeline = graph.build().expect("builds");
    let facts = pipeline.exposed(facts).expect("the exposed reader");
    pipeline.run();
    let found: Vec<u32> = events(&drain(&facts))
        .into_iter()
        .map(|event| event.3.0)
        .collect();
    assert_eq!(found, vec![25_000_000, 3]);
}

/// Channel 0 toggling every `period` samples for `len` samples, channel 1 always high.
fn toggling(start: u64, len: u64, period: u64) -> Arc<LogicBlock> {
    let mut builder = LogicBlockBuilder::new(Tick(start), 2);
    for i in 0..len {
        let level = ((start + i) / period) % 2 == 1;
        builder.push(Levels(0b10 | u64::from(level)));
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

fn events<T: Clone>(items: &[Item<T>]) -> Vec<(u64, TickRange, Tick, T)> {
    items
        .iter()
        .filter_map(Item::event)
        .map(|event| (event.seq, event.span, event.settled, event.payload.clone()))
        .collect()
}

/// Source → edge counter → doubler, with the doubler exposed.
fn chain(
    config: GraphConfig,
) -> (
    Pipeline,
    uiharu_core::SourceId,
    uiharu_core::ExposedRef<Num>,
) {
    let mut graph = Graph::with_config(config);
    let source = graph
        .source(SourceSpec::new("logic", 2, TimeBase::new(1_000)))
        .expect("adds the source");
    let counter = graph.add(EdgeCounter { channel: 0 });
    let doubler = graph.add(Doubler::new());
    graph
        .connect(source.samples, EdgeCounter::input(counter))
        .expect("connects");
    graph
        .connect(EdgeCounter::output(counter), Doubler::input(doubler))
        .expect("connects");
    let exposed = graph
        .expose("doubled", Doubler::output(doubler))
        .expect("exposes");
    (graph.build().expect("builds"), source.id, exposed)
}

#[test]
fn events_flow_through_the_chain_and_the_end_follows() {
    let (mut pipeline, source, exposed) = chain(GraphConfig::default());
    let doubled = pipeline.exposed(exposed).expect("the exposed reader");
    pipeline
        .push(source, SignalItem::Data(toggling(0, 100, 10)))
        .expect("pushes");
    let report = pipeline.run();
    assert!(report.ops > 0);
    assert!(!pipeline.is_finished());
    assert_eq!(
        report.waiting,
        vec![
            ("edges#0".to_owned(), Wait::Input),
            ("doubler#1".to_owned(), Wait::Input)
        ]
    );

    pipeline
        .push(source, SignalItem::End(Tick(100)))
        .expect("pushes");
    pipeline.run();
    assert!(pipeline.is_finished());

    let items = drain(&doubled);
    let found = events(&items);
    // Channel 0 changes at 10, 20, … 90: nine changes, numbered from one, doubled.
    assert_eq!(found.len(), 9);
    for (n, (seq, span, settled, Num(value))) in found.iter().enumerate() {
        let tick = Tick(10 * (n as u64 + 1));
        assert_eq!(*seq, n as u64);
        assert_eq!(*span, TickRange::one(tick));
        assert_eq!(*settled, tick);
        assert_eq!(*value, 2 * (n as u32 + 1));
    }
    assert_eq!(items.last(), Some(&Item::End(Tick(100))));
    assert!(doubled.is_ended());
    assert!(pipeline.stats().within_capacity());
}

#[test]
fn chunking_does_not_change_the_events() {
    let whole = {
        let (mut pipeline, source, exposed) = chain(GraphConfig::default());
        let doubled = pipeline.exposed(exposed).expect("the exposed reader");
        pipeline
            .push(source, SignalItem::Data(toggling(0, 1000, 7)))
            .expect("pushes");
        pipeline
            .push(source, SignalItem::End(Tick(1000)))
            .expect("pushes");
        pipeline.run();
        events(&drain(&doubled))
    };
    assert_eq!(whole.len(), 142);
    for sizes in [[1u64, 1, 1], [7, 64, 3], [63, 1, 64], [1000, 1, 1]] {
        let (mut pipeline, source, exposed) = chain(GraphConfig::default());
        let doubled = pipeline.exposed(exposed).expect("the exposed reader");
        let mut at = 0;
        let mut cut = 0;
        let mut found = Vec::new();
        while at < 1000 {
            let len = sizes[cut % sizes.len()].min(1000 - at);
            cut += 1;
            let block = toggling(at, len, 7);
            if let Err(PushError::Full(_)) = pipeline.push(source, SignalItem::Data(block.clone()))
            {
                pipeline.run();
                found.extend(drain(&doubled));
                pipeline
                    .push(source, SignalItem::Data(block))
                    .expect("pushes");
            }
            at += len;
        }
        // The source may be full: it takes the end only once the nodes have caught up.
        pipeline.run();
        found.extend(drain(&doubled));
        pipeline
            .push(source, SignalItem::End(Tick(1000)))
            .expect("pushes");
        pipeline.run();
        found.extend(drain(&doubled));
        assert_eq!(events(&found), whole, "cut into blocks of {sizes:?}");
    }
}

#[test]
fn a_slow_reader_holds_everything_upstream() {
    let config = GraphConfig {
        queue_capacity: 4,
        budget: 1024,
    };
    let (mut pipeline, source, exposed) = chain(config);
    let doubled = pipeline.exposed(exposed).expect("the exposed reader");
    // 100 changes, which do not fit in the queues.
    pipeline
        .push(source, SignalItem::Data(toggling(0, 1_000, 10)))
        .expect("pushes");
    pipeline
        .push(source, SignalItem::Data(toggling(1_000, 1_000, 10)))
        .expect("pushes");
    let report = pipeline.run();
    assert_eq!(
        report.waiting,
        vec![
            ("edges#0".to_owned(), Wait::Output),
            ("doubler#1".to_owned(), Wait::Output)
        ]
    );
    let stats = pipeline.stats();
    assert!(stats.within_capacity());
    assert_eq!(stats.high_water(), 4);
    assert_eq!(doubled.pending(), 4);

    // The source keeps a bounded number of blocks too.
    let mut pushed = 0;
    loop {
        match pipeline.push(
            source,
            SignalItem::Data(toggling(2_000 + 1_000 * pushed, 1_000, 10)),
        ) {
            Ok(()) => pushed += 1,
            Err(PushError::Full(SignalItem::Data(block))) => {
                assert_eq!(block.start(), Tick(2_000 + 1_000 * pushed));
                break;
            }
            Err(other) => panic!("unexpected {other:?}"),
        }
        assert!(pushed < 100, "the source never filled up");
    }
    assert!(!pipeline.has_room(source));
    // Every block has a change at each multiple of ten, including its first tick, except the
    // very first tick of the stream, which has nothing to compare with.
    let total_changes = 100 * (2 + pushed) - 1;

    // Reading in small sips drains everything exactly once.
    let mut found = Vec::new();
    while found
        .iter()
        .filter(|item: &&Item<Num>| item.event().is_some())
        .count()
        < total_changes as usize
    {
        for _ in 0..3 {
            if let Some(item) = doubled.next() {
                found.push(item);
            }
        }
        pipeline.run();
    }
    let found = events(&found);
    assert_eq!(found.len(), total_changes as usize);
    for (n, (seq, _, _, Num(value))) in found.iter().enumerate() {
        assert_eq!(*seq, n as u64);
        assert_eq!(*value, 2 * (n as u32 + 1));
    }
    assert!(pipeline.has_room(source));
}

#[test]
fn readers_of_one_output_share_its_events() {
    let mut graph = Graph::new();
    let source = graph
        .source(SourceSpec::new("logic", 2, TimeBase::new(1_000)))
        .expect("adds the source");
    let counter = graph.add(EdgeCounter { channel: 0 });
    let doubler = graph.add(Doubler::new());
    graph
        .connect(source.samples, EdgeCounter::input(counter))
        .expect("connects");
    graph
        .connect(EdgeCounter::output(counter), Doubler::input(doubler))
        .expect("connects");
    let counts = graph
        .expose("counts", EdgeCounter::output(counter))
        .expect("exposes");
    let doubled = graph
        .expose("doubled", Doubler::output(doubler))
        .expect("exposes");
    let mut pipeline = graph.build().expect("builds");
    let counts = pipeline.exposed(counts).expect("the exposed reader");
    let doubled = pipeline.exposed(doubled).expect("the exposed reader");
    assert!(
        pipeline.exposed(counts_again()).is_none(),
        "an exposure is taken once"
    );

    pipeline
        .push(source.id, SignalItem::Data(toggling(0, 50, 5)))
        .expect("pushes");
    pipeline
        .push(source.id, SignalItem::End(Tick(50)))
        .expect("pushes");
    pipeline.run();
    let counts = events(&drain(&counts));
    let doubled = events(&drain(&doubled));
    assert_eq!(counts.len(), 9);
    assert_eq!(doubled.len(), 9);
    for (count, double) in counts.iter().zip(&doubled) {
        assert_eq!(count.0, double.0);
        assert_eq!(count.1, double.1);
        assert_eq!(count.3.0 * 2, double.3.0);
    }

    fn counts_again() -> uiharu_core::ExposedRef<Num> {
        // The same exposure index as `counts` above.
        let mut graph = Graph::new();
        let source = graph
            .source(SourceSpec::new("logic", 2, TimeBase::new(1)))
            .expect("adds the source");
        let counter = graph.add(EdgeCounter { channel: 0 });
        graph
            .connect(source.samples, EdgeCounter::input(counter))
            .expect("connects");
        graph
            .expose("counts", EdgeCounter::output(counter))
            .expect("exposes")
    }
}

#[test]
fn a_small_budget_only_takes_more_rounds() {
    let (mut pipeline, source, exposed) = chain(GraphConfig {
        queue_capacity: 256,
        budget: 1,
    });
    let doubled = pipeline.exposed(exposed).expect("the exposed reader");
    pipeline
        .push(source, SignalItem::Data(toggling(0, 500, 5)))
        .expect("pushes");
    pipeline
        .push(source, SignalItem::End(Tick(500)))
        .expect("pushes");
    let report = pipeline.run();
    assert!(report.rounds > 10, "{report:?}");
    assert!(pipeline.is_finished());
    assert_eq!(events(&drain(&doubled)).len(), 99);
}

#[test]
fn gaps_and_progress_pass_through() {
    let (mut pipeline, source, exposed) = chain(GraphConfig::default());
    let doubled = pipeline.exposed(exposed).expect("the exposed reader");
    pipeline
        .push(source, SignalItem::Data(toggling(0, 30, 10)))
        .expect("pushes");
    pipeline
        .push(source, SignalItem::Progress(Tick(30)))
        .expect("pushes");
    pipeline
        .push(source, SignalItem::Gap(TickRange::new(Tick(30), Tick(45))))
        .expect("pushes");
    pipeline.run();
    // The first sample after the gap is no edge, whatever came before.
    pipeline
        .push(source, SignalItem::Data(toggling(45, 15, 10)))
        .expect("pushes");
    pipeline
        .push(source, SignalItem::End(Tick(60)))
        .expect("pushes");
    pipeline.run();
    let items = drain(&doubled);
    let ticks: Vec<_> = items
        .iter()
        .map(|item| match item {
            Item::Event(event) => format!("e{}", event.settled),
            Item::Gap(range) => format!("gap{range}"),
            Item::Progress(tick) => format!("p{tick}"),
            Item::End(tick) => format!("end{tick}"),
        })
        .collect();
    assert_eq!(
        ticks,
        vec!["e10", "e20", "p30", "gap30..45", "e50", "end60"]
    );
}

#[test]
fn the_first_item_decides_where_a_source_starts() {
    let (mut pipeline, source, exposed) = chain(GraphConfig::default());
    let doubled = pipeline.exposed(exposed).expect("the exposed reader");
    assert_eq!(pipeline.next_tick(source), None);
    pipeline
        .push(source, SignalItem::Data(toggling(1_000, 50, 10)))
        .expect("pushes");
    assert_eq!(pipeline.next_tick(source), Some(Tick(1_050)));
    pipeline
        .push(source, SignalItem::End(Tick(1_050)))
        .expect("pushes");
    pipeline.run();
    let found = events(&drain(&doubled));
    assert_eq!(found.first().map(|event| event.2), Some(Tick(1_010)));
    assert_eq!(found.len(), 4);
}

#[test]
fn pushes_must_be_contiguous() {
    let (mut pipeline, source, _) = chain(GraphConfig::default());
    pipeline
        .push(source, SignalItem::Data(toggling(0, 10, 5)))
        .expect("pushes");
    assert!(matches!(
        pipeline.push(source, SignalItem::Data(toggling(11, 10, 5))),
        Err(PushError::Discontinuity {
            expected: Tick(10),
            found: Tick(11)
        })
    ));
    assert!(matches!(
        pipeline.push(source, SignalItem::Gap(TickRange::new(Tick(9), Tick(20)))),
        Err(PushError::Discontinuity {
            expected: Tick(10),
            found: Tick(9)
        })
    ));
    let narrow = Arc::new(LogicBlock::from_interleaved(Tick(10), 1, &[0, 1]).expect("a block"));
    assert!(matches!(
        pipeline.push(source, SignalItem::Data(narrow)),
        Err(PushError::Channels {
            expected: 2,
            found: 1
        })
    ));
    assert!(matches!(
        pipeline.push(source, SignalItem::End(Tick(12))),
        Err(PushError::Discontinuity {
            expected: Tick(10),
            found: Tick(12)
        })
    ));
    pipeline
        .push(source, SignalItem::Gap(TickRange::new(Tick(10), Tick(20))))
        .expect("pushes");
    assert_eq!(pipeline.next_tick(source), Some(Tick(20)));
    pipeline
        .push(source, SignalItem::End(Tick(20)))
        .expect("pushes");
    assert!(matches!(
        pipeline.push(source, SignalItem::Progress(Tick(20))),
        Err(PushError::Ended)
    ));
    assert!(matches!(
        pipeline.push(
            uiharu_core::SourceId::from_index(7),
            SignalItem::Progress(Tick(20))
        ),
        Err(PushError::UnknownSource(7))
    ));
}

#[test]
fn connections_are_checked() {
    let mut graph = Graph::new();
    let source = graph
        .source(SourceSpec::new("logic", 1, TimeBase::new(1_000)))
        .expect("adds the source");
    let counter = graph.add(EdgeCounter { channel: 3 });
    let error = graph
        .connect(source.samples, EdgeCounter::input(counter))
        .expect_err("fails");
    assert_eq!(
        error,
        GraphError::Requirement {
            from: "logic#0.samples".into(),
            to: "edges#0.logic".into(),
            key: "channels",
            requirement: ">= 4".into(),
            found: "1".into(),
        }
    );
    assert_eq!(
        error.to_string(),
        "edges#0.logic requires channels >= 4 of logic#0.samples, which declares 1"
    );

    let picky = graph.add(Doubler {
        requires: Requires::new().equals("channel", 1),
    });
    let wide = graph
        .source(SourceSpec::new("wide", 4, TimeBase::new(1_000)))
        .expect("adds the source");
    graph
        .connect(wide.samples, EdgeCounter::input(counter))
        .expect("connects");
    assert!(matches!(
        graph.connect(EdgeCounter::output(counter), Doubler::input(picky)),
        Err(GraphError::Requirement { key: "channel", .. })
    ));
    assert_eq!(
        graph.connect(wide.samples, EdgeCounter::input(counter)),
        Err(GraphError::AlreadyConnected {
            to: "edges#0.logic".into()
        })
    );

    // A reference that lies about the port's type is caught by the schema check.
    let merger = graph.add(Merger);
    let lying: OutputRef<Num> = OutputRef::of_node(merger, 0);
    assert_eq!(
        graph.connect(lying, Doubler::input(picky)),
        Err(GraphError::SchemaMismatch {
            from: "merger#2.text".into(),
            to: "doubler#1.numbers".into(),
            output: Text::SCHEMA,
            input: Num::SCHEMA,
        })
    );
    assert!(matches!(
        graph.connect(OutputRef::<Num>::of_node(merger, 5), Doubler::input(picky)),
        Err(GraphError::UnknownPort {
            direction: "output",
            index: 5,
            ..
        })
    ));
    assert!(matches!(
        graph.connect(
            EdgeCounter::output(counter),
            InputRef::<Num>::of_node(NodeId::from_index(9), 0)
        ),
        Err(GraphError::UnknownNode(9))
    ));
}

#[test]
fn building_checks_the_whole_graph() {
    // An unconnected input.
    let mut graph = Graph::new();
    let _ = graph
        .source(SourceSpec::new("logic", 1, TimeBase::new(1_000)))
        .expect("adds the source");
    let _ = graph.add(EdgeCounter { channel: 0 });
    assert_eq!(
        graph.build().expect_err("fails"),
        GraphError::Unconnected {
            to: "edges#0.logic".into()
        }
    );

    // A cycle.
    let mut graph = Graph::new();
    let a = graph.add(Doubler::new());
    let b = graph.add(Doubler::new());
    graph
        .connect(Doubler::output(a), Doubler::input(b))
        .expect("connects");
    graph
        .connect(Doubler::output(b), Doubler::input(a))
        .expect("connects");
    assert_eq!(
        graph.build().expect_err("fails"),
        GraphError::Cycle {
            node: "doubler#0".into()
        }
    );

    // Two time domains in one node.
    let mut graph = Graph::new();
    let first = graph
        .source(SourceSpec::new("first", 1, TimeBase::new(1_000)))
        .expect("adds the source");
    let second = graph
        .source(SourceSpec::new("second", 1, TimeBase::new(2_000)))
        .expect("adds the source");
    let left = graph.add(EdgeCounter { channel: 0 });
    let right = graph.add(EdgeCounter { channel: 0 });
    let merger = graph.add(Merger);
    graph
        .connect(first.samples, EdgeCounter::input(left))
        .expect("connects");
    graph
        .connect(second.samples, EdgeCounter::input(right))
        .expect("connects");
    graph
        .connect(EdgeCounter::output(left), InputRef::of_node(merger, 0))
        .expect("connects");
    graph
        .connect(EdgeCounter::output(right), InputRef::of_node(merger, 1))
        .expect("connects");
    assert_eq!(
        graph.build().expect_err("fails"),
        GraphError::TimeDomain {
            node: "merger#2".into(),
            first: "first#0".into(),
            second: "second#1".into(),
        }
    );

    // Two exposures of one name.
    let mut graph = Graph::new();
    let source = graph
        .source(SourceSpec::new("logic", 1, TimeBase::new(1_000)))
        .expect("adds the source");
    let counter = graph.add(EdgeCounter { channel: 0 });
    graph
        .connect(source.samples, EdgeCounter::input(counter))
        .expect("connects");
    graph
        .expose("counts", EdgeCounter::output(counter))
        .expect("exposes");
    assert_eq!(
        graph
            .expose("counts", EdgeCounter::output(counter))
            .expect_err("fails"),
        GraphError::DuplicateExposure("counts".into())
    );

    // Too many channels.
    assert_eq!(
        Graph::new()
            .source(SourceSpec::new("wide", 65, TimeBase::new(1)))
            .expect_err("fails"),
        GraphError::TooManyChannels {
            name: "wide".into(),
            channels: 65
        }
    );
}
