// SPDX-License-Identifier: Apache-2.0

//! The constructs of the IR the reference decoders do not use, on modules written for them:
//! timed reads, every loop and its labels, indexed places, records changed through a
//! parameter or a reference, batches, the integer arithmetic at its edges, a stream decoder,
//! the hooks, the faults and the refusals.

mod common;

use common::as_dyn;
use serde_json::{Value as Json, json};
use std::sync::Arc;
use uiharu_core::testing::Harness;
use uiharu_core::{
    GraphConfig, InputRef, Item, Levels, LogicBlockBuilder, Node, OutputRef, Tick, TickRange,
    TimeBase,
};
use uiharu_interp::{DynEvent, Interpreter, LoadError, Program, ToDyn};
use uiharu_ir::Module;
use uiharu_protocols::pwm::synth::PwmWaveform;
use uiharu_protocols::pwm::{Pwm, PwmConfig, PwmEvent};

// The IR, written short.

fn lit(n: u64) -> Json {
    json!({ "expr": "lit", "value": n })
}
fn flag(b: bool) -> Json {
    json!({ "expr": "lit", "value": b })
}
fn local(name: &str) -> Json {
    json!({ "expr": "local", "name": name })
}
fn this(field: &str) -> Json {
    json!({ "expr": "self", "field": field })
}
fn field(of: Json, name: &str) -> Json {
    json!({ "expr": "field", "of": of, "name": name })
}
fn index(of: Json, at: Json) -> Json {
    json!({ "expr": "index", "of": of, "at": at })
}
fn bin(op: &str, left: Json, right: Json) -> Json {
    json!({ "expr": "binary", "op": op, "left": left, "right": right })
}
fn un(op: &str, value: Json) -> Json {
    json!({ "expr": "unary", "op": op, "value": value })
}
fn cast(value: Json, to: &str) -> Json {
    json!({ "expr": "cast", "value": value, "to": to })
}
fn intrinsic(name: &str, args: Vec<Json>) -> Json {
    json!({ "expr": "intrinsic", "name": name, "args": args })
}
fn call(name: &str, args: Vec<Json>) -> Json {
    json!({ "expr": "call", "name": name, "args": args })
}
fn variant(ty: &str, kind: &str, fields: Json) -> Json {
    json!({ "expr": "variant", "type": ty, "kind": kind, "fields": fields })
}
fn record(ty: &str, fields: Json) -> Json {
    json!({ "expr": "record", "type": ty, "fields": fields })
}
fn array(items: Vec<Json>) -> Json {
    json!({ "expr": "array", "items": items })
}
fn awaited(op: Json) -> Json {
    json!({ "expr": "await", "op": op })
}
fn emit(event: Json, at: Json) -> Json {
    json!({ "stmt": "expr", "expr": awaited(json!({ "op": "emit", "event": event, "at": at })) })
}
fn let_(name: &str, value: Json) -> Json {
    json!({ "stmt": "let", "name": name, "value": value })
}
fn let_as(name: &str, ty: Json, value: Json) -> Json {
    json!({ "stmt": "let", "name": name, "mutable": true, "type": ty, "value": value })
}
fn let_ref(name: &str, place: Json) -> Json {
    json!({ "stmt": "let-ref", "name": name, "place": place })
}
fn assign(target: Json, value: Json) -> Json {
    json!({ "stmt": "assign", "target": target, "value": value })
}
fn assign_op(target: Json, op: &str, value: Json) -> Json {
    json!({ "stmt": "assign", "target": target, "op": op, "value": value })
}
fn ret(value: Json) -> Json {
    json!({ "stmt": "return", "value": value })
}
fn stmt(expr: Json) -> Json {
    json!({ "stmt": "expr", "expr": expr })
}
fn if_(cond: Json, then: Vec<Json>, otherwise: Vec<Json>) -> Json {
    json!({ "stmt": "if", "cond": cond, "then": then, "else": otherwise })
}
fn match_(value: Json, arms: Vec<Json>) -> Json {
    json!({ "stmt": "match", "value": value, "arms": arms })
}
fn arm_variant(name: &str, binds: Json, body: Vec<Json>) -> Json {
    json!({ "pattern": { "pattern": "variant", "name": name, "binds": binds }, "body": body })
}
fn arm_values(values: Vec<Json>, body: Vec<Json>) -> Json {
    json!({ "pattern": { "pattern": "values", "values": values }, "body": body })
}
fn arm_default(body: Vec<Json>) -> Json {
    json!({ "pattern": { "pattern": "default" }, "body": body })
}
fn loop_(label: Option<&str>, body: Vec<Json>) -> Json {
    json!({ "stmt": "loop", "label": label, "body": body })
}
fn while_(cond: Json, body: Vec<Json>) -> Json {
    json!({ "stmt": "while", "cond": cond, "body": body })
}
fn for_(var: &str, from: Json, to: Json, body: Vec<Json>) -> Json {
    json!({ "stmt": "for", "var": var, "from": from, "to": to, "body": body })
}
fn for_each(var: &str, array: Json, body: Vec<Json>) -> Json {
    json!({ "stmt": "for-each", "var": var, "array": array, "body": body })
}
fn brk(label: Option<&str>) -> Json {
    json!({ "stmt": "break", "label": label })
}
fn cont() -> Json {
    json!({ "stmt": "continue" })
}
fn one(tick: Json) -> Json {
    intrinsic("one", vec![tick])
}

