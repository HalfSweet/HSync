// SPDX-License-Identifier: Apache-2.0

use uiharu_core::testing::{Harness, assert_block_invariant, assert_progress_promise};
use uiharu_core::{GraphConfig, Item, LogicBlock, Tick, TickRange, TimeBase};
use uiharu_protocols::sdcard_sd::synth::SdcardSdWaveform;
use uiharu_protocols::sdcard_sd::{
    EndReason, SdcardSd, SdcardSdConfig, SdcardSdConfigError, SdcardSdEvent as E, crc7_bit,
    crc16_bit,
};

fn harness(c: &SdcardSdConfig, g: GraphConfig) -> Harness<E> {
    Harness::build(6, TimeBase::new(1_000_000), g, |graph, source| {
        let node = graph.add(SdcardSd::new(c.clone()).expect("valid"));
        graph.connect(source.samples, SdcardSd::input(node))?;
        Ok(SdcardSd::events(node))
    })
    .expect("graph")
}
fn decode(c: &SdcardSdConfig, b: &LogicBlock, gaps: &[TickRange]) -> Harness<E> {
    let mut h = harness(c, GraphConfig::default());
    h.feed_with_gaps(b, gaps, &[u64::MAX]);
    h.end();
    assert_progress_promise(&h.items);
    h
}
fn payloads(h: &Harness<E>) -> Vec<E> {
    h.items
        .iter()
        .filter_map(|i| {
            if let Item::Event(e) = i {
                Some(e.payload.clone())
            } else {
                None
            }
        })
        .collect()
}
fn wave(c: &SdcardSdConfig) -> SdcardSdWaveform {
    let mut w = SdcardSdWaveform::new(c.clone(), 6, 2, Tick::ZERO);
    w.clocks(8);
    w
}

#[test]
fn crc_known_vectors_and_config() {
    let crc = |bytes: &[u8]| {
        bytes.iter().fold(0, |mut c, &b| {
            for bit in (0..8).rev() {
                c = crc7_bit(c, b & (1 << bit) != 0);
            }
            c
        })
    };
    assert_eq!(crc(&[0x40, 0, 0, 0, 0]), 0x4a);
    assert_eq!(crc(&[0x48, 0, 0, 1, 0xaa]), 0x43);
    let mut crc16 = 0;
    for byte in b"123456789" {
        for bit in (0..8).rev() {
            crc16 = crc16_bit(crc16, byte & (1 << bit) != 0);
        }
    }
    assert_eq!(crc16, 0x31c3);
    let mut c = SdcardSdConfig::default();
    assert!(c.validate().is_ok());
    c.cmd = 0;
    assert_eq!(c.validate(), Err(SdcardSdConfigError::Duplicate(0)));
    c.cmd = 64;
    assert_eq!(c.validate(), Err(SdcardSdConfigError::Channel(64)));
    c.cmd = 1;
    c.data = vec![2, 3];
    assert_eq!(c.validate(), Err(SdcardSdConfigError::Width));
    c.data = vec![2];
    c.block_bytes = 0;
    assert_eq!(c.validate(), Err(SdcardSdConfigError::BlockLength));
    c.block_bytes = 4097;
    assert!(c.validate().is_err());
    assert_eq!(
        SdcardSdConfig::default().props().get("readOnly"),
        Some(&true.into())
    );
}

