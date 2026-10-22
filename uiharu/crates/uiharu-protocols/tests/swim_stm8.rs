// SPDX-License-Identifier: Apache-2.0

//! SWIM vectors follow ST UM0470 Rev 4 figures 6–9 and table 1, not another decoder.

use uiharu_core::testing::Harness;
use uiharu_core::{GraphConfig, GraphError, Item, LogicBlock, Tick, TickRange, TimeBase};
use uiharu_protocols::swim::synth::SwimWaveform;
use uiharu_protocols::swim::{Direction, ErrorReason, Field, Speed, SwimConfig, SwimEvent};

use uiharu_protocols::swim_stm8::{Speed as DslSpeed, SwimStm8, SwimStm8Config};
fn dsl(config: SwimConfig) -> SwimStm8Config {
    SwimStm8Config {
        data: config.data,
        clock_hz: config.clock_hz,
        speed: if config.speed == Speed::Low {
            DslSpeed::Low
        } else {
            DslSpeed::High
        },
        start_active: config.start_active,
        calibrate_sync: false,
    }
}
const RATE: u64 = 16_000_000;
fn config() -> SwimConfig {
    SwimConfig {
        data: 2,
        ..SwimConfig::default()
    }
}
fn build(config: SwimConfig, graph: GraphConfig) -> Harness<SwimEvent> {
    Harness::build(3, TimeBase::new(RATE), graph, |graph, source| {
        let node = graph.add(SwimStm8::new(dsl(config)).expect("valid config"));
        graph.connect(source.samples, SwimStm8::input(node))?;
        Ok(SwimStm8::events(node))
    })
    .expect("builds")
}
fn waveform(config: SwimConfig) -> SwimWaveform {
    let mut wave = SwimWaveform::new(config, 3, TimeBase::new(RATE), Tick::ZERO);
    wave.idle(32);
    wave
}
fn decode(config: SwimConfig, block: &LogicBlock) -> Vec<SwimEvent> {
    let mut h = build(config, GraphConfig::default());
    h.run_all(block);
    h.events().into_iter().map(|event| event.payload).collect()
}
fn parameters(wave: &mut SwimWaveform, count: u8, address: u32) {
    for value in [
        count,
        (address >> 16) as u8,
        (address >> 8) as u8,
        address as u8,
    ] {
        wave.byte(Direction::HostToTarget, value, true);
    }
}

#[test]
fn manual_bit_vectors_decode_commands_parity_ack_and_exact_provenance() {
    // UM0470 table 1 ROTF = 001; figure 8 supplies H=0, XOR parity=1, ACK=1:
    // the independent literal wire vector is 0 0 0 1 1 1.
    for speed in [Speed::Low, Speed::High] {
        let config = SwimConfig { speed, ..config() };
        let mut wave = waveform(config);
        let start = wave.position();
        wave.bits([false, false, false, true, true, true]);
        let release = start.offset(u64::from(speed.clocks()) * 2 * 5 + 4);
        wave.idle(32);
        let mut h = build(config, GraphConfig::default());
        h.run_all(&wave.finish());
        let events: Vec<_> = h.events();
        assert_eq!(events.len(), 1);
        assert_eq!(
            events[0].payload,
            SwimEvent::Command {
                value: 1,
                parity_ok: true,
                ack: true
            }
        );
        assert_eq!(events[0].span, TickRange::new(start, release.next()));
        assert_eq!(events[0].settled, release);
    }
}

