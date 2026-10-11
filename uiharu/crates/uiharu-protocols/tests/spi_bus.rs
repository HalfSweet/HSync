// SPDX-License-Identifier: Apache-2.0

//! The SPI bus cursor read by a plain decoder: framed transactions and the unframed
//! transaction `0` of a bus without chip select.

use uiharu_core::testing::{self, Harness, assert_block_invariant};
use uiharu_core::{
    EventSchema, GraphConfig, InputRef, InputSpec, Io, Node, NodeFuture, NodeId, NodeSpec, Outbox,
    OutputRef, OutputSpec, Props, Requires, SchemaId, Tick, TickRange, TimeBase,
};
use uiharu_protocols::spi::synth::SpiWaveform;
use uiharu_protocols::spi::{Next, Read, Spi, SpiBus, SpiConfig, SpiEvent};

const CHANNELS: u8 = 4;
const RATE: u64 = 1_000_000;
const HALF: u64 = 5;

/// What the listing decoder saw through the cursor.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Seen {
    Open { tx: u64, partial: bool },
    Word { tx: u64, mosi: Option<u8> },
    Closed { tx: u64, outcome: String },
}

impl EventSchema for Seen {
    const SCHEMA: SchemaId = SchemaId::new("test.seen", 1);
}

/// A decoder that lists every transaction and word the cursor hands out.
struct Lister;

impl Lister {
    fn input(id: NodeId) -> InputRef<SpiEvent> {
        InputRef::of_node(id, 0)
    }

    fn events(id: NodeId) -> OutputRef<Seen> {
        OutputRef::of_node(id, 0)
    }

    async fn run(mut bus: SpiBus, mut out: Outbox<Seen>) {
        loop {
            match bus.next().await {
                Next::Transaction(mut tx) => {
                    let id = tx.id().0;
                    let partial = tx.is_partial();
                    out.emit(tx.consumed(), Seen::Open { tx: id, partial })
                        .await;
                    let outcome = loop {
                        match tx.word_or_end().await {
                            Ok(Read::Word(word)) => {
                                out.emit(
                                    tx.consumed(),
                                    Seen::Word {
                                        tx: id,
                                        mosi: word.mosi,
                                    },
                                )
                                .await;
                            }
                            Ok(Read::End) => break "end".to_owned(),
                            Err(error) => break error.to_string(),
                        }
                    };
                    out.emit(tx.consumed(), Seen::Closed { tx: id, outcome })
                        .await;
                }
                Next::Gap(range) => out.gap(range).await,
                Next::Progress(tick) => out.progress(tick).await,
                Next::End(tick) => {
                    out.end(tick).await;
                    return;
                }
            }
        }
    }
}

impl Node for Lister {
    fn spec(&self) -> NodeSpec {
        NodeSpec {
            name: "lister".into(),
            inputs: vec![InputSpec::new::<SpiEvent>("bus", Requires::new())],
            outputs: vec![OutputSpec::new::<Seen>("events", Props::new())],
        }
    }

    fn start(self: Box<Self>, mut io: Io) -> NodeFuture {
        Box::pin(Self::run(SpiBus::new(io.input(0)), io.output(0)))
    }
}

fn config(cs: bool) -> SpiConfig {
    SpiConfig {
        cs: cs.then_some(1),
        miso: None,
        ..SpiConfig::default()
    }
}

fn build(spi: SpiConfig, graph: GraphConfig) -> Harness<Seen> {
    Harness::build(CHANNELS, TimeBase::new(RATE), graph, |g, source| {
        let spi = g.add(Spi::new(spi).expect("a valid configuration"));
        g.connect(source.samples, Spi::input(spi))?;
        let lister = g.add(Lister);
        g.connect(Spi::events(spi), Lister::input(lister))?;
        Ok(Lister::events(lister))
    })
    .expect("the graph builds")
}