#[test]
fn commands_short_and_long_responses_have_exact_bits_and_spans() {
    let c = SdcardSdConfig::default();
    let mut w = wave(&c);
    w.token(true, 8, 0x1aa, false);
    w.clocks(3);
    w.token(false, 8, 0x1aa, false);
    w.clocks(8);
    w.token(true, 2, 0, false);
    w.clocks(3);
    w.r2([0xa5; 15]);
    w.clocks(8);
    w.token(true, 55, 0, false);
    w.clocks(3);
    w.token(false, 55, 0x20, false);
    w.clocks(8);
    w.token(true, 41, 0x40ff8000, false);
    w.clocks(3);
    w.token(false, 63, 0x80ff8000, true);
    w.clocks(8);
    let h = decode(&c, &w.finish(), &[]);
    let p = payloads(&h);
    assert_eq!(
        p.iter()
            .filter(|e| matches!(
                e,
                E::End {
                    reason: EndReason::Complete,
                    ..
                }
            ))
            .count(),
        8
    );
    assert!(p.contains(&E::Header {
        token: 4,
        host: false,
        index: 63,
        bits: 136
    }));
    assert!(p.contains(&E::Payload {
        token: 4,
        offset: 96,
        value: 0xa5a5a5,
        bits: 24
    }));
    assert!(p.contains(&E::Crc {
        token: 8,
        value: 127,
        expected: None,
        ok: None
    }));
    assert!(p.contains(&E::Crc {
        token: 1,
        value: 0x43,
        expected: Some(0x43),
        ok: Some(true)
    }));
    let events: Vec<_> = h
        .items
        .iter()
        .filter_map(|i| {
            if let Item::Event(e) = i {
                Some(e)
            } else {
                None
            }
        })
        .collect();
    assert_eq!(events[0].span, TickRange::one(Tick(34)));
    assert_eq!(events[1].span, TickRange::new(Tick(34), Tick(63)));
    assert_eq!(events[2].span, TickRange::new(Tick(66), Tick(191)));
    assert_eq!(events[2].settled, Tick(190));
}

#[test]
fn bad_crc_and_end_bits_are_reported_and_truncation_never_completes() {
    let c = SdcardSdConfig::default();
    let mut w = wave(&c);
    // CMD0 with deliberately zero CRC and low end bit.
    w.bits([false, true, false, false, false, false, false, false]);
    w.bits([false; 40]);
    w.clocks(8);
    // An incomplete CMD8 header is flushed at end of capture.
    w.bits([false, true, false, false, true]);
    let h = decode(&c, &w.finish(), &[]);
    let p = payloads(&h);
    assert!(p.contains(&E::Crc {
        token: 1,
        value: 0,
        expected: Some(0x4a),
        ok: Some(false)
    }));
    assert!(p.contains(&E::End {
        token: 1,
        reason: EndReason::InvalidEnd
    }));
    assert!(p.contains(&E::End {
        token: 2,
        reason: EndReason::Aborted
    }));
    assert!(!p.iter().any(|e| matches!(
        e,
        E::End {
            reason: EndReason::Complete,
            ..
        }
    )));
}

#[test]
fn initial_low_is_partial_and_gaps_abort_then_clear_response_context() {
    let c = SdcardSdConfig::default();
    let mut w = SdcardSdWaveform::new(c.clone(), 6, 2, Tick::ZERO);
    w.bits([false, false, true]);
    w.clocks(8);
    w.token(true, 2, 0, false);
    w.clocks(3);
    let cut = w.position();
    w.r2([0x33; 15]);
    w.clocks(16);
    w.token(true, 8, 0x1aa, false);
    w.clocks(8);
    let block = w.finish();
    // Resume on both low and high payload levels. High is not proof of idle.
    for resume in [32, 40, 48] {
        let gaps = [TickRange::new(Tick(cut.0 + 24), Tick(cut.0 + resume))];
        let h = decode(&c, &block, &gaps);
        let p = payloads(&h);
        assert!(p.contains(&E::Begin {
            token: 1,
            partial: true
        }));
        assert!(p.contains(&E::End {
            token: 1,
            reason: EndReason::Resynchronized
        }));
        assert!(p.iter().any(|e| matches!(
            e,
            E::End {
                reason: EndReason::Aborted,
                ..
            }
        )));
        assert!(p.iter().any(|e| matches!(
            e,
            E::Header {
                host: true,
                index: 8,
                ..
            }
        )));
        assert_block_invariant(|g| harness(&c, g), &block, &gaps);
    }
}