#[test]
fn reads_and_writes_report_fields_addresses_and_nack_retries() {
    let mut wave = waveform(config());
    wave.command(1, true);
    parameters(&mut wave, 2, 0x123456);
    wave.byte(Direction::TargetToHost, 0xa5, false);
    wave.byte(Direction::TargetToHost, 0xa5, true);
    wave.byte(Direction::TargetToHost, 0x3c, true);
    wave.command(2, true);
    parameters(&mut wave, 1, 0x654321);
    wave.byte(Direction::HostToTarget, 0x81, true);
    wave.idle(32);
    let events = decode(config(), &wave.finish());
    assert_eq!(events.len(), 14);
    assert!(matches!(
        events[1],
        SwimEvent::Byte {
            field: Field::Count,
            value: 2,
            ..
        }
    ));
    assert!(matches!(
        events[4],
        SwimEvent::Byte {
            field: Field::AddressLow,
            value: 0x56,
            ..
        }
    ));
    for (index, address, ack) in [
        (5, 0x123456, false),
        (6, 0x123456, true),
        (7, 0x123457, true),
        (13, 0x654321, true),
    ] {
        assert!(
            matches!(events[index], SwimEvent::Byte { field: Field::Data, address: Some(a), ack: observed, .. } if a == address && observed == ack),
            "{events:?}"
        );
    }
}

#[test]
fn parity_errors_and_busy_receivers_do_not_advance_command_fields() {
    let mut wave = waveform(config());
    wave.packet(false, 2, 3, true, true); // broken command; next is still a command
    wave.command(2, false);
    wave.command(2, true);
    wave.packet(false, 2, 8, true, true); // broken count
    wave.byte(Direction::HostToTarget, 1, false); // busy count
    parameters(&mut wave, 1, 0x123456);
    wave.byte(Direction::HostToTarget, 0x55, true);
    wave.idle(32);
    let events = decode(config(), &wave.finish());
    assert!(matches!(
        events[0],
        SwimEvent::Command {
            parity_ok: false,
            ..
        }
    ));
    for index in [3, 4, 5] {
        assert!(matches!(
            events[index],
            SwimEvent::Byte {
                field: Field::Count,
                ..
            }
        ));
    }
    assert!(matches!(
        events.last(),
        Some(SwimEvent::Byte {
            address: Some(0x123456),
            field: Field::Data,
            ..
        })
    ));
}

#[test]
fn csr_speed_change_and_communication_reset_follow_the_wire() {
    let mut wave = waveform(config());
    wave.command(2, true);
    parameters(&mut wave, 1, 0x007f80);
    wave.byte(Direction::HostToTarget, 0xb0, true);
    wave.set_speed(Speed::High);
    wave.command(0, true);
    wave.reset();
    wave.command(0, true);
    wave.idle(32);
    let events = decode(config(), &wave.finish());
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(
                event,
                SwimEvent::Command {
                    value: 0,
                    parity_ok: true,
                    ack: true
                }
            ))
            .count(),
        2
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, SwimEvent::Reset))
            .count(),
        1
    );
}

#[test]
fn observed_entry_and_target_sync_allow_a_conservative_start() {
    let config = SwimConfig {
        start_active: false,
        ..config()
    };
    let mut wave = waveform(config);
    wave.command(0, true);
    wave.entry();
    wave.reset();
    wave.command(0, true);
    wave.idle(32);
    let events = decode(config, &wave.finish());
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, SwimEvent::Entry))
            .count(),
        1
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, SwimEvent::Command { .. }))
            .count(),
        1
    );
}

#[test]
fn initial_partial_pulses_and_truncation_never_fabricate_bytes() {
    let mut wave = waveform(config());
    wave.command(2, true);
    parameters(&mut wave, 1, 0x12);
    wave.byte(Direction::HostToTarget, 0x55, true);
    wave.idle(32);
    let whole = wave.finish();
    // Stop in the command's third symbol, before its parity and ACK.
    let end = Tick(32 + 2 * 44 + 10);
    let short = whole.slice(TickRange::new(Tick::ZERO, end)).expect("slice");
    assert_eq!(
        decode(config(), &short),
        [SwimEvent::Error {
            reason: ErrorReason::Aborted,
            partial: false
        }]
    );
    let tail = whole
        .slice(TickRange::new(Tick(33), whole.end()))
        .expect("slice");
    let events = decode(config(), &tail);
    assert_eq!(
        events[0],
        SwimEvent::Error {
            reason: ErrorReason::Aborted,
            partial: true
        }
    );
    assert!(
        events
            .iter()
            .all(|event| !matches!(event, SwimEvent::Command { .. } | SwimEvent::Byte { .. }))
    );
}

