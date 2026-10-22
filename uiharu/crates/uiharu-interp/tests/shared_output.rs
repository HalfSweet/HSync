// SPDX-License-Identifier: Apache-2.0

//! A separate producer of an imported contract publishes the same native payload through a
//! bridge, including its provenance, across block and queue boundaries.

mod common;

use std::sync::Arc;

use serde_json::json;
use uiharu_core::testing::Harness;
use uiharu_core::{GraphConfig, InputRef, Item, Node, OutputRef, Tick, TimeBase};
use uiharu_interp::{DynEvent, FromDyn, Interpreter, Program};
use uiharu_ir::Module;
use uiharu_protocols::pwm::synth::PwmWaveform;
use uiharu_protocols::pwm::{Pwm, PwmConfig, PwmEvent};

fn producer() -> Module {
    let mut module = common::module("pwm");
    module.name = "independent-pwm".into();
    module.decoder.as_mut().expect("a decoder").name = module.name.clone();
    let event = module.types.remove(0);
    let contract = module.contracts.remove(0);
    module.imports.push(
        serde_json::from_value(json!({ "module": "pwm", "types": [event], "contract": contract }))
            .expect("the imported contract parses"),
    );
    module
}

#[test]
fn an_independent_producer_emits_the_imported_contract_as_native_events() {
    let module = producer();
    let program = Arc::new(Program::load(&module).expect("an imported output contract loads"));
    let config = PwmConfig::default();
    let config_json = serde_json::to_value(config).expect("configuration serializes");
    let mut fault = None;
    let mut interpreted: Harness<PwmEvent> = Harness::build(
        1,
        TimeBase::new(1_000_000),
        GraphConfig::default(),
        |graph, source| {
            let producer = Interpreter::new(program, &config_json).expect("configures");
            let props = producer.spec().outputs[0].props.clone();
            let producer = graph.add(producer);
            let bridge = FromDyn::<PwmEvent>::new(props);
            fault = Some(bridge.fault_handle());
            let bridge = graph.add(bridge);
            graph.connect(source.samples, Interpreter::logic_input(producer))?;
            graph.connect(
                Interpreter::events(producer),
                InputRef::<DynEvent>::of_node(bridge, 0),
            )?;
            Ok(OutputRef::of_node(bridge, 0))
        },
    )
    .expect("the shared schema connects to the native bridge");
    let mut compiled = Harness::build(
        1,
        TimeBase::new(1_000_000),
        GraphConfig::default(),
        |graph, source| {
            let node = graph.add(Pwm::new(config).expect("configures"));
            graph.connect(source.samples, Pwm::input(node))?;
            Ok(Pwm::events(node))
        },
    )
    .expect("the native producer builds");
    let mut waveform = PwmWaveform::new(&config, 1, Tick::ZERO);
    waveform.level(false, 5);
    waveform.cycles([(20, 7), (31, 11), (13, 3), (19, 8)]);
    let block = waveform.finish();
    interpreted.feed_with_gaps(&block, &[], &[1, 2, 5, 3]);
    compiled.feed_with_gaps(&block, &[], &[u64::MAX]);
    interpreted.end();
    compiled.end();
    assert_eq!(fault.expect("the bridge has a fault handle").get(), None);
    let events = |items: &[Item<PwmEvent>]| {
        items
            .iter()
            .filter_map(Item::event)
            .cloned()
            .collect::<Vec<_>>()
    };
    assert_eq!(events(&interpreted.items), events(&compiled.items));
    assert_eq!(events(&interpreted.items).len(), 3);
}

#[test]
fn the_output_import_must_match_the_schema_version() {
    let mut module = producer();
    module.imports[0].contract.schema.version = 2;
    let error = Program::load(&module).expect_err("the output version is unavailable");
    assert!(error.to_string().contains("decoder.output"), "{error}");
}
