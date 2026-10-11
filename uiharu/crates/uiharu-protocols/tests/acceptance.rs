// SPDX-License-Identifier: Apache-2.0

//! The acceptance criteria of the engine on the SPI → SPI NOR pipeline: however the samples
//! are cut into blocks, however often and for how long the reader pauses, and whatever the
//! budget and queue sizes, the events are the same as in one uninterrupted run; the queues
//! never grow past their capacity; cut-off and lost samples never pass for a whole command.

mod common;

use std::sync::Arc;

use common::{CHANNELS, Harness, without_progress};
use proptest::prelude::*;
use uiharu_core::{GraphConfig, Item, LogicBlock, SignalItem, Tick, TickRange};
use uiharu_protocols::spi::synth::{SpiWaveform, nor_read_words};
use uiharu_protocols::spi::{SpiConfig, SpiEvent};
use uiharu_protocols::spi_nor::{DATA_BATCH, NorEnd, NorEvent};

/// One transaction of a scenario.
#[derive(Clone, Debug)]
enum Tx {
    Read { address: u32, len: usize },
    Unknown { opcode: u8, extra: u8 },
    Short { address_bytes: u8 },
    Empty,
}

fn tx() -> impl Strategy<Value = Tx> {
    prop_oneof![
        6 => (0u32..0x100_0000, 0usize..80).prop_map(|(address, len)| Tx::Read { address, len }),
        2 => (4u8..=255, 0u8..4).prop_map(|(opcode, extra)| Tx::Unknown { opcode, extra }),
        1 => (0u8..3).prop_map(|address_bytes| Tx::Short { address_bytes }),
        1 => Just(Tx::Empty),
    ]
}

fn waveform(txs: &[Tx], mode: u8, half: u64) -> LogicBlock {
    let config = SpiConfig {
        mode,
        ..SpiConfig::default()
    };
    let mut waveform = SpiWaveform::new(config, CHANNELS, half, Tick::ZERO);
    waveform.idle(7);
    for (n, tx) in txs.iter().enumerate() {
        match tx {
            Tx::Read { address, len } => {
                let data: Vec<u8> = (0..*len).map(|i| (i * 31 + n * 7) as u8).collect();
                waveform.transaction(nor_read_words(*address, &data));
            }
            Tx::Unknown { opcode, extra } => {
                let words = std::iter::once((*opcode, 0xff))
                    .chain((0..*extra).map(|i| (i, i.wrapping_mul(3))));
                waveform.transaction(words);
            }
            Tx::Short { address_bytes } => {
                waveform.transaction(
                    nor_read_words(0xabcdef, &[])
                        .into_iter()
                        .take(1 + usize::from(*address_bytes)),
                );
            }
            Tx::Empty => waveform.transaction([]),
        }
        waveform.idle(3 + (n as u64 % 5) * 4);
    }
    waveform.finish()
}