fn load(module: Json) -> Result<Program, LoadError> {
    let module: Module = serde_json::from_value(module).expect("the module parses");
    Program::load(&module)
}

/// A logic decoder of every construct: `lab.events`.
fn lab() -> Json {
    let sum = variant(
        "LabEvent",
        "sum",
        json!({
            "value": local("value"),
            "each": local("each"),
            "ones": intrinsic("count-ones", vec![lit(255)]),
            "shifted": cast(bin("shl", local("value"), lit(5)), "u8"),
            "gone": bin("shl", local("small"), lit(8)),
            "div": bin("div", lit(7), lit(0)),
            "rem": bin("rem", lit(7), lit(0)),
            "sat": intrinsic("saturating-sub", vec![local("small"), lit(5)]),
            "muldiv": intrinsic("mul-div", vec![lit(1_000_000), lit(100), lit(200 * 115_200)]),
            "not8": un("bit-not", local("small")),
            "cast": cast(local("big"), "u8"),
            "whiles": local("j"),
            "labelled": local("lab"),
            "picked": call("pick", vec![field(this("config"), "mode"), local("t2")]),
            "wrapped": bin("add", local("small"), lit(250)),
            "wide": bin("mul", local("small"), lit(100)),
            "nested_wide": bin("add", bin("mul", local("small"), lit(100)), local("big")),
            "cast_wide": cast(bin("shl", lit(1), lit(40)), "u64"),
            "cast_barrier": cast(cast(bin("mul", local("small"), lit(100)), "u8"), "u64"),
            "count_width": bin("shl", lit(1), cast(lit(8), "u8")),
            "maxed": intrinsic("max", vec![local("small"), lit(3)]),
            "either": json!({ "expr": "if-variant", "value": local("t"), "variant": "quiet",
                "binds": { "tick": "q" }, "then": bin("eq", local("q"), local("t2")), "else": flag(false) }),
        }),
    );
    let quiet_arm = vec![
        emit(
            variant("LabEvent", "tick", json!({ "tick": local("t2") })),
            intrinsic("at", vec![local("t2")]),
        ),
        let_as("value", json!("u16"), lit(0)),
        for_(
            "i",
            lit(0),
            lit(3),
            vec![assign(
                local("value"),
                bin("add", local("value"), index(this("table"), local("i"))),
            )],
        ),
        let_as("each", json!("u16"), lit(0)),
        for_each(
            "item",
            this("table"),
            vec![assign_op(local("each"), "add", local("item"))],
        ),
        let_as("j", json!("u64"), lit(0)),
        while_(
            flag(true),
            vec![
                if_(bin("ge", local("j"), lit(2)), vec![brk(None)], vec![]),
                assign_op(local("j"), "add", lit(1)),
            ],
        ),
        let_as("lab", json!("u64"), lit(0)),
        loop_(
            Some("outer"),
            vec![loop_(
                None,
                vec![
                    assign_op(local("lab"), "add", lit(1)),
                    if_(bin("lt", local("lab"), lit(2)), vec![cont()], vec![]),
                    brk(Some("outer")),
                ],
            )],
        ),
        let_as("small", json!("u8"), lit(15)),
        let_as("big", json!("u16"), lit(300)),
        emit(sum, one(local("t2"))),
        assign(
            index(this("table"), lit(2)),
            bin("add", index(this("table"), lit(2)), lit(1)),
        ),
    ];
    let edge_arm = vec![
        let_ref("a", this("acc")),
        stmt(call("bump", vec![local("a"), lit(1)])),
        assign_op(this("steps"), "add", lit(1)),
        emit(
            variant(
                "LabEvent",
                "edge",
                json!({
                    "tick": local("tick"),
                    "high": intrinsic("levels-high", vec![local("lv"), field(this("config"), "channel")]),
                    "count": field(this("acc"), "count"),
                    "total": field(this("acc"), "total"),
                }),
            ),
            one(local("tick")),
        ),
        stmt(intrinsic(
            "batch-push",
            vec![this("batch"), cast(local("tick"), "u8"), one(local("tick"))],
        )),
        if_(
            intrinsic("batch-is-full", vec![this("batch")]),
            vec![
                let_("taken", intrinsic("batch-take", vec![this("batch")])),
                emit(
                    variant(
                        "LabEvent",
                        "items",
                        json!({
                            "bytes": field(local("taken"), "items"),
                            "left": intrinsic("batch-length", vec![this("batch")]),
                        }),
                    ),
                    intrinsic(
                        "unwrap-or",
                        vec![
                            field(local("taken"), "at"),
                            intrinsic("at", vec![local("tick")]),
                        ],
                    ),
                ),
            ],
            vec![],
        ),
    ];
    let run = vec![
        stmt(awaited(json!({ "op": "sample" }))),
        loop_(
            None,
            vec![
                let_(
                    "t",
                    awaited(json!({
                        "op": "next-change-before",
                        "mask": this("watch"),
                        "until": bin("add", intrinsic("position", vec![]), field(this("config"), "step")),
                    })),
                ),
                match_(
                    local("t"),
                    vec![
                        arm_variant("edge", json!({ "tick": "tick", "levels": "lv" }), edge_arm),
                        arm_variant("quiet", json!({ "tick": "t2" }), quiet_arm),
                    ],
                ),
            ],
        ),
    ];
    json!({
        "ir": 1,
        "name": "lab",
        "types": [
            { "name": "LabEvent", "type": "union", "variants": [
                { "name": "tick", "fields": [{ "name": "tick", "type": "u64" }] },
                { "name": "edge", "fields": [
                    { "name": "tick", "type": "u64" }, { "name": "high", "type": "bool" },
                    { "name": "count", "type": "u32" }, { "name": "total", "type": "u16" }] },
                { "name": "sum", "fields": [
                    { "name": "value", "type": "u16" }, { "name": "each", "type": "u16" },
                    { "name": "ones", "type": "u8" }, { "name": "shifted", "type": "u8" },
                    { "name": "gone", "type": "u8" }, { "name": "div", "type": "u64" },
                    { "name": "rem", "type": "u64" }, { "name": "sat", "type": "u8" },
                    { "name": "muldiv", "type": "u64" }, { "name": "not8", "type": "u8" },
                    { "name": "cast", "type": "u8" }, { "name": "whiles", "type": "u64" },
                    { "name": "labelled", "type": "u64" }, { "name": "picked", "type": "u64" },
                    { "name": "wrapped", "type": "u8" }, { "name": "wide", "type": "u16" },
                    { "name": "nested_wide", "type": "u16" }, { "name": "cast_wide", "type": "u64" },
                    { "name": "cast_barrier", "type": "u64" }, { "name": "count_width", "type": "u16" },
                    { "name": "maxed", "type": "u8" }, { "name": "either", "type": "bool" }] },
                { "name": "items", "fields": [
                    { "name": "bytes", "type": { "array": "u8", "bound": 4 } },
                    { "name": "left", "type": "u32" }] },
                { "name": "done", "fields": [
                    { "name": "steps", "type": "u64" }, { "name": "progress", "type": "u64" }] }
            ] },
            { "name": "Acc", "type": "record", "fields": [
                { "name": "total", "type": "u16" }, { "name": "count", "type": "u32" }] },
            { "name": "Mode", "type": "enum", "variants": ["a", "b"] },
            { "name": "LabConfig", "type": "record", "fields": [
                { "name": "channel", "type": "u8", "channel": true },
                { "name": "mode", "type": "Mode" },
                { "name": "step", "type": "u64" }] }
        ],
        "contracts": [{ "schema": { "name": "lab.events", "version": 1 }, "event": "LabEvent" }],
        "functions": [
            { "name": "bump", "params": [
                { "name": "acc", "type": "Acc", "mutable": true }, { "name": "by", "type": "u16" }],
              "body": [
                assign_op(field(local("acc"), "total"), "add", local("by")),
                assign_op(field(local("acc"), "count"), "add", lit(1)) ] },
            { "name": "validate", "params": [{ "name": "config", "type": "LabConfig" }],
              "ret": { "option": "text" },
              "body": [ json!({ "stmt": "return", "value": { "expr": "ternary",
                  "cond": bin("eq", field(local("config"), "step"), lit(0)),
                  "then": { "expr": "format", "parts": [
                      { "expr": "text", "value": "a step of " },
                      field(local("config"), "step"),
                      { "expr": "text", "value": " samples; it must be above zero" }] },
                  "else": { "expr": "none" } } }) ] },
            { "name": "pick", "params": [{ "name": "mode", "type": "Mode" }, { "name": "x", "type": "u64" }],
              "ret": "u64",
              "body": [ match_(local("mode"), vec![
                  arm_values(vec![json!("a")], vec![ret(local("x"))]),
                  arm_default(vec![ret(bin("mul", local("x"), lit(2)))]) ]) ] }
        ],
        "decoder": {
            "name": "lab",
            "category": "line",
            "config": { "type": "LabConfig", "defaults": { "channel": 0, "mode": "a", "step": 10 }, "validate": "validate" },
            "input": { "kind": "logic" },
            "requires": [{ "key": "channels", "requirement": "at-least",
                "values": [bin("add", field(local("config"), "channel"), lit(1))] }],
            "output": { "schema": { "name": "lab.events", "version": 1 } },
            "props": [
                { "key": "mode", "value": field(local("config"), "mode") },
                { "key": "step", "value": field(local("config"), "step") }],
            "state": [
                { "name": "watch", "type": "mask", "init": intrinsic("mask", vec![field(this("config"), "channel")]), "readonly": true },
                { "name": "acc", "type": "Acc", "init": record("Acc", json!({ "total": lit(0), "count": lit(0) })) },
                { "name": "batch", "type": { "batch": "u8", "bound": 4 }, "init": intrinsic("batch", vec![]) },
                { "name": "table", "type": { "array": "u8", "bound": 3 }, "init": array(vec![lit(1), lit(2), lit(3)]) },
                { "name": "progressed", "type": "u64", "init": lit(0) },
                { "name": "steps", "type": "u64", "init": lit(0) }
            ],
            "methods": [
                { "name": "hold", "ret": { "option": "u64" }, "body": [ret(json!({ "expr": "none" }))] },
                { "name": "onProgress", "params": [{ "name": "tick", "type": "u64" }],
                  "body": [assign(this("progressed"), local("tick"))] },
                { "name": "onEnd", "async": true, "params": [{ "name": "tick", "type": "u64" }],
                  "body": [emit(variant("LabEvent", "done", json!({ "steps": this("steps"), "progress": this("progressed") })),
                                intrinsic("at", vec![local("tick")]))] },
                { "name": "run", "async": true, "body": run }
            ]
        }
    })
}

