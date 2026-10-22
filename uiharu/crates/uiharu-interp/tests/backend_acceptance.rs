// SPDX-License-Identifier: Apache-2.0

//! TypeScript → freshly compiled IR → generated Rust and interpreted bytecode,
//! with independent wire-value assertions and mixed execution under backpressure.
//! Prepare the ignored fixture IR and Rust with `pnpm dsl:generate` before Cargo tests.

#[path = "fixtures/backend/generated/lib.rs"]
mod generated;
// Generated modules import their shared types through the crate root, as production does.
pub use generated::{backend_contracts, backend_forward, backend_probe};

use std::path::PathBuf;
use std::sync::Arc;

use backend_contracts::{Ack, Clause, ProbeEvent, Version};
use backend_forward::{BackendForward, ForwardConfig};
use backend_probe::{BackendProbe, ProbeConfig};
use serde_json::{Value, json};
use uiharu_core::testing::{Harness, assert_block_invariant};
use uiharu_core::{
    GraphConfig, InputRef, Item, Levels, LogicBlock, LogicBlockBuilder, Node, OutputRef, Props,
    Tick, TickRange, TimeBase,
};
use uiharu_interp::{DynEvent, FromDyn, Interpreter, Program, ToDyn};
use uiharu_ir::Module;

fn program(name: &str) -> Arc<Program> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join(format!("tests/fixtures/backend/ir/{name}.json"));
    let contents = std::fs::read_to_string(&path).unwrap_or_else(|error| {
        panic!(
            "{} reads: {error}; run `pnpm dsl:generate` before Cargo tests",
            path.display()
        )
    });
    let module: Module = serde_json::from_str(&contents).expect("fixture parses");
    Arc::new(Program::load(&module).expect("source-generated fixture loads"))
}

#[derive(Clone, Copy, Debug)]
struct Mix {
    producer: bool,
    consumer: bool,
}

const MIXES: [Mix; 4] = [
    Mix {
        producer: false,
        consumer: false,
    },
    Mix {
        producer: false,
        consumer: true,
    },
    Mix {
        producer: true,
        consumer: false,
    },
    Mix {
        producer: true,
        consumer: true,
    },
];

fn pipeline(mix: Mix, config: &Value, graph: GraphConfig) -> Harness<DynEvent> {
    let raw_config = config;
    let config: ProbeConfig = serde_json::from_value(config.clone()).expect("native config parses");
    let producer = BackendProbe::new(config).expect("native config validates");
    let props = producer.spec().outputs[0].props.clone();
    Harness::build(1, TimeBase::new(1_000_000), graph, |g, source| {
        if !mix.producer && !mix.consumer {
            // A direct connection also proves both producers use precisely the same native
            // imported payload type, without a serde bridge hiding a duplicate definition.
            let producer = g.add(producer);
            g.connect(source.samples, BackendProbe::input(producer))?;
            let consumer =
                g.add(BackendForward::new(ForwardConfig::default()).expect("configures"));
            g.connect(
                BackendProbe::events(producer),
                BackendForward::input(consumer),
            )?;
            let bridge = g.add(ToDyn::<ProbeEvent>::new(props));
            g.connect(
                BackendForward::events(consumer),
                InputRef::<ProbeEvent>::of_node(bridge, 0),
            )?;
            return Ok(OutputRef::of_node(bridge, 0));
        }
        let output: OutputRef<DynEvent> = if mix.producer {
            let node =
                g.add(Interpreter::new(program("backend-probe"), raw_config).expect("configures"));
            g.connect(source.samples, Interpreter::logic_input(node))?;
            Interpreter::events(node)
        } else {
            let producer = g.add(producer);
            g.connect(source.samples, BackendProbe::input(producer))?;
            let bridge = g.add(ToDyn::<ProbeEvent>::new(props.clone()));
            g.connect(
                BackendProbe::events(producer),
                InputRef::<ProbeEvent>::of_node(bridge, 0),
            )?;
            OutputRef::of_node(bridge, 0)
        };
        if mix.consumer {
            let node = g
                .add(Interpreter::new(program("backend-forward"), &json!({})).expect("configures"));
            g.connect(output, Interpreter::bus_input(node))?;
            Ok(Interpreter::events(node))
        } else {
            let bridge = g.add(FromDyn::<ProbeEvent>::new(props.clone()));
            g.connect(output, InputRef::<DynEvent>::of_node(bridge, 0))?;
            let consumer =
                g.add(BackendForward::new(ForwardConfig::default()).expect("configures"));
            g.connect(
                OutputRef::<ProbeEvent>::of_node(bridge, 0),
                BackendForward::input(consumer),
            )?;
            let bridge = g.add(ToDyn::<ProbeEvent>::new(props));
            g.connect(
                BackendForward::events(consumer),
                InputRef::<ProbeEvent>::of_node(bridge, 0),
            )?;
            Ok(OutputRef::of_node(bridge, 0))
        }
    })
    .expect("shared schema connects")
}

