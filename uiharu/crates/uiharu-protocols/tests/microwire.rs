// SPDX-License-Identifier: Apache-2.0

//! The Microwire decoder on synthesized waveforms.

use uiharu_core::testing::{Harness, assert_block_invariant, without_progress};
use uiharu_core::{
    EventSchema, GraphConfig, InputRef, InputSpec, Io, Item, Levels, LogicBlock, LogicBlockBuilder,
    Node, NodeFuture, NodeId, NodeSpec, Outbox, OutputRef, OutputSpec, Props, Requires, SchemaId,
    Tick, TickRange, TimeBase,
};
use uiharu_protocols::microwire::synth::MicrowireWaveform;
use uiharu_protocols::microwire::{
    Microwire, MicrowireBus, MicrowireConfig, MicrowireEvent, Next, Read, ReadError, Transaction,
};

const CHANNELS: u8 = 4;
const HALF: u64 = 5;

/// CS on channel 0, SK on 1, DI on 2, DO on 3.
const CS: u64 = 1;
const SK: u64 = 2;
const DI: u64 = 4;
const DO: u64 = 8;

fn build(config: MicrowireConfig, graph: GraphConfig) -> Harness<MicrowireEvent> {
    Harness::build(
        CHANNELS,
        TimeBase::new(1_000_000),
        graph,
        |graph, source| {
            let bus = graph.add(Microwire::new(config).expect("a valid configuration"));
            graph.connect(source.samples, Microwire::input(bus))?;
            Ok(Microwire::events(bus))
        },
    )
    .expect("builds")
}

fn waveform(config: MicrowireConfig) -> MicrowireWaveform {
    let mut waveform = MicrowireWaveform::new(config, CHANNELS, HALF, Tick::ZERO);
    waveform.idle(20);
    waveform
}

fn at(span: TickRange, settled: Tick) -> String {
    if span.is_empty() {
        format!("@{settled}")
    } else {
        format!("[{},{})@{settled}", span.start, span.end)
    }
}

fn level(value: Option<bool>) -> &'static str {
    match value {
        Some(true) => "1",
        Some(false) => "0",
        None => "-",
    }
}