/// Channel 0 low and high for 20 samples in turn: edges at 20, 40, 60 and 80.
fn toggles() -> uiharu_core::LogicBlock {
    let mut builder = LogicBlockBuilder::new(Tick::ZERO, 2);
    for i in 0..5u64 {
        builder.push_repeated(Levels(i & 1), 20);
    }
    builder.finish().expect("a block")
}

fn run_lab(config: Json) -> (Vec<Item<DynEvent>>, Option<String>) {
    let program = Arc::new(load(lab()).expect("loads"));
    let mut fault = None;
    let mut harness: Harness<DynEvent> = Harness::build(
        2,
        TimeBase::new(1_000_000),
        GraphConfig::default(),
        |graph, source| {
            let node = Interpreter::new(program, &config).expect("configures");
            fault = Some(node.fault_handle());
            let node = graph.add(node);
            graph.connect(source.samples, Interpreter::logic_input(node))?;
            Ok(Interpreter::events(node))
        },
    )
    .expect("builds");
    // Two blocks, each read before the next arrives, so that progress is announced.
    harness.feed_cut(&toggles(), &[], &[50], &[0]);
    harness.end();
    (harness.items, fault.and_then(|handle| handle.get()))
}

#[test]
fn every_construct_runs() {
    let (items, fault) = run_lab(json!({ "mode": "b" }));
    assert_eq!(fault, None);
    let events: Vec<(Json, TickRange, Tick)> = items
        .iter()
        .filter_map(Item::event)
        .map(|event| (event.payload.0.clone(), event.span, event.settled))
        .collect();
    let of = |kind: &str| -> Vec<&(Json, TickRange, Tick)> {
        events
            .iter()
            .filter(|(e, _, _)| e["kind"] == kind)
            .collect()
    };
    // Edges at 20, 40, 60 and 80, each bumping the accumulator through the reference.
    let edges = of("edge");
    assert_eq!(edges.len(), 4);
    for (n, (event, span, settled)) in edges.iter().enumerate() {
        let tick = 20 * (n as u64 + 1);
        assert_eq!(event["tick"], tick);
        assert_eq!(event["high"], n % 2 == 0);
        assert_eq!(event["count"], n as u64 + 1);
        assert_eq!(event["total"], n as u64 + 1);
        assert_eq!(*span, TickRange::one(Tick(tick)));
        assert_eq!(*settled, Tick(tick));
    }
    // The batch goes out at its fourth byte, placed over its items.
    let items = of("items");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].0["bytes"], json!([20, 40, 60, 80]));
    assert_eq!(items[0].0["left"], 0);
    assert_eq!(items[0].1, TickRange::new(Tick(20), Tick(81)));
    assert_eq!(items[0].2, Tick(80));
    // Deadlines go by in order, doubled by `pick` in mode b.
    let ticks: Vec<u64> = of("tick")
        .iter()
        .map(|(e, _, _)| e["tick"].as_u64().expect("a tick"))
        .collect();
    assert!(ticks.windows(2).all(|pair| pair[0] < pair[1]), "{ticks:?}");
    assert_eq!(ticks[0], 10);
    // The arithmetic, at the width of each field.
    let sums = of("sum");
    assert!(sums.len() >= 2);
    let (first, _, _) = sums[0];
    assert_eq!(first["value"], 6);
    assert_eq!(first["each"], 6);
    assert_eq!(first["ones"], 8);
    assert_eq!(first["shifted"], (6u16 << 5) as u8);
    assert_eq!(first["gone"], 0);
    assert_eq!(first["div"], 0);
    assert_eq!(first["rem"], 0);
    assert_eq!(first["sat"], 10);
    assert_eq!(first["muldiv"], 4);
    assert_eq!(first["not8"], 240);
    assert_eq!(first["cast"], 44);
    assert_eq!(first["whiles"], 2);
    assert_eq!(first["labelled"], 2);
    assert_eq!(first["picked"], 20);
    assert_eq!(first["wrapped"], 9);
    assert_eq!(first["wide"], 1500);
    assert_eq!(first["nested_wide"], 1800);
    assert_eq!(first["cast_wide"], 1_u64 << 40);
    assert_eq!(first["cast_barrier"], 220);
    assert_eq!(first["count_width"], 256);
    assert_eq!(first["maxed"], 15);
    assert_eq!(first["either"], true);
    // The table's last item grows by one at every deadline.
    assert_eq!(sums[1].0["value"], 7);
    assert_eq!(sums[1].0["each"], 7);
    // The end hook sees the state and the last progress.
    let done = of("done");
    assert_eq!(done.len(), 1);
    assert_eq!(done[0].0["steps"], 4);
    assert_eq!(done[0].0["progress"], 100);
    assert_eq!(done[0].2, Tick(100));
    assert_eq!(items.len(), 1);
    assert!(matches!(
        run_lab(json!({})).0.last(),
        Some(Item::End(Tick(100)))
    ));
}