fn waveform() -> LogicBlock {
    let mut builder = LogicBlockBuilder::new(Tick::ZERO, 1);
    for level in [0, 1, 0, 1, 0, 1] {
        builder.push_repeated(Levels(level), 20);
    }
    builder.finish().expect("sample block")
}

fn expected(config: &Value) -> Value {
    let ack = config.get("ack").cloned().unwrap_or(json!("ok"));
    let clause = config.get("clause").cloned().unwrap_or(json!(22));
    let fixed_sum: u64 = config
        .get("four")
        .and_then(Value::as_array)
        .map_or(10, |values| {
            values
                .iter()
                .map(|value| value.as_u64().expect("fixed byte"))
                .sum()
        });
    let last = config
        .get("five")
        .map_or(9, |values| values[4].as_u64().expect("fixed byte"));
    let code = match ack.as_str() {
        Some("ok") => 1,
        Some("wait") => 2,
        Some("fault") => 3,
        _ => 100 + ack["other"].as_u64().expect("payload byte"),
    };
    json!({
        "kind": "probe", "four": [1, 2, 3, 4], "five": [5, 6, 7, 8, 9],
        "optional": if config.get("enabled") == Some(&json!(false)) { Value::Null } else { json!([9,8,7,6]) },
        "address": "0xfedcba9876543212", "clause": clause, "constantClause": 45,
        "ack": ack, "constructed": { "other": 7 }, "unit": "wait",
        "version": config.get("version").cloned().unwrap_or(json!("1.2")),
        "fixedSum": fixed_sum, "last": last, "switchCount": 12, "rangeCast": 10, "optionalWidth": 75,
        "conditionalWidth": if config.get("enabled") == Some(&json!(false)) { 0 } else { 75 },
        "nestedWide": 1800, "castWide": 1_u64 << 40,
        "castBarrier": 220, "ternaryCast": if config.get("enabled") == Some(&json!(false)) {45} else {44}, "countWidth": 256, "contextualMask": 300, "compareWide": false,
        "literalTree": 21, "ackCode": code, "isOk": ack == json!("ok"),
        "clauseCode": if clause == json!(22) { 220 } else { 450 },
        "isC22": clause == json!(22),
    })
}

fn run(mix: Mix, config: &Value, gaps: &[TickRange]) -> Vec<Item<DynEvent>> {
    let mut harness = pipeline(
        mix,
        config,
        GraphConfig {
            queue_capacity: 1,
            budget: 1,
        },
    );
    harness.drain_on_full = 1;
    harness.feed_cut(&waveform(), gaps, &[1, 3, 7], &[0, 1, 0, 2]);
    harness.end();
    harness.assert_progress_promise();
    harness.without_progress()
}