fn seen(harness: &Harness<Seen>) -> Vec<String> {
    testing::without_progress(
        &harness
            .events()
            .iter()
            .map(|event| uiharu_core::Item::Event(event.clone()))
            .collect::<Vec<_>>(),
    )
    .into_iter()
    .filter_map(|item| item.into_event())
    .map(|event| match event.payload {
        Seen::Open { tx, partial } => format!("open {tx}{}", if partial { " partial" } else { "" }),
        Seen::Word { tx, mosi } => format!("word {tx} {:02x}", mosi.unwrap_or(0)),
        Seen::Closed { tx, outcome } => format!("closed {tx}: {outcome}"),
    })
    .collect()
}

/// Words on a bus without chip select, with a pause between the two groups.
fn unframed_waveform() -> SpiWaveform {
    let mut waveform = SpiWaveform::new(config(false), CHANNELS, HALF, Tick::ZERO);
    waveform.idle(20);
    waveform.words([(0x11, 0), (0x22, 0), (0x33, 0)]);
    waveform.idle(100);
    waveform.words([(0x44, 0), (0x55, 0)]);
    waveform.idle(20);
    waveform
}

#[test]
fn an_unframed_bus_is_one_transaction_zero_cut_only_by_the_end() {
    let block = unframed_waveform().finish();
    let mut harness = build(config(false), GraphConfig::default());
    harness.run_all(&block);
    assert_eq!(
        seen(&harness),
        [
            "open 0",
            "word 0 11",
            "word 0 22",
            "word 0 33",
            "word 0 44",
            "word 0 55",
            &format!("closed 0: the stream ended at tick {}", block.len()),
        ]
    );
}

#[test]
fn a_gap_cuts_transaction_zero_and_the_next_word_reopens_it() {
    let block = unframed_waveform().finish();
    // The three words take 8 bits of two half periods each from tick 20; the gap falls in
    // the pause after them.
    let gap = TickRange::new(Tick(280), Tick(300));
    let mut harness = build(config(false), GraphConfig::default());
    harness.feed_with_gaps(&block, &[gap], &[4096]);
    harness.end();
    assert_eq!(
        seen(&harness),
        [
            "open 0",
            "word 0 11",
            "word 0 22",
            "word 0 33",
            "closed 0: samples are missing from tick 280",
            "open 0",
            "word 0 44",
            "word 0 55",
            &format!("closed 0: the stream ended at tick {}", block.len()),
        ]
    );
    let gaps: Vec<TickRange> = harness
        .without_progress()
        .into_iter()
        .filter_map(|item| match item {
            uiharu_core::Item::Gap(range) => Some(range),
            _ => None,
        })
        .collect();
    assert_eq!(gaps, [gap]);
}

#[test]
fn framed_transactions_open_at_their_begin_event() {
    let mut waveform = SpiWaveform::new(config(true), CHANNELS, HALF, Tick::ZERO);
    waveform.idle(20);
    waveform.transaction([(0xa1, 0), (0xa2, 0)]);
    waveform.idle(20);
    waveform.transaction([(0xb1, 0)]);
    waveform.idle(20);
    let block = waveform.finish();
    let mut harness = build(config(true), GraphConfig::default());
    harness.run_all(&block);
    assert_eq!(
        seen(&harness),
        [
            "open 1",
            "word 1 a1",
            "word 1 a2",
            "closed 1: end",
            "open 2",
            "word 2 b1",
            "closed 2: end",
        ]
    );
}

#[test]
fn the_listing_does_not_depend_on_blocks_pauses_or_queues() {
    let block = unframed_waveform().finish();
    assert_block_invariant(
        |graph| build(config(false), graph),
        &block,
        &[TickRange::new(Tick(280), Tick(300))],
    );
    let mut waveform = SpiWaveform::new(config(true), CHANNELS, HALF, Tick::ZERO);
    waveform.idle(20);
    waveform.transaction([(0xa1, 0), (0xa2, 0)]);
    waveform.idle(20);
    waveform.transaction([(0xb1, 0)]);
    waveform.idle(20);
    let block = waveform.finish();
    assert_block_invariant(
        |graph| build(config(true), graph),
        &block,
        &[TickRange::new(Tick(60), Tick(70))],
    );
}