#[test]
fn the_configuration_is_checked_and_declared() {
    let program = Arc::new(load(lab()).expect("loads"));
    let error = Interpreter::new(Arc::clone(&program), &json!({ "step": 0 })).expect_err("refused");
    assert_eq!(
        error.to_string(),
        "a step of 0 samples; it must be above zero"
    );
    let error =
        Interpreter::new(Arc::clone(&program), &json!({ "mode": "c" })).expect_err("refused");
    assert_eq!(
        error.to_string(),
        "unknown variant `c`, expected `a` or `b`"
    );
    let node =
        Interpreter::new(program, &json!({ "channel": 1, "mode": "b" })).expect("configures");
    let spec = node.spec();
    assert_eq!(
        spec.outputs[0].props,
        uiharu_core::Props::new().with("mode", "b").with("step", 10)
    );
    assert_eq!(
        spec.inputs[0].requires,
        uiharu_core::Requires::new().at_least("channels", 2)
    );
    assert_eq!(spec.outputs[0].schema.to_string(), "lab.events@1");
}

#[test]
fn a_hook_that_changes_the_state_outside_the_body_is_a_fault() {
    let mut module = lab();
    module["decoder"]["methods"][0]["body"] = json!([
        assign(this("steps"), lit(99)),
        ret(json!({ "expr": "none" }))
    ]);
    let program = Arc::new(load(module).expect("loads"));
    let mut fault = None;
    let mut harness: Harness<DynEvent> = Harness::build(
        2,
        TimeBase::new(1_000_000),
        GraphConfig::default(),
        |graph, source| {
            let node = Interpreter::new(program, &json!({})).expect("configures");
            fault = Some(node.fault_handle());
            let node = graph.add(node);
            graph.connect(source.samples, Interpreter::logic_input(node))?;
            Ok(Interpreter::events(node))
        },
    )
    .expect("builds");
    harness.run_all(&toggles());
    assert_eq!(
        fault.and_then(|handle| handle.get()).as_deref(),
        Some("the decoder's state is changed outside its body")
    );
    // The stream still ends.
    assert!(matches!(harness.items.last(), Some(Item::End(Tick(100)))));
}