#[test]
fn source_generated_and_interpreted_pipelines_preserve_native_wire_types_and_values() {
    let configs = [
        json!({}),
        json!({"ack":"wait", "clause":45, "version":"2.0", "enabled":false}),
        json!({"ack":"fault", "version":"foo-bar"}),
        json!({"ack":{"other":7}, "version":"foo_bar"}),
        json!({"version":"FooBar"}),
        json!({"version":"Self"}),
        json!({"four":[10,20,30,40], "five":[1,2,3,4,99]}),
    ];
    let gaps = [TickRange::new(Tick(35), Tick(65))];
    for config in configs {
        for gap in [&[][..], &gaps[..]] {
            let ticks: &[u64] = if gap.is_empty() {
                &[20, 40, 60, 80, 100]
            } else {
                &[20, 80, 100]
            };
            let reference = run(MIXES[0], &config, gap);
            let events: Vec<_> = reference.iter().filter_map(Item::event).collect();
            assert_eq!(events.len(), ticks.len());
            for (event, tick) in events.iter().zip(ticks) {
                assert_eq!(event.payload.0, expected(&config));
                assert_eq!(event.span, TickRange::one(Tick(*tick)));
                assert_eq!(event.settled, Tick(*tick));
            }
            assert_eq!(
                reference
                    .iter()
                    .filter(|item| matches!(item, Item::Gap(_)))
                    .count(),
                gap.len()
            );
            assert!(matches!(reference.last(), Some(Item::End(Tick(120)))));
            for mix in &MIXES[1..] {
                assert_eq!(run(*mix, &config, gap), reference, "{mix:?}: {config}");
            }
        }
    }
}

#[test]
fn all_backend_execution_combinations_are_block_and_backpressure_invariant() {
    let block = waveform();
    let gaps = [TickRange::new(Tick(35), Tick(65))];
    for mix in MIXES {
        assert_block_invariant(
            |graph| pipeline(mix, &json!({"ack":{"other":255}, "clause":45}), graph),
            &block,
            &gaps,
        );
    }
}

#[test]
fn wire_deserialization_rejects_wrong_fixed_lengths_and_unknown_enum_values() {
    let program = program("backend-probe");
    for config in [
        json!({"four":[1,2,3]}),
        json!({"four":[1,2,3,4,5]}),
        json!({"five":[1,2,3,4]}),
        json!({"clause":23}),
        json!({"clause":"c22"}),
        json!({"ack":{"other":256}}),
        json!({"ack":{"unknown":1}}),
        json!({"version":"1_2"}),
    ] {
        assert!(
            serde_json::from_value::<ProbeConfig>(config.clone()).is_err(),
            "native accepts {config}"
        );
        assert!(
            Interpreter::new(Arc::clone(&program), &config).is_err(),
            "interpreter accepts {config}"
        );
    }
    for wire in ["1.2", "2.0", "foo-bar", "foo_bar", "FooBar", "Self"] {
        let version: Version = serde_json::from_value(json!(wire)).expect("version parses");
        assert_eq!(
            serde_json::to_value(version).expect("serializes"),
            json!(wire)
        );
        assert_eq!(version.name(), wire);
    }
    assert_eq!(
        serde_json::to_value(Clause::C45).expect("serializes"),
        json!(45)
    );
    assert_eq!(
        serde_json::to_value(Ack::Other(7)).expect("serializes"),
        json!({"other":7})
    );
}

#[test]
fn numeric_enum_properties_and_validation_messages_use_wire_values() {
    let program = program("backend-probe");
    for (clause, version) in [(22, "1.2"), (45, "foo-bar")] {
        let config = json!({"clause": clause, "version": version});
        let native = BackendProbe::new(
            serde_json::from_value(config.clone()).expect("native config parses"),
        )
        .expect("native config validates");
        let interpreted =
            Interpreter::new(Arc::clone(&program), &config).expect("interpreted config validates");
        let expected = Props::new().with("clause", clause);
        assert_eq!(native.spec().outputs[0].props, expected);
        assert_eq!(interpreted.spec().outputs[0].props, expected);

        let rejected = json!({"channel": 1, "clause": clause, "version": version});
        let native = BackendProbe::new(
            serde_json::from_value(rejected.clone()).expect("native rejected config parses"),
        )
        .expect_err("the fixture validator rejects channel one");
        let interpreted = Interpreter::new(Arc::clone(&program), &rejected)
            .expect_err("the interpreted fixture validator rejects channel one");
        let expected = format!("clause {clause}; version {version}");
        assert_eq!(native.to_string(), expected);
        assert_eq!(interpreted.to_string(), expected);
    }
}
