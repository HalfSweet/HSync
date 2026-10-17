// SPDX-License-Identifier: Apache-2.0

//! The reference modules load, make nodes with the ports of the generated decoders, refuse
//! the configurations they refuse, and decode.

mod common;

use common::{as_dyn, module, program};
use uiharu_core::testing::Harness;
use uiharu_core::{GraphConfig, Item, Node, Tick, TimeBase};
use uiharu_interp::{DynEvent, Interpreter, Program};
use uiharu_protocols::pwm::synth::PwmWaveform;
use uiharu_protocols::pwm::{Pwm, PwmConfig};
use uiharu_protocols::spi::{Spi, SpiConfig};

#[test]
fn the_reference_modules_load() {
    for name in ["pwm", "spi", "uart", "spi-nor"] {
        let program = program(name);
        assert_eq!(program.name(), name);
    }
    let error = Program::load(&module("i2c")).expect_err("a contract alone is no decoder");
    assert_eq!(error.to_string(), "decoder: the module declares no decoder");
}

#[test]
fn an_interpreted_node_declares_the_ports_of_the_generated_one() {
    let node =
        Interpreter::new(program("pwm"), &serde_json::json!({ "channel": 3 })).expect("configures");
    let spec = node.spec();
    let compiled = Pwm::new(PwmConfig {
        channel: 3,
        ..PwmConfig::default()
    })
    .expect("configures")
    .spec();
    assert_eq!(spec.name, compiled.name);
    assert_eq!(spec.inputs[0].name, compiled.inputs[0].name);
    assert_eq!(spec.inputs[0].schema, compiled.inputs[0].schema);
    assert_eq!(spec.inputs[0].requires, compiled.inputs[0].requires);
    assert_eq!(spec.outputs[0].name, compiled.outputs[0].name);
    assert_eq!(spec.outputs[0].schema, compiled.outputs[0].schema);
    assert_eq!(spec.outputs[0].props, compiled.outputs[0].props);

    let spi = Interpreter::new(
        program("spi"),
        &serde_json::json!({ "cs": null, "word_bits": 4, "bit_order": "lsb-first" }),
    )
    .expect("configures");
    let compiled = Spi::new(SpiConfig {
        cs: None,
        word_bits: 4,
        bit_order: uiharu_protocols::spi::BitOrder::LsbFirst,
        ..SpiConfig::default()
    })
    .expect("configures")
    .spec();
    assert_eq!(spi.spec().outputs[0].props, compiled.outputs[0].props);
    assert_eq!(spi.spec().inputs[0].requires, compiled.inputs[0].requires);
    assert_eq!(
        spi.config(),
        serde_json::json!({
            "clk": 0, "cs": null, "mosi": 2, "miso": 3, "mode": 0, "word_bits": 4,
            "bit_order": "lsb-first", "cs_active_low": true
        })
    );
}

#[test]
fn configurations_are_refused_as_the_generated_decoders_refuse_them() {
    let refused = |config: serde_json::Value| {
        Interpreter::new(program("spi"), &config)
            .err()
            .map(|error| error.to_string())
    };
    assert_eq!(
        refused(serde_json::json!({ "clock": 1 })),
        Some("unknown field `clock`, expected one of `clk`, `cs`, `mosi`, `miso`, `mode`, `word_bits`, `bit_order`, `cs_active_low`".into())
    );
    assert_eq!(
        refused(serde_json::json!({ "mode": 4 })),
        Spi::new(SpiConfig {
            mode: 4,
            ..SpiConfig::default()
        })
        .err()
        .map(|error| error.to_string())
    );
    assert_eq!(
        refused(serde_json::json!({ "mosi": null, "miso": null })),
        Some("neither MOSI nor MISO is connected".into())
    );
    assert_eq!(
        refused(serde_json::json!({ "clk": 1, "cs": 1 })),
        Some("channel 1 is used for two lines".into())
    );
    assert_eq!(
        refused(serde_json::json!({ "bit_order": "little" })),
        Some("unknown variant `little`, expected `msb-first` or `lsb-first`".into())
    );
    assert_eq!(
        refused(serde_json::json!({ "mode": 300 })),
        Some("invalid value: integer `300`, expected u8".into())
    );
    assert_eq!(
        refused(serde_json::json!({ "mode": true })),
        Some("invalid type: boolean `true`, expected u8".into())
    );
}

#[test]
fn an_interpreted_pwm_decoder_measures_the_cycles_the_generated_one_measures() {
    let config = PwmConfig::default();
    let mut waveform = PwmWaveform::new(&config, 2, Tick::ZERO);
    waveform.level(false, 10);
    waveform.cycles([(100, 25), (100, 50), (80, 10)]);
    waveform.level(true, 5);
    let block = waveform.finish();

    let mut interpreted: Harness<DynEvent> = Harness::build(
        2,
        TimeBase::new(1_000_000),
        GraphConfig::default(),
        |graph, source| {
            let node = graph
                .add(Interpreter::new(program("pwm"), &serde_json::json!({})).expect("configures"));
            graph.connect(source.samples, Interpreter::logic_input(node))?;
            Ok(Interpreter::events(node))
        },
    )
    .expect("builds");
    interpreted.run_all(&block);

    let mut compiled = Harness::build(
        2,
        TimeBase::new(1_000_000),
        GraphConfig::default(),
        |graph, source| {
            let node = graph.add(Pwm::new(config).expect("configures"));
            graph.connect(source.samples, Pwm::input(node))?;
            Ok(Pwm::events(node))
        },
    )
    .expect("builds");
    compiled.run_all(&block);

    let events = interpreted.events();
    assert_eq!(
        events[0].payload.0,
        serde_json::json!({ "kind": "cycle", "periodTicks": 100, "activeTicks": 25 })
    );
    assert_eq!(interpreted.items, as_dyn(&compiled.items));
    assert!(matches!(interpreted.items.last(), Some(Item::End(_))));
}