#[test]
fn one_and_four_lane_reads_are_incremental_and_check_each_crc() {
    for data in [vec![2], vec![2, 3, 4, 5]] {
        let c = SdcardSdConfig {
            data,
            block_bytes: 3,
            ..Default::default()
        };
        let mut w = wave(&c);
        w.token(true, 17, 0x1234, false);
        w.clocks(3);
        w.token(false, 17, 0, false);
        w.clocks(8);
        let start = w.position();
        w.read_block(&[0x12, 0xab, 0xf0]);
        w.clocks(8);
        let b = w.finish();
        let h = decode(&c, &b, &[]);
        let p = payloads(&h);
        assert!(p.contains(&E::Data {
            block: 3,
            offset: 0,
            value: 0x12
        }));
        assert!(p.contains(&E::Data {
            block: 3,
            offset: 1,
            value: 0xab
        }));
        assert!(p.contains(&E::Data {
            block: 3,
            offset: 2,
            value: 0xf0
        }));
        assert_eq!(
            p.iter()
                .filter(|e| matches!(e, E::DataCrc { ok: true, .. }))
                .count(),
            c.data.len()
        );
        assert!(p.contains(&E::DataEnd {
            block: 3,
            reason: EndReason::Complete
        }));
        let mut partial = harness(&c, GraphConfig::default());
        let stop = Tick(start.0 + 4 * (1 + 8 / c.data.len() as u64));
        let prefix = b.slice(TickRange::new(Tick::ZERO, stop)).expect("prefix");
        partial.feed(&prefix, &[1]);
        assert!(payloads(&partial).contains(&E::Data {
            block: 3,
            offset: 0,
            value: 0x12
        }));
        assert!(
            !payloads(&partial)
                .iter()
                .any(|e| matches!(e, E::DataEnd { .. }))
        );
        partial.end();
        assert!(payloads(&partial).contains(&E::DataEnd {
            block: 3,
            reason: EndReason::Aborted
        }));
        assert_block_invariant(|g| harness(&c, g), &b, &[]);
    }
}

#[test]
fn multiblock_stop_and_data_gap_do_not_invent_more_blocks() {
    let c = SdcardSdConfig {
        data: vec![2, 3, 4, 5],
        block_bytes: 1,
        ..Default::default()
    };
    let mut w = wave(&c);
    w.token(true, 18, 0, false);
    w.clocks(3);
    w.token(false, 18, 0, false);
    w.clocks(8);
    w.read_block(&[0x56]);
    w.clocks(8);
    w.read_block(&[0x78]);
    w.clocks(8);
    w.token(true, 12, 0, false);
    w.clocks(8);
    w.read_block(&[0x90]);
    w.clocks(8);
    let b = w.finish();
    let h = decode(&c, &b, &[]);
    assert_eq!(
        payloads(&h)
            .iter()
            .filter(|e| matches!(e, E::DataBegin { .. }))
            .count(),
        2
    );
    let gaps = [TickRange::new(Tick(474), Tick(478))];
    let h = decode(&c, &b, &gaps);
    assert!(payloads(&h).iter().any(|e| matches!(
        e,
        E::DataEnd {
            reason: EndReason::Aborted,
            ..
        }
    )));
    assert_block_invariant(|g| harness(&c, g), &b, &gaps);
}

#[test]
fn serialized_fields_preserve_transport_values() {
    let events = [
        E::Begin {
            token: 1,
            partial: false,
        },
        E::Header {
            token: 1,
            host: true,
            index: 17,
            bits: 48,
        },
        E::Payload {
            token: 1,
            offset: 0,
            value: u32::MAX,
            bits: 32,
        },
        E::Crc {
            token: 1,
            value: 1,
            expected: None,
            ok: None,
        },
        E::End {
            token: 1,
            reason: EndReason::Aborted,
        },
        E::DataBegin {
            block: 2,
            partial: false,
        },
        E::Data {
            block: 2,
            offset: 0,
            value: 255,
        },
        E::DataCrc {
            block: 2,
            lane: 0,
            value: 12,
            expected: 13,
            ok: false,
        },
        E::DataEnd {
            block: 2,
            reason: EndReason::Complete,
        },
    ];
    for e in events {
        let json = serde_json::to_value(&e).expect("serialize");
        assert!(json.get("kind").is_some());
        assert_eq!(serde_json::from_value::<E>(json).expect("deserialize"), e);
    }
}