#[test]
fn modules_the_backend_would_not_emit_are_refused() {
    let mut module = lab();
    module["decoder"]["methods"][3]["body"] = json!([
        let_("k", json!({ "expr": "tag", "value": this("acc") })),
        loop_(None, vec![stmt(awaited(json!({ "op": "sample" })))])
    ]);
    let error = load(module).expect_err("refused");
    assert_eq!(
        error.to_string(),
        "decoder.methods[3].body[0]: a tag is read through a match"
    );

    let mut module = lab();
    module["decoder"]["methods"][3]["body"] = json!([
        let_("x", lit(5)),
        loop_(None, vec![stmt(awaited(json!({ "op": "sample" })))])
    ]);
    assert_eq!(
        load(module).expect_err("refused").to_string(),
        "decoder.methods[3].body[0]: the width of x is unknown"
    );

    let mut module = lab();
    module["decoder"]["methods"][3]["body"] = json!([
        let_as("x", json!("u8"), lit(0)),
        let_as("y", json!("u16"), lit(0)),
        assign(local("x"), local("y")),
        loop_(None, vec![stmt(awaited(json!({ "op": "sample" })))])
    ]);
    assert_eq!(
        load(module).expect_err("refused").to_string(),
        "decoder.methods[3].body[2]: a u16 does not fit a u8"
    );

    let mut module = lab();
    module["decoder"]["state"][0]["type"] = json!("Nope");
    assert!(matches!(load(module), Err(LoadError::Invalid(_))));
}

