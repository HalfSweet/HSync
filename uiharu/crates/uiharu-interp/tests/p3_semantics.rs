// SPDX-License-Identifier: Apache-2.0
//! Independent MIDI, DMX and LIN vectors, run through both execution paths.
mod common;
use common::program;
use serde_json::{Value, json};
use uiharu_core::testing::{Harness, assert_block_invariant, without_progress};
use uiharu_core::{GraphConfig, InputRef, LogicBlock, OutputRef, Props, Tick, TickRange, TimeBase};
use uiharu_interp::{DynEvent, Interpreter, ToDyn};
use uiharu_protocols::midi::{Midi, MidiEvent};
use uiharu_protocols::uart::{StopBits, Uart, UartConfig, UartEvent, synth::UartWaveform};
use uiharu_protocols::{dmx512_analysis, lin_profile, midi_semantics};

fn line(name: &str) -> UartConfig {
    UartConfig {
        baud: match name {
            "midi-semantics" => 31_250,
            "dmx512-analysis" => 250_000,
            _ => 19_200,
        },
        stop_bits: if name == "dmx512-analysis" {
            StopBits::Two
        } else {
            StopBits::One
        },
        ..UartConfig::default()
    }
}
fn build(name: &str, interpreted: bool, config: &Value, limits: GraphConfig) -> Harness<DynEvent> {
    let uart_config = line(name);
    Harness::build(
        1,
        TimeBase::new(u64::from(uart_config.baud) * 8),
        limits,
        |g, s| {
            let uart = g.add(Uart::new(uart_config).expect("UART"));
            g.connect(s.samples, Uart::input(uart))?;
            if name == "midi-semantics" {
                let midi = g.add(Midi::new());
                g.connect(Uart::events(uart), Midi::input(midi))?;
                if interpreted {
                    let b = g.add(ToDyn::<MidiEvent>::new(Props::new()));
                    g.connect(Midi::events(midi), InputRef::of_node(b, 0))?;
                    let n =
                        g.add(Interpreter::new(program(name), config).expect("MIDI interpreter"));
                    g.connect(OutputRef::of_node(b, 0), Interpreter::bus_input(n))?;
                    return Ok(Interpreter::events(n));
                }
                let n = g.add(
                    midi_semantics::MidiSemantics::new(
                        serde_json::from_value(config.clone()).expect("config"),
                    )
                    .expect("MIDI semantics"),
                );
                g.connect(Midi::events(midi), midi_semantics::MidiSemantics::input(n))?;
                let b = g.add(ToDyn::<midi_semantics::Event>::new(Props::new()));
                g.connect(
                    midi_semantics::MidiSemantics::events(n),
                    InputRef::of_node(b, 0),
                )?;
                return Ok(OutputRef::of_node(b, 0));
            }
            if interpreted {
                let b = g.add(ToDyn::<UartEvent>::new(uart_config.props()));
                g.connect(Uart::events(uart), InputRef::of_node(b, 0))?;
                let n = g.add(Interpreter::new(program(name), config).expect("interpreter"));
                g.connect(OutputRef::of_node(b, 0), Interpreter::bus_input(n))?;
                return Ok(Interpreter::events(n));
            }
            if name == "dmx512-analysis" {
                let n = g.add(
                    dmx512_analysis::Dmx512Analysis::new(
                        serde_json::from_value(config.clone()).expect("config"),
                    )
                    .expect("DMX analysis"),
                );
                g.connect(
                    Uart::events(uart),
                    dmx512_analysis::Dmx512Analysis::input(n),
                )?;
                let b = g.add(ToDyn::<dmx512_analysis::Event>::new(Props::new()));
                g.connect(
                    dmx512_analysis::Dmx512Analysis::events(n),
                    InputRef::of_node(b, 0),
                )?;
                return Ok(OutputRef::of_node(b, 0));
            }
            let n = g.add(
                lin_profile::LinProfile::new(
                    serde_json::from_value(config.clone()).expect("config"),
                )
                .expect("LIN profile"),
            );
            g.connect(Uart::events(uart), lin_profile::LinProfile::input(n))?;
            let b = g.add(ToDyn::<lin_profile::Event>::new(Props::new()));
            g.connect(lin_profile::LinProfile::events(n), InputRef::of_node(b, 0))?;
            Ok(OutputRef::of_node(b, 0))
        },
    )
    .expect("graph")
}
fn decode(name: &str, config: Value, block: &LogicBlock) -> Vec<Value> {
    let mut reference = build(name, false, &config, GraphConfig::default());
    reference.run_all(block);
    reference.assert_progress_promise();
    for gaps in [vec![], vec![TickRange::new(Tick(900), Tick(950))]] {
        for limits in [
            GraphConfig::default(),
            GraphConfig {
                queue_capacity: 1,
                budget: 3,
            },
        ] {
            let mut compiled = build(name, false, &config, limits);
            let mut interpreted = build(name, true, &config, limits);
            compiled.feed_cut(block, &gaps, &[1, 97, 211], &[0, 1, 7]);
            compiled.end();
            interpreted.feed_cut(block, &gaps, &[1, 97, 211], &[0, 1, 7]);
            interpreted.end();
            compiled.assert_progress_promise();
            interpreted.assert_progress_promise();
            assert_eq!(
                without_progress(&compiled.items),
                without_progress(&interpreted.items),
                "{name}"
            );
        }
    }
    if name != "lin-profile" {
        for interpreted in [false, true] {
            assert_block_invariant(
                |g| build(name, interpreted, &config, g),
                block,
                &[TickRange::new(Tick(900), Tick(950))],
            );
        }
    }
    reference
        .events()
        .iter()
        .map(|e| e.payload.0.clone())
        .collect()
}
#[test]
fn midi_semantics_uses_standard_units_tables_and_extended_id_namespaces() {
    let mut w = UartWaveform::new(line("midi-semantics"), 1, 8, Tick::ZERO);
    w.idle(100);
    w.bytes([
        0x90, 60, 100, 60, 0, 0x99, 38, 0x7f, 0xb0, 7, 100, 39, 1, 0xe0, 0, 0x40, 0, 0, 0xf2, 0x10,
        2, 0xf1, 0x73, 0xf0, 0x43, 1, 0xf8, 2, 0xf7, 0xf0, 0, 0x20, 0x29, 0xf7, 0xf0, 0, 0, 1,
        0xf7,
    ]);
    w.idle(100);
    let b = w.finish();
    let e = decode("midi-semantics", json!({}), &b);
    assert!(e.iter().any(|e| e["kind"] == "note"
        && e["pitchClass"] == "c"
        && e["octave"] == 4
        && e["action"] == "on"));
    assert!(
        e.iter()
            .any(|e| e["kind"] == "note" && e["note"] == 60 && e["action"] == "off")
    );
    assert!(e.iter().any(|e| e["drum"] == "acoustic-snare"));
    assert!(e.iter().any(|e| e["kind"] == "controller"
        && e["controller"] == "channel-volume"
        && e["lsb"] == true));
    assert!(e.iter().any(|e| e["kind"] == "pitch-bend"
        && e["value"] == 8192
        && e["magnitude"] == 0
        && e["negative"] == false));
    assert!(e.iter().any(|e| e["kind"] == "pitch-bend"
        && e["value"] == 0
        && e["magnitude"] == 8192
        && e["negative"] == true));
    assert!(
        e.iter().any(|e| e["kind"] == "song-position"
            && e["midiBeats"] == 272
            && e["midiClocks"] == 1632)
    );
    assert!(
        e.iter()
            .any(|e| e["kind"] == "quarter-frame" && e["piece"] == 7 && e["value"] == 3)
    );
    assert!(e.iter().any(|e| e["manufacturer"] == "yamaha"));
    assert!(e.iter().any(|e| e["manufacturer"] == "novation"));
    assert!(e.iter().any(|e| e["kind"] == "manufacturer"
        && e["extended"] == true
        && e["identifier"] == 1
        && e["manufacturer"].is_null()));
}
#[test]
fn dmx_checks_each_mark_and_filters_only_zero_dimmer_channels() {
    let mut w = UartWaveform::new(line("dmx512-analysis"), 1, 8, Tick::ZERO);
    w.idle(100);
    // 40 ticks at 2 MHz is a physically detectable 20 µs UART BREAK, short for DMX.
    w.break_(120);
    w.idle(8);
    w.bytes([0, 0, 7]);
    w.idle(500);
    w.break_(200);
    w.idle(24);
    w.bytes([0xcc, 0, 1]);
    w.idle(20);
    let e = decode("dmx512-analysis", json!({"max_mark_us":20}), &w.finish());
    for reason in [
        "short-break",
        "short-mab",
        "short-break-interval",
        "long-mark",
    ] {
        assert!(e.iter().any(|e| e["reason"] == reason), "{reason}: {e:?}");
    }
    assert!(
        e.iter().any(|e| e["kind"] == "slot"
            && e["slot"] == 2
            && e["value"] == 7
            && e["channel"] == true)
    );
    assert!(
        !e.iter()
            .any(|e| e["kind"] == "slot" && e["channel"] == true && e["value"] == 0)
    );
    assert!(
        e.iter()
            .any(|e| e["kind"] == "start-code" && e["profile"] == "rdm")
    );
    assert!(
        e.iter()
            .any(|e| e["kind"] == "slot" && e["channel"] == false && e["value"] == 0)
    );
}
#[test]
fn lin_enforces_selected_checksum_and_finishes_at_idle_without_next_break() {
    for (version, checksum, valid) in [("1", 0xe6, true), ("2", 0xe6, false), ("2", 0x96, true)] {
        let mut w = UartWaveform::new(line("lin-profile"), 1, 8, Tick::ZERO);
        w.idle(100);
        w.break_(13 * 8);
        w.idle(8);
        w.bytes([0x55, 0x50, 0x4a, 0x55, 0x93, 0xe5, checksum]);
        w.idle(20 * 8 + 100);
        let b = w.finish();
        let e = decode("lin-profile", json!({"version":version}), &b);
        assert!(
            e.iter()
                .any(|e| e["kind"] == "checksum" && e["selectedOk"] == valid),
            "{e:?}"
        );
        assert!(
            e.iter().any(|e| e["kind"] == "end"
                && e["idle"] == true
                && e["reason"] == if valid { "complete" } else { "error" }),
            "{e:?}"
        );
        assert_block_invariant(
            |g| build("lin-profile", false, &json!({"version":version}), g),
            &b,
            &[],
        );
    }
}