#[test]
fn rejected_reads_and_subsequent_write_busy_are_not_read_blocks() {
    let c = SdcardSdConfig {
        data: vec![2],
        block_bytes: 1,
        ..Default::default()
    };
    for (response_index, status) in [(17, 1 << 22), (18, 0), (17, 1 << 19)] {
        let mut w = wave(&c);
        w.token(true, 17, 0, false);
        w.clocks(3);
        w.token(false, response_index, status, false);
        w.clocks(8);
        w.read_block(&[0xab]);
        w.clocks(8);
        w.token(true, 24, 0, false);
        w.clocks(8);
        w.read_block(&[0xcd]);
        w.clocks(8);
        let p = payloads(&decode(&c, &w.finish(), &[]));
        assert!(!p.iter().any(|e| matches!(e, E::DataBegin { .. })));
    }
}

#[test]
fn concurrent_command_and_data_events_keep_progress_and_budget_invariance() {
    let c = SdcardSdConfig {
        data: vec![2, 3, 4, 5],
        block_bytes: 32,
        ..Default::default()
    };
    let mut w = wave(&c);
    w.token(true, 18, 0, false);
    w.clocks(3);
    w.token(false, 18, 0, false);
    w.clocks(8);
    w.symbol(true, 0); // A read start; following CMD token clocks also carry DAT=15.
    w.token(true, 8, 0x1aa, false);
    w.clocks(8);
    let b = w.finish();
    let h = decode(&c, &b, &[]);
    assert!(payloads(&h).iter().any(|e| matches!(
        e,
        E::Header {
            host: true,
            index: 8,
            ..
        }
    )));
    assert!(
        payloads(&h)
            .iter()
            .any(|e| matches!(e, E::Data { value: 255, .. }))
    );
    assert!(payloads(&h).iter().any(|e| matches!(
        e,
        E::DataEnd {
            reason: EndReason::Aborted,
            ..
        }
    )));
    assert_block_invariant(|g| harness(&c, g), &b, &[]);
}

#[test]
fn data_crc_mismatch_and_low_end_are_explicit() {
    let c = SdcardSdConfig {
        data: vec![2],
        block_bytes: 1,
        ..Default::default()
    };
    let mut w = wave(&c);
    w.token(true, 17, 0, false);
    w.clocks(3);
    w.token(false, 17, 0, false);
    w.clocks(8);
    w.symbol(true, 0);
    for _ in 0..8 {
        w.symbol(true, 0);
    } // zero byte, calculated CRC zero
    for _ in 0..16 {
        w.symbol(true, 1);
    } // wrong received CRC 0xffff
    w.symbol(true, 0);
    w.clocks(8);
    let p = payloads(&decode(&c, &w.finish(), &[]));
    assert!(p.contains(&E::DataCrc {
        block: 3,
        lane: 0,
        value: 65535,
        expected: 0,
        ok: false
    }));
    assert!(p.contains(&E::DataEnd {
        block: 3,
        reason: EndReason::InvalidEnd
    }));
}

#[test]
fn r2_crc_covers_register_bits_and_ignores_response_header() {
    let c = SdcardSdConfig::default();
    let mut w = wave(&c);
    w.token(true, 9, 0, false);
    w.clocks(3);
    w.bits([false, false, true, true, true, true, true, true]);
    w.bits([false; 120]);
    w.bits([true; 7]);
    w.bits([true]);
    w.clocks(8);
    let p = payloads(&decode(&c, &w.finish(), &[]));
    assert!(p.contains(&E::Crc {
        token: 2,
        value: 127,
        expected: Some(0),
        ok: Some(false)
    }));
    assert!(p.contains(&E::End {
        token: 2,
        reason: EndReason::Complete
    }));
}