/// The reference: the whole block in one piece, read without pausing.
fn reference(
    block: &LogicBlock,
    gap: Option<TickRange>,
    mode: u8,
) -> (Vec<Item<SpiEvent>>, Vec<Item<NorEvent>>) {
    let config = SpiConfig {
        mode,
        ..SpiConfig::default()
    };
    let mut harness = Harness::nor(config);
    match gap {
        Some(gap) => harness.feed_with_gap(block, gap, u64::MAX),
        None => harness.push(SignalItem::Data(Arc::new(block.clone()))),
    }
    harness.end();
    (
        without_progress(&harness.spi_items),
        without_progress(&harness.nor_items),
    )
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 64, ..ProptestConfig::default() })]

    /// Blocks of any size, a reader that pauses at random and tiny queues and budgets all
    /// yield the reference events.
    #[test]
    fn blocks_pauses_queues_and_budgets_do_not_change_the_events(
        txs in prop::collection::vec(tx(), 1..10),
        mode in 0u8..4,
        half in 1u64..4,
        sizes in prop::collection::vec(1u64..400, 1..6),
        pauses in prop::collection::vec(0usize..12, 1..8),
        queue_capacity in 1usize..6,
        budget in 1u32..40,
        cut in prop::option::of(0.05f64..1.0),
        gap in prop::option::of((0.05f64..0.95, 1u64..200)),
    ) {
        let whole = waveform(&txs, mode, half);
        let block = match cut {
            Some(cut) => {
                let end = Tick(((whole.len() as f64) * cut) as u64).max(Tick(1));
                whole.slice(TickRange::new(Tick::ZERO, end)).expect("a slice")
            }
            None => whole,
        };
        let gap = gap.and_then(|(at, len)| {
            let start = ((block.len() as f64) * at) as u64;
            let end = (start + len).min(block.len());
            (start < end && end <= block.len()).then(|| TickRange::new(Tick(start), Tick(end)))
        });
        let (spi_ref, nor_ref) = reference(&block, gap, mode);

        let config = GraphConfig { queue_capacity, budget };
        let mut harness = Harness::build(SpiConfig { mode, ..SpiConfig::default() }, true, config)
            .expect("builds");
        harness.drain_on_full = 1;
        let mut pause = 0;
        let mut feed = |harness: &mut Harness, piece: &LogicBlock| {
            let mut at = piece.start();
            let mut cut = 0;
            while at < piece.end() {
                let size = sizes[cut % sizes.len()];
                cut += 1;
                let end = at.offset(size).min(piece.end());
                let slice = piece.slice(TickRange::new(at, end)).expect("a slice");
                harness.push(SignalItem::Data(Arc::new(slice)));
                // Read a little, or nothing at all, between blocks.
                let count = pauses[pause % pauses.len()];
                pause += 1;
                harness.collect_some(count);
                at = end;
            }
        };
        match gap {
            Some(gap) => {
                if let Some(before) = block.slice(TickRange::new(block.start(), gap.start)) {
                    feed(&mut harness, &before);
                }
                harness.push(SignalItem::Gap(gap));
                if let Some(after) = block.slice(TickRange::new(gap.end, block.end())) {
                    feed(&mut harness, &after);
                }
            }
            None => feed(&mut harness, &block),
        }
        harness.end();

        prop_assert_eq!(without_progress(&harness.spi_items), spi_ref);
        prop_assert_eq!(without_progress(&harness.nor_items), nor_ref);
        let stats = harness.pipeline.stats();
        prop_assert!(stats.within_capacity(), "{stats:?}");
        for queue in &stats.queues {
            prop_assert!(queue.high_water <= queue.capacity.max(1), "{queue:?}");
        }
    }

    /// What a command is called depends on how it ended, and never on where the samples were
    /// cut: cut-off or lost samples never leave a complete command behind.
    #[test]
    fn ends_tell_the_truth(
        txs in prop::collection::vec(tx(), 1..6),
        cut in prop::option::of(0.05f64..1.0),
        gap in prop::option::of((0.05f64..0.95, 1u64..300)),
    ) {
        let whole = waveform(&txs, 0, 2);
        let block = match cut {
            Some(cut) => {
                let end = Tick(((whole.len() as f64) * cut) as u64).max(Tick(1));
                whole.slice(TickRange::new(Tick::ZERO, end)).expect("a slice")
            }
            None => whole,
        };
        let gap = gap.and_then(|(at, len)| {
            let start = ((block.len() as f64) * at) as u64;
            let end = (start + len).min(block.len());
            (start < end).then(|| TickRange::new(Tick(start), Tick(end)))
        });
        let (spi, nor) = reference(&block, gap, 0);

        // Every operation that began has exactly one end, and a complete one only where the
        // SPI transaction it rides on was released normally after all its words.
        let mut open = std::collections::BTreeMap::new();
        for item in &nor {
            if let Item::Event(event) = item {
                match &event.payload {
                    NorEvent::Begin { op, tx, .. } => {
                        prop_assert!(open.insert(*op, *tx).is_none(), "op {op:?} began twice");
                    }
                    NorEvent::End { op, reason } => {
                        let tx = open.remove(op).expect("an open operation");
                        let released = spi.iter().filter_map(Item::event).any(|spi_event| {
                            matches!(spi_event.payload, SpiEvent::End { tx: end_tx, reason: uiharu_protocols::spi::EndReason::CsRelease } if end_tx == tx)
                        });
                        let in_gap = gap.is_some_and(|gap| gap.contains(event.settled) || event.settled == gap.start);
                        match reason {
                            NorEnd::Complete | NorEnd::Incomplete => prop_assert!(released, "{event:?}"),
                            NorEnd::Aborted => prop_assert!(!released || in_gap, "{event:?}"),
                        }
                    }
                    NorEvent::Data { bytes, .. } => prop_assert!(bytes.len() <= DATA_BATCH),
                    _ => {}
                }
            }
        }
        prop_assert!(open.is_empty(), "operations left open: {open:?}");
    }
}

#[test]
fn queues_and_state_stay_bounded_on_a_long_read() {
    let bytes: Vec<u8> = (0..20_000u32).map(|i| (i % 253) as u8).collect();
    let mut waveform = SpiWaveform::new(SpiConfig::default(), CHANNELS, 2, Tick::ZERO);
    waveform.idle(5);
    waveform.transaction(nor_read_words(0x000000, &bytes));
    waveform.idle(5);
    let block = waveform.finish();
    let config = GraphConfig {
        queue_capacity: 8,
        budget: 64,
    };
    let mut harness = Harness::build(SpiConfig::default(), true, config).expect("builds");
    harness.drain_on_full = 2;
    let mut at = block.start();
    let mut biggest_batch = 0;
    while at < block.end() {
        let end = at.offset(1_000).min(block.end());
        let piece = block.slice(TickRange::new(at, end)).expect("a slice");
        harness.push(SignalItem::Data(Arc::new(piece)));
        harness.collect_some(3);
        let stats = harness.pipeline.stats();
        assert!(stats.high_water() <= 8, "{stats:?}");
        at = end;
    }
    harness.end();
    for item in &harness.nor_items {
        if let Item::Event(event) = item
            && let NorEvent::Data { bytes, .. } = &event.payload
        {
            biggest_batch = biggest_batch.max(bytes.len());
        }
    }
    assert_eq!(biggest_batch, DATA_BATCH);
    let data: Vec<u8> = harness
        .nor_items
        .iter()
        .filter_map(Item::event)
        .filter_map(|event| match &event.payload {
            NorEvent::Data { bytes, .. } => Some(bytes.clone()),
            _ => None,
        })
        .flatten()
        .collect();
    assert_eq!(data, bytes);
    assert!(harness.nor_items.iter().any(|item| matches!(
        item,
        Item::Event(event) if matches!(event.payload, NorEvent::End { reason: NorEnd::Complete, .. })
    )));
}