/// One line per item, progress left out: it depends on how the samples were cut.
fn describe(items: &[Item<MicrowireEvent>]) -> Vec<String> {
    without_progress(items)
        .iter()
        .map(|item| match item {
            Item::Event(event) => {
                let what = match event.payload {
                    MicrowireEvent::Begin { tx, partial } => {
                        format!("begin tx{}{}", tx.0, if partial { " partial" } else { "" })
                    }
                    MicrowireEvent::Bit { tx, di, r#do } => {
                        format!("bit tx{} {}/{}", tx.0, u8::from(di), level(r#do))
                    }
                    MicrowireEvent::End {
                        tx,
                        bits,
                        reason,
                        r#do,
                    } => format!("end tx{} {bits} {reason:?} do {}", tx.0, level(r#do)),
                };
                format!("{what} {}", at(event.span, event.settled))
            }
            Item::Gap(range) => format!("gap {range}"),
            Item::Progress(tick) => format!("progress {tick}"),
            Item::End(tick) => format!("eof {tick}"),
        })
        .collect()
}

fn decode(config: MicrowireConfig, block: &LogicBlock) -> Vec<String> {
    let mut harness = build(config, GraphConfig::default());
    harness.run_all(block);
    describe(&harness.items)
}

/// The line of bit `n` of a transaction selected at 20, DO read on the falling edge.
fn bit_line(n: u64, bits: &str) -> String {
    let rise = 20 + 2 * HALF + 2 * HALF * n;
    let fall = rise + HALF;
    format!("bit tx1 {bits} [{rise},{})@{fall}", fall + 1)
}

#[test]
fn di_is_read_on_the_rising_edge_and_do_on_the_falling_edge() {
    let config = MicrowireConfig::default();
    let mut waveform = waveform(config);
    waveform.transaction([(true, true), (false, false), (true, false)]);
    waveform.idle(10);
    let block = waveform.finish();
    // Chip select asserts at 20; bits rise every period from 30; it releases half a period
    // after the last bit, at 60. DO is read low on the last sample before.
    assert_eq!(
        decode(config, &block),
        vec![
            "begin tx1 [20,21)@20".to_owned(),
            bit_line(0, "1/1"),
            bit_line(1, "0/0"),
            bit_line(2, "1/0"),
            "end tx1 3 Release do 0 [60,61)@60".to_owned(),
            "eof 70".to_owned(),
        ]
    );
}

#[test]
fn without_do_bits_are_read_on_the_rising_edge() {
    let config = MicrowireConfig {
        r#do: None,
        ..MicrowireConfig::default()
    };
    let mut waveform = waveform(config);
    waveform.transaction([(true, true), (false, true)]);
    waveform.idle(10);
    assert_eq!(
        decode(config, &waveform.finish()),
        vec![
            "begin tx1 [20,21)@20",
            "bit tx1 1/- [30,31)@30",
            "bit tx1 0/- [40,41)@40",
            "end tx1 2 Release do - [50,51)@50",
            "eof 60",
        ]
    );
}

#[test]
fn without_chip_select_bits_are_unframed() {
    let config = MicrowireConfig {
        cs: None,
        ..MicrowireConfig::default()
    };
    let mut waveform = waveform(config);
    waveform.bits([(true, false), (true, true)]);
    waveform.idle(10);
    assert_eq!(
        decode(config, &waveform.finish()),
        vec!["bit tx0 1/0 [25,31)@30", "bit tx0 1/1 [35,41)@40", "eof 50"]
    );
}

#[test]
fn chip_select_may_be_active_low() {
    let config = MicrowireConfig {
        cs_active_low: true,
        ..MicrowireConfig::default()
    };
    let mut waveform = waveform(config);
    waveform.transaction([(false, true)]);
    waveform.idle(10);
    assert_eq!(
        decode(config, &waveform.finish()),
        vec![
            "begin tx1 [20,21)@20".to_owned(),
            bit_line(0, "0/1"),
            "end tx1 1 Release do 1 [40,41)@40".to_owned(),
            "eof 50".to_owned(),
        ]
    );
}

#[test]
fn chip_select_found_asserted_begins_a_partial_transaction() {
    let config = MicrowireConfig::default();
    let mut waveform = waveform(config);
    waveform.transaction([(true, false), (false, true), (true, true), (false, false)]);
    waveform.idle(10);
    let block = waveform.finish();
    // From the middle of bit 1's high phase: its rising edge was not seen.
    let tail = block
        .slice(TickRange::new(Tick(42), block.end()))
        .expect("a slice");
    assert_eq!(
        decode(config, &tail),
        vec![
            "begin tx1 partial [42,43)@42".to_owned(),
            bit_line(2, "1/1"),
            bit_line(3, "0/0"),
            "end tx1 2 Release do 0 [70,71)@70".to_owned(),
            "eof 80".to_owned(),
        ]
    );
}

#[test]
fn chip_select_released_with_the_clock_high_ends_the_bit_there() {
    let mut builder = LogicBlockBuilder::new(Tick::ZERO, CHANNELS);
    builder.push_repeated(Levels(DO), 5);
    builder.push_repeated(Levels(CS | DO), 5);
    builder.push_repeated(Levels(CS | DI | DO), 5);
    builder.push_repeated(Levels(CS | SK | DI | DO), 1);
    builder.push_repeated(Levels(CS | SK | DI), 4);
    // Chip select falls with SK still high, and DO is released at the same tick.
    builder.push_repeated(Levels(SK | DI | DO), 5);
    builder.push_repeated(Levels(DO), 5);
    let block = builder.finish().expect("a block");
    assert_eq!(
        decode(MicrowireConfig::default(), &block),
        vec![
            "begin tx1 [5,6)@5",
            "bit tx1 1/0 [15,20)@20",
            "end tx1 1 Release do 0 [20,21)@20",
            "eof 30"
        ]
    );
}

#[test]
fn a_clock_edge_as_chip_select_changes_belongs_to_no_transaction() {
    let mut builder = LogicBlockBuilder::new(Tick::ZERO, CHANNELS);
    builder.push_repeated(Levels(DO), 5);
    // Chip select and SK rise together: no bit.
    builder.push_repeated(Levels(CS | SK | DI | DO), 5);
    builder.push_repeated(Levels(CS | DO), 5);
    // One whole bit.
    builder.push_repeated(Levels(CS | SK | DO), 5);
    builder.push_repeated(Levels(CS | DI | DO), 5);
    // A rising edge, then chip select and SK fall together: the bit ends at the release.
    builder.push_repeated(Levels(CS | SK | DI | DO), 5);
    builder.push_repeated(Levels(DI | DO), 5);
    let block = builder.finish().expect("a block");
    assert_eq!(
        decode(MicrowireConfig::default(), &block),
        vec![
            "begin tx1 [5,6)@5",
            "bit tx1 0/1 [15,21)@20",
            "bit tx1 1/1 [25,30)@30",
            "end tx1 2 Release do 1 [30,31)@30",
            "eof 35"
        ]
    );
}

#[test]
fn a_gap_aborts_the_transaction_and_drops_the_bit_under_way() {
    let config = MicrowireConfig::default();
    let mut waveform = waveform(config);
    waveform.transaction([(true, false), (true, true), (false, true), (true, false)]);
    waveform.idle(10);
    let block = waveform.finish();
    // The gap begins between bit 1's rising and falling edges, after DO changed, and ends in
    // bit 2's low phase.
    let gap = TickRange::new(Tick(42), Tick(48));
    let mut harness = build(config, GraphConfig::default());
    harness.feed_with_gaps(&block, &[gap], &[7]);
    harness.end();
    assert_eq!(
        describe(&harness.items),
        vec![
            "begin tx1 [20,21)@20".to_owned(),
            bit_line(0, "1/0"),
            "end tx1 1 Aborted do 1 @42".to_owned(),
            format!("gap {gap}"),
            "begin tx2 partial [48,49)@48".to_owned(),
            "bit tx2 0/1 [50,56)@55".to_owned(),
            "bit tx2 1/0 [60,66)@65".to_owned(),
            "end tx2 2 Release do 0 [70,71)@70".to_owned(),
            "eof 80".to_owned(),
        ]
    );
}

#[test]
fn the_end_of_the_samples_aborts_an_open_transaction() {
    let config = MicrowireConfig::default();
    let mut waveform = waveform(config);
    waveform.select();
    waveform.bits([(true, true), (false, false)]);
    // The samples end in the low phase after the second bit.
    waveform.idle(HALF);
    let block = waveform.finish();
    let lines = decode(config, &block);
    assert_eq!(
        lines[1..],
        [
            bit_line(0, "1/1"),
            bit_line(1, "0/0"),
            "end tx1 2 Aborted do 0 @50".to_owned(),
            "eof 50".to_owned(),
        ]
    );

    // A rising edge whose falling edge the end cuts off makes no bit.
    let mut builder = LogicBlockBuilder::new(Tick::ZERO, CHANNELS);
    builder.push_repeated(Levels(DO), 5);
    builder.push_repeated(Levels(CS | DO), 5);
    builder.push_repeated(Levels(CS | SK | DI), 5);
    let block = builder.finish().expect("a block");
    assert_eq!(
        decode(config, &block),
        vec!["begin tx1 [5,6)@5", "end tx1 0 Aborted do 0 @15", "eof 15"]
    );
}

#[test]
fn the_end_reports_a_ready_state_shown_on_do() {
    let config = MicrowireConfig::default();
    let mut waveform = waveform(config);
    // Busy: DO low while chip select is asserted, no clock at all.
    waveform.select();
    waveform.set_out(false);
    waveform.idle(30);
    waveform.release();
    waveform.idle(10);
    // Then ready: DO rises while chip select is asserted.
    waveform.select();
    waveform.set_out(false);
    waveform.idle(10);
    waveform.set_out(true);
    waveform.idle(10);
    waveform.release();
    waveform.idle(10);
    let lines = decode(config, &waveform.finish());
    assert_eq!(
        lines,
        vec![
            "begin tx1 [20,21)@20",
            "end tx1 0 Release do 0 [60,61)@60",
            "begin tx2 [70,71)@70",
            "end tx2 0 Release do 1 [100,101)@100",
            "eof 110"
        ]
    );
}

/// Transactions of every shape.
fn busy_bus(config: MicrowireConfig) -> LogicBlock {
    let mut waveform = waveform(config);
    for n in 0..12u32 {
        let bits = (0..(n % 7 + 1)).map(|i| ((n + i) % 3 == 0, (n * i) % 2 == 1));
        waveform.transaction(bits);
        waveform.idle(u64::from(n % 4) * 3 + 1);
    }
    waveform.finish()
}

#[test]
fn every_cut_and_pause_yields_the_same_events() {
    for config in [
        MicrowireConfig::default(),
        MicrowireConfig {
            r#do: None,
            ..MicrowireConfig::default()
        },
        MicrowireConfig {
            cs: None,
            ..MicrowireConfig::default()
        },
    ] {
        let block = busy_bus(config);
        let third = block.len() / 3;
        let gaps = [
            TickRange::new(Tick(third), Tick(third + 13)),
            TickRange::new(Tick(2 * third + 3), Tick(2 * third + 4)),
        ];
        assert_block_invariant(|graph| build(config, graph), &block, &gaps);
    }
}

#[test]
fn progress_never_passes_a_bit_under_way() {
    let config = MicrowireConfig::default();
    let block = busy_bus(config);
    for sizes in [&[1u64][..], &[3], &[64, 7]] {
        let mut harness = build(config, GraphConfig::default());
        harness.feed_cut(&block, &[], sizes, &[1]);
        harness.end();
        let mut seen = Tick::ZERO;
        for item in &harness.items {
            match item {
                Item::Progress(tick) => seen = seen.max(*tick),
                Item::Event(event) => {
                    assert!(event.span.start >= seen, "{event:?} after {seen}");
                }
                Item::Gap(_) | Item::End(_) => {}
            }
        }
        assert!(seen > Tick::ZERO, "blocks of {sizes:?} announce progress");
    }
}

// Reading the stream through the cursor.

/// What the test reader made of a transaction.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Seen {
    /// Read to its normal end: the bits on DI and on DO, the first in the highest place, how
    /// many, and DO at the end.
    Whole {
        tx: u64,
        di: u64,
        out: u64,
        bits: u32,
        end: Option<bool>,
    },
    /// Ended otherwise.
    Failed { tx: u64, error: ReadError },
    /// Found under way, and drained.
    Partial { tx: u64 },
}

impl EventSchema for Seen {
    const SCHEMA: SchemaId = SchemaId::new("test.seen", 1);
}

/// Reads every transaction through the cursor: `least` bits with `bit`, with their DO level,
/// then the rest with `bit_or_end`.
struct Reader {
    least: u32,
}

impl Reader {
    fn input(node: NodeId) -> InputRef<MicrowireEvent> {
        InputRef::of_node(node, 0)
    }

    fn seen(node: NodeId) -> OutputRef<Seen> {
        OutputRef::of_node(node, 0)
    }
}

impl Node for Reader {
    fn spec(&self) -> NodeSpec {
        NodeSpec {
            name: "reader".into(),
            inputs: vec![InputSpec::new::<MicrowireEvent>(
                "microwire",
                Requires::new(),
            )],
            outputs: vec![OutputSpec::new::<Seen>("seen", Props::new())],
        }
    }

    fn start(self: Box<Self>, mut io: Io) -> NodeFuture {
        Box::pin(read(
            MicrowireBus::new(io.input(0)),
            io.output(0),
            self.least,
        ))
    }
}

async fn whole(
    tx: &mut Transaction<'_>,
    least: u32,
) -> Result<(u64, u64, u32, Option<bool>), ReadError> {
    let (mut di, mut out, mut bits) = (0u64, 0u64, 0u32);
    while bits < least {
        let bit = tx.bit().await?;
        di = (di << 1) | u64::from(bit.di);
        out = (out << 1) | u64::from(bit.data_out()?);
        bits += 1;
    }
    loop {
        match tx.bit_or_end().await? {
            Read::Bit(bit) => {
                di = (di << 1) | u64::from(bit.di);
                out = (out << 1) | u64::from(bit.r#do.unwrap_or(false));
                bits += 1;
            }
            Read::End { r#do } => return Ok((di, out, bits, r#do)),
        }
    }
}

async fn read(mut bus: MicrowireBus, mut out: Outbox<Seen>, least: u32) {
    loop {
        match bus.next().await {
            Next::Transaction(mut tx) => {
                let id = tx.id().0;
                let seen = if tx.is_partial() {
                    tx.drain().await.map(|()| Seen::Partial { tx: id })
                } else {
                    whole(&mut tx, least)
                        .await
                        .map(|(di, out, bits, end)| Seen::Whole {
                            tx: id,
                            di,
                            out,
                            bits,
                            end,
                        })
                };
                let seen = seen.unwrap_or_else(|error| Seen::Failed { tx: id, error });
                let at = tx.consumed();
                out.emit(at, seen).await;
            }
            Next::Gap(range) => out.gap(range).await,
            Next::Progress(tick) => out.progress(tick).await,
            Next::End(tick) => {
                out.end(tick).await;
                return;
            }
        }
        // Whatever the reader left of a transaction is skipped.
        let _ = bus.finish().await;
        bus.consumed();
    }
}

fn reader(config: MicrowireConfig, least: u32) -> Harness<Seen> {
    Harness::build(
        CHANNELS,
        TimeBase::new(1_000_000),
        GraphConfig::default(),
        |graph, source| {
            let bus = graph.add(Microwire::new(config).expect("a valid configuration"));
            graph.connect(source.samples, Microwire::input(bus))?;
            let reader = graph.add(Reader { least });
            graph.connect(Microwire::events(bus), Reader::input(reader))?;
            Ok(Reader::seen(reader))
        },
    )
    .expect("builds")
}

fn seen(items: &[Item<Seen>]) -> Vec<String> {
    without_progress(items)
        .iter()
        .map(|item| match item {
            Item::Event(event) => format!("{:?} {}", event.payload, at(event.span, event.settled)),
            Item::Gap(range) => format!("gap {range}"),
            Item::Progress(tick) => format!("progress {tick}"),
            Item::End(tick) => format!("eof {tick}"),
        })
        .collect()
}

#[test]
fn the_cursor_reads_bits_up_to_the_normal_end() {
    let config = MicrowireConfig::default();
    let mut waveform = waveform(config);
    waveform.transaction([(true, false), (false, true), (true, true)]);
    waveform.idle(10);
    waveform.transaction([(true, true)]);
    waveform.idle(10);
    let mut harness = reader(config, 2);
    harness.run_all(&waveform.finish());
    // The first transaction spans its chip select period; the second ends before the two bits
    // the reader asks for.
    assert_eq!(
        seen(&harness.items),
        vec![
            "Whole { tx: 1, di: 5, out: 3, bits: 3, end: Some(true) } [20,61)@60",
            "Failed { tx: 2, error: Incomplete } [70,91)@90",
            "eof 100",
        ]
    );
}

#[test]
fn the_cursor_reports_how_a_transaction_was_cut_off() {
    let config = MicrowireConfig::default();
    let mut waveform = waveform(config);
    waveform.transaction([(true, false), (true, true), (false, true), (true, false)]);
    waveform.idle(10);
    waveform.select();
    waveform.bits([(false, false)]);
    let block = waveform.finish();
    let mut harness = reader(config, 0);
    harness.feed_with_gaps(&block, &[TickRange::new(Tick(42), Tick(48))], &[9]);
    harness.end();
    // An aborted end is a truncated transaction; the one found under way after the gap is
    // partial; the end of the samples truncates the last, whose only bit the end cut off.
    assert_eq!(
        seen(&harness.items),
        vec![
            "Failed { tx: 1, error: Truncated } [20,42)@42",
            "gap 42..48",
            "Partial { tx: 2 } [48,71)@70",
            "Failed { tx: 3, error: Truncated } [80,95)@95",
            "eof 95",
        ]
    );
}

#[test]
fn the_cursor_has_no_do_level_without_the_line() {
    let config = MicrowireConfig {
        r#do: None,
        ..MicrowireConfig::default()
    };
    let mut waveform = waveform(config);
    waveform.transaction([(true, true), (true, false)]);
    waveform.idle(10);
    let mut harness = reader(config, 1);
    harness.run_all(&waveform.finish());
    assert_eq!(
        seen(&harness.items)[0],
        "Failed { tx: 1, error: NoDataOut } [20,31)@30"
    );
}

#[test]
fn without_chip_select_the_cursor_opens_transaction_zero_at_its_first_bit() {
    let config = MicrowireConfig {
        cs: None,
        ..MicrowireConfig::default()
    };
    let mut waveform = waveform(config);
    waveform.bits([(true, false), (false, true), (true, true)]);
    waveform.idle(30);
    waveform.bits([(true, true)]);
    waveform.idle(10);
    let block = waveform.finish();
    // Read whole: one transaction 0 from the first bit, cut by the end of the samples; the
    // reader's provenance shows every bit went through the cursor.
    let mut harness = reader(config, 4);
    harness.run_all(&block);
    assert_eq!(
        seen(&harness.items),
        vec![
            "Failed { tx: 0, error: EndOfInput(Tick(100)) } [25,91)@90",
            "eof 100",
        ]
    );
    // A gap cuts transaction 0; the next bit opens it again.
    let gap = TickRange::new(Tick(60), Tick(70));
    let mut harness = reader(config, 3);
    harness.feed_with_gaps(&block, &[gap], &[7]);
    harness.end();
    assert_eq!(
        seen(&harness.items),
        vec![
            "Failed { tx: 0, error: Gap(TickRange { start: Tick(60), end: Tick(70) }) } [25,51)@50",
            "gap 60..70",
            "Failed { tx: 0, error: EndOfInput(Tick(100)) } [85,91)@90",
            "eof 100",
        ]
    );
}