/// A stream decoder over `pwm.events`: counts the cycles and keeps the longest.
fn tally() -> Json {
    let pwm_event = json!({ "name": "PwmEvent", "type": "union", "variants": [
        { "name": "cycle", "fields": [
            { "name": "periodTicks", "type": "u64" }, { "name": "activeTicks", "type": "u64" }] }] });
    json!({
        "ir": 1,
        "name": "tally",
        "types": [
            { "name": "TallyEvent", "type": "union", "variants": [
                { "name": "total", "fields": [
                    { "name": "cycles", "type": "u32" }, { "name": "longest", "type": "u64" },
                    { "name": "from", "type": "u64" }, { "name": "to", "type": "u64" },
                    { "name": "settled", "type": "u64" }] }] },
            { "name": "TallyConfig", "type": "record", "fields": [] }
        ],
        "contracts": [{ "schema": { "name": "tally.events", "version": 1 }, "event": "TallyEvent" }],
        "imports": [{ "module": "pwm", "types": [pwm_event],
            "contract": { "schema": { "name": "pwm.events", "version": 1 }, "event": "PwmEvent" } }],
        "decoder": {
            "name": "tally",
            "category": "device",
            "config": { "type": "TallyConfig", "defaults": {} },
            "input": { "kind": "stream", "schema": { "name": "pwm.events", "version": 1 } },
            "output": { "schema": { "name": "tally.events", "version": 1 } },
            "state": [
                { "name": "cycles", "type": "u32", "init": lit(0) },
                { "name": "longest", "type": "u64", "init": lit(0) }
            ],
            "methods": [
                { "name": "run", "async": true, "body": [loop_(None, vec![
                    let_("r", awaited(json!({ "op": "next" }))),
                    match_(field(local("r"), "event"), vec![
                        arm_variant("cycle", json!({ "periodTicks": "p" }), vec![
                            assign_op(this("cycles"), "add", lit(1)),
                            assign(this("longest"), intrinsic("max", vec![this("longest"), local("p")])),
                            emit(variant("TallyEvent", "total", json!({
                                "cycles": this("cycles"),
                                "longest": this("longest"),
                                "from": field(field(field(local("r"), "at"), "span"), "start"),
                                "to": field(field(field(local("r"), "at"), "span"), "end"),
                                "settled": field(field(local("r"), "at"), "settled"),
                            })), field(local("r"), "at"))
                        ])
                    ])
                ])] }
            ]
        }
    })
}