#[test]
fn gaps_abort_packets_and_a_reset_recovers_without_guessing_alignment() {
    let mut wave = waveform(config());
    wave.command(1, true);
    let gap_start = wave.position().offset(8);
    parameters(&mut wave, 1, 0x123456);
    wave.byte(Direction::TargetToHost, 0x55, true);
    wave.reset();
    wave.command(0, true);
    wave.idle(32);
    let block = wave.finish();
    let gap = TickRange::new(gap_start, gap_start.offset(100));
    let mut h = build(config(), GraphConfig::default());
    h.feed_with_gaps(&block, &[gap], &[17]);
    h.end();
    assert!(h.events().into_iter().any(|event| matches!(
        event.payload,
        SwimEvent::Error {
            reason: ErrorReason::Aborted,
            ..
        }
    )));
    assert!(
        h.events()
            .into_iter()
            .any(|event| matches!(event.payload, SwimEvent::Command { value: 0, .. }))
    );
    assert!(
        h.items
            .iter()
            .any(|item| matches!(item, Item::Gap(range) if *range == gap))
    );
}

#[test]
fn insufficient_sampling_is_rejected() {
    let result = Harness::build(
        3,
        TimeBase::new(7_999_999),
        GraphConfig::default(),
        |graph, source| {
            let node = graph.add(SwimStm8::new(dsl(config())).expect("valid"));
            graph.connect(source.samples, SwimStm8::input(node))?;
            Ok(SwimStm8::events(node))
        },
    );
    assert!(matches!(result, Err(GraphError::Requirement { key, .. }) if key == "rateHz"));
}
#[test]
fn synchronization_response_calibrates_after_entry_and_reset_only_when_enabled() {
    for entry in [false, true] {
        let actual = SwimConfig {
            clock_hz: 4_000_000,
            ..config()
        };
        let mut wave = waveform(actual);
        if entry {
            wave.entry()
        } else {
            wave.reset()
        }
        wave.reset();
        wave.command(1, true);
        parameters(&mut wave, 1, 0x123456);
        wave.byte(Direction::TargetToHost, 0xa5, true);
        wave.idle(32);
        let block = wave.finish();
        let build = |graph_config| {
            Harness::build(3, TimeBase::new(RATE), graph_config, |graph, source| {
                let node = graph.add(
                    SwimStm8::new(SwimStm8Config {
                        calibrate_sync: true,
                        ..dsl(config())
                    })
                    .expect("config"),
                );
                graph.connect(source.samples, SwimStm8::input(node))?;
                Ok(SwimStm8::events(node))
            })
            .expect("pipeline")
        };
        let mut h = build(GraphConfig::default());
        h.run_all(&block);
        let events = h.events();
        assert!(events.iter().any(|event| matches!(
            event.payload,
            SwimEvent::Calibration {
                clock_hz: 4_000_000,
                low_ticks: 512,
                ..
            }
        )));
        assert!(events.iter().any(|event| matches!(
            event.payload,
            SwimEvent::Byte {
                field: Field::Data,
                address: Some(0x123456),
                value: 0xa5,
                parity_ok: true,
                ack: true,
                ..
            }
        )));
        uiharu_core::testing::assert_block_invariant(build, &block, &[]);
        uiharu_core::testing::assert_block_invariant(
            build,
            &block,
            &[TickRange::new(Tick(110), Tick(140))],
        );
    }
}
#[test]
fn illegal_zero_count_does_not_consume_following_command_as_a_parameter() {
    let mut wave = waveform(config());
    wave.command(2, true);
    wave.byte(Direction::HostToTarget, 0, true);
    wave.command(0, true);
    wave.idle(32);
    let events = decode(config(), &wave.finish());
    assert!(events.contains(&SwimEvent::Error {
        reason: ErrorReason::InvalidCount,
        partial: false
    }));
    assert!(events.contains(&SwimEvent::Command {
        value: 0,
        parity_ok: true,
        ack: true
    }));
}
