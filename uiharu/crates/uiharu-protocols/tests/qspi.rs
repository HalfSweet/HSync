// SPDX-License-Identifier: Apache-2.0

use uiharu_core::testing::{Harness, assert_block_invariant};
use uiharu_core::{
    GraphConfig, Item, Levels, LogicBlock, LogicBlockBuilder, Tick, TickRange, TimeBase,
};
use uiharu_protocols::qspi::synth::QspiWaveform;
use uiharu_protocols::qspi::{EndReason, Qspi, QspiConfig, QspiEvent};

fn harness(config: &QspiConfig, g: GraphConfig) -> Harness<QspiEvent> {
    Harness::build(8, TimeBase::new(1_000_000), g, |graph, source| {
        let node = graph.add(Qspi::new(config.clone()).expect("valid"));
        graph.connect(source.samples, Qspi::input(node))?;
        Ok(Qspi::events(node))
    })
    .expect("graph")
}
fn decode(config: &QspiConfig, block: &LogicBlock, gaps: &[TickRange]) -> Vec<Item<QspiEvent>> {
    let mut h = harness(config, GraphConfig::default());
    h.feed_with_gaps(block, gaps, &[u64::MAX]);
    h.end();
    h.items
}
fn payloads(items: &[Item<QspiEvent>]) -> Vec<QspiEvent> {
    items
        .iter()
        .filter_map(|i| match i {
            Item::Event(e) => Some(e.payload),
            _ => None,
        })
        .collect()
}
fn wave(config: &QspiConfig) -> LogicBlock {
    let mut w = QspiWaveform::new(config.clone(), 8, 2, Tick::ZERO);
    w.idle(3);
    w.select();
    for v in [0xa5, 0x3c, 0xff] {
        w.word(v);
    }
    w.release();
    w.idle(3);
    w.finish()
}
#[test]
fn all_modes_lane_counts_polarities_orders_and_widths() {
    for mode in 0..4 {
        for lanes in [1, 2, 4] {
            for active_low in [false, true] {
                for lsb in [false, true] {
                    for bits in [4, 8, 16, 32] {
                        let c = QspiConfig {
                            mode,
                            data: (2..2 + lanes).collect(),
                            word_bits: bits,
                            cs_active_low: active_low,
                            lsb_first: lsb,
                            ..Default::default()
                        };
                        let mut w = QspiWaveform::new(c.clone(), 8, 2, Tick::ZERO);
                        w.idle(3);
                        w.select();
                        w.word(0xa5a5_3c3c);
                        w.release();
                        w.idle(3);
                        let block = w.finish();
                        let items = decode(&c, &block, &[]);
                        let mask = u32::MAX >> (32 - bits);
                        assert_eq!(
                            payloads(&items),
                            vec![
                                QspiEvent::Begin {
                                    tx: 1,
                                    partial: false
                                },
                                QspiEvent::Word {
                                    tx: 1,
                                    value: 0xa5a5_3c3c & mask
                                },
                                QspiEvent::End {
                                    tx: 1,
                                    reason: EndReason::Release,
                                    dropped_bits: 0
                                }
                            ]
                        );
                        let word = items
                            .iter()
                            .find_map(|i| match i {
                                Item::Event(e) if matches!(e.payload, QspiEvent::Word { .. }) => {
                                    Some(e)
                                }
                                _ => None,
                            })
                            .expect("word");
                        assert_eq!(word.span.start, Tick(7));
                        assert_eq!(
                            word.settled,
                            Tick(7 + 4 * (u64::from(bits) / u64::from(lanes) - 1))
                        );
                        assert_eq!(word.span.end, word.settled.next());
                    }
                }
            }
        }
    }
}
#[test]
fn external_lane_vector_is_not_a_synthesizer_roundtrip() {
    // W25Q128JV rev F fig. 20 (quad output): IO3..0 carry bits 7..4,
    // then bits 3..0. Byte A5 therefore has IO0..3 = 0,1,0,1 then 1,0,1,0.
    let c = QspiConfig::default();
    let mut b = LogicBlockBuilder::new(Tick::ZERO, 8);
    for (levels, len) in [
        (1, 2),
        (0, 2),
        (0b101000, 2),
        (0b101010, 2),
        (0b010100, 2),
        (0b010110, 2),
        (0, 2),
        (1, 2),
    ] {
        b.push_repeated(Levels(levels), len);
    }
    let items = decode(&c, &b.finish().expect("samples"), &[]);
    assert_eq!(payloads(&items)[1], QspiEvent::Word { tx: 1, value: 0xa5 });
}
#[test]
fn gaps_partial_frames_incomplete_words_and_unframed_stream() {
    let c = QspiConfig::default();
    let mut w = QspiWaveform::new(c.clone(), 8, 2, Tick::ZERO);
    w.select();
    w.word(0xa5);
    w.group(3);
    w.release();
    w.idle(3);
    w.select();
    w.word(0x3c);
    w.idle(2);
    let block = w.finish();
    let items = decode(&c, &block, &[]);
    assert_eq!(
        payloads(&items),
        vec![
            QspiEvent::Begin {
                tx: 1,
                partial: true
            },
            QspiEvent::Word { tx: 1, value: 0xa5 },
            QspiEvent::End {
                tx: 1,
                reason: EndReason::Release,
                dropped_bits: 4
            },
            QspiEvent::Begin {
                tx: 2,
                partial: false
            },
            QspiEvent::Word { tx: 2, value: 0x3c },
            QspiEvent::End {
                tx: 2,
                reason: EndReason::Aborted,
                dropped_bits: 0
            }
        ]
    );
    let gaps = [TickRange::new(Tick(5), Tick(7))];
    let cut = payloads(&decode(&c, &block, &gaps));
    assert!(cut.contains(&QspiEvent::End {
        tx: 1,
        reason: EndReason::Aborted,
        dropped_bits: 4
    }));
    assert!(cut.contains(&QspiEvent::Begin {
        tx: 2,
        partial: true
    }));
    assert_block_invariant(|g| harness(&c, g), &block, &gaps);
    let c = QspiConfig { cs: None, ..c };
    let items = decode(&c, &wave(&c), &[]);
    assert_eq!(
        payloads(&items),
        vec![
            QspiEvent::Word { tx: 0, value: 0xa5 },
            QspiEvent::Word { tx: 0, value: 0x3c },
            QspiEvent::Word { tx: 0, value: 0xff }
        ]
    );
}
#[test]
fn malformed_configuration_and_serialization() {
    for c in [
        QspiConfig {
            mode: 4,
            ..Default::default()
        },
        QspiConfig {
            word_bits: 0,
            ..Default::default()
        },
        QspiConfig {
            word_bits: 33,
            ..Default::default()
        },
        QspiConfig {
            word_bits: 7,
            ..Default::default()
        },
        QspiConfig {
            data: vec![2, 3, 4],
            ..Default::default()
        },
        QspiConfig {
            data: vec![64],
            ..Default::default()
        },
        QspiConfig {
            data: vec![1],
            ..Default::default()
        },
    ] {
        assert!(c.validate().is_err());
    }
    let e = QspiEvent::End {
        tx: 1,
        reason: EndReason::Aborted,
        dropped_bits: 4,
    };
    assert_eq!(
        serde_json::to_value(e).expect("json"),
        serde_json::json!({"kind":"end","tx":1,"reason":"aborted","droppedBits":4})
    );
    assert!(serde_json::from_str::<QspiConfig>(r#"{"unknown":1}"#).is_err());
}
#[test]
fn block_budget_queue_and_reader_pause_invariance() {
    for mode in 0..4 {
        let c = QspiConfig {
            mode,
            ..Default::default()
        };
        let block = wave(&c);
        assert_block_invariant(
            |g| harness(&c, g),
            &block,
            &[TickRange::new(Tick(10), Tick(11))],
        );
    }
}

#[test]
fn complete_word_is_emitted_before_cs_release_and_cancellation() {
    let c = QspiConfig::default();
    let mut w = QspiWaveform::new(c.clone(), 8, 2, Tick::ZERO);
    w.idle(3);
    w.select();
    w.word(0x5a);
    w.idle(2);
    let mut h = harness(&c, GraphConfig::default());
    h.feed(&w.finish(), &[1]);
    h.collect();
    assert!(payloads(&h.items).contains(&QspiEvent::Word { tx: 1, value: 0x5a }));
    // Dropping an unterminated pipeline cancels it, including any suspended reads.
    drop(h);
}

#[test]
fn a_cs_transition_suppresses_a_simultaneous_clock_edge() {
    let c = QspiConfig {
        word_bits: 4,
        ..Default::default()
    };
    let mut b = LogicBlockBuilder::new(Tick::ZERO, 8);
    // Both clocks coincide with CS edges: neither is a word.
    for levels in [1, 0b101010, 0, 0b010111] {
        b.push_repeated(Levels(levels), 2);
    }
    assert_eq!(
        payloads(&decode(&c, &b.finish().expect("samples"), &[])),
        vec![
            QspiEvent::Begin {
                tx: 1,
                partial: false
            },
            QspiEvent::End {
                tx: 1,
                reason: EndReason::Release,
                dropped_bits: 0
            }
        ]
    );
}

#[test]
fn graph_rejects_a_source_missing_a_configured_channel() {
    let result = Harness::<QspiEvent>::build(
        5,
        TimeBase::new(1_000_000),
        GraphConfig::default(),
        |graph, source| {
            let q = graph.add(Qspi::new(QspiConfig::default()).expect("valid config"));
            graph.connect(source.samples, Qspi::input(q))?;
            Ok(Qspi::events(q))
        },
    );
    assert!(result.is_err());
}