#[test]
fn a_stream_decoder_reads_a_compiled_decoder_through_a_bridge() {
    let program = Arc::new(load(tally()).expect("loads"));
    let config = PwmConfig::default();
    let mut waveform = PwmWaveform::new(&config, 2, Tick::ZERO);
    waveform.level(false, 10);
    waveform.cycles([(100, 25), (60, 50), (80, 10)]);
    waveform.level(true, 5);
    let block = waveform.finish();
    let mut harness: Harness<DynEvent> = Harness::build(
        2,
        TimeBase::new(1_000_000),
        GraphConfig::default(),
        |graph, source| {
            let pwm = graph.add(Pwm::new(config).expect("configures"));
            graph.connect(source.samples, Pwm::input(pwm))?;
            let bridge = graph.add(ToDyn::<PwmEvent>::new(
                Pwm::new(config).expect("configures").spec().outputs[0]
                    .props
                    .clone(),
            ));
            graph.connect(Pwm::events(pwm), InputRef::<PwmEvent>::of_node(bridge, 0))?;
            let node = graph.add(Interpreter::new(program, &json!({})).expect("configures"));
            graph.connect(
                OutputRef::<DynEvent>::of_node(bridge, 0),
                Interpreter::bus_input(node),
            )?;
            Ok(Interpreter::events(node))
        },
    )
    .expect("builds");
    harness.run_all(&block);
    let events = harness.events();
    assert_eq!(events.len(), 3);
    assert_eq!(
        events[2].payload.0,
        json!({ "kind": "total", "cycles": 3, "longest": 100, "from": 170, "to": 250, "settled": 250 })
    );
    assert_eq!(events[2].span, TickRange::new(Tick(170), Tick(250)));
    assert_eq!(events[2].settled, Tick(250));
    assert!(matches!(harness.items.last(), Some(Item::End(Tick(255)))));
    let _ = as_dyn::<PwmEvent>(&[]);
}
