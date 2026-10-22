// SPDX-License-Identifier: Apache-2.0

//! Native contract representations affect types, expressions, patterns and defaults together.

use serde_json::{Value, json};
use uiharu_codegen::generate;
use uiharu_ir::Module;

fn module() -> Value {
    json!({
        "ir": 1, "name": "native-shapes",
        "types": [
            {"name": "Clause", "type": "numeric-enum", "variants": [
                {"name": "c22", "value": 22}, {"name": "c45", "value": 45}]},
            {"name": "Address", "type": "external-enum", "variants": [
                {"name": "broadcast"}, {"name": "device", "payload": "u16"}]},
            {"name": "Config", "type": "record", "fields": [
                {"name": "bytes", "type": {"fixedArray": "u8", "length": 2}},
                {"name": "rom", "type": "u64", "json": "hex-u64"},
                {"name": "clause", "type": "Clause"}, {"name": "address", "type": "Address"}]},
            {"name": "Event", "type": "union", "variants": [{"name": "data", "fields": [
                {"name": "rom", "type": "u64", "json": "hex-u64"}]}]}
        ],
        "contracts": [{"schema": {"name": "native.events", "version": 1}, "event": "Event"}],
        "consts": [{"name": "CLAUSE", "type": "Clause", "value": 22},
            {"name": "BROADCAST", "type": "Address", "value": "broadcast"}],
        "functions": [
            {"name": "bytes", "ret": {"fixedArray": "u8", "length": 2}, "body": [
                {"stmt": "return", "value": {"expr": "array", "items": [
                    {"expr": "lit", "value": 1}, {"expr": "lit", "value": 2}]}}]},
            {"name": "device", "ret": "Address", "body": [
                {"stmt": "return", "value": {"expr": "variant", "type": "Address", "kind": "device",
                    "fields": {"value": {"expr": "lit", "value": 7}}}}]},
            {"name": "pick", "params": [{"name": "address", "type": "Address"}], "ret": "u16", "body": [
                {"stmt": "match", "value": {"expr": "local", "name": "address"}, "arms": [
                    {"pattern": {"pattern": "variant", "name": "device", "binds": {"value": "n"}}, "body": [
                        {"stmt": "return", "value": {"expr": "local", "name": "n"}}]},
                    {"pattern": {"pattern": "default"}, "body": [{"stmt": "return", "value": {"expr": "lit", "value": 0}}]}]}]},
            {"name": "pickIf", "params": [{"name": "address", "type": "Address"}], "ret": "u16", "body": [
                {"stmt": "return", "value": {"expr": "if-variant", "value": {"expr": "local", "name": "address"},
                    "variant": "device", "binds": {"value": "n"}, "then": {"expr": "local", "name": "n"},
                    "else": {"expr": "lit", "value": 0}}}]},
            {"name": "message", "params": [{"name": "clause", "type": "Clause"}], "ret": "text", "body": [
                {"stmt": "return", "value": {"expr": "format", "parts": [{"expr": "local", "name": "clause"}]}}]}
        ],
        "decoder": {"name": "native-shapes", "category": "bus", "config": {"type": "Config", "defaults": {
            "bytes": [1, 2], "rom": "0xffffffffffffffff", "clause": 22, "address": {"device": 7}}},
            "input": {"kind": "logic"}, "output": {"schema": {"name": "native.events", "version": 1}},
            "props": [{"key": "clause", "value": {"expr": "field", "of": {"expr": "local", "name": "config"}, "name": "clause"}}],
            "methods": [{"name": "run", "async": true, "body": [{"stmt": "loop", "body": [
                {"stmt": "expr", "expr": {"expr": "await", "op": {"op": "sample"}}}]}]}]}
    })
}

fn generate_value(value: Value) -> Result<String, uiharu_codegen::GenError> {
    let module: Module = serde_json::from_value(value).expect("the fixture parses");
    generate(&module)
}

#[test]
fn native_types_keep_their_rust_and_wire_representations() {
    let rust = generate_value(module()).expect("native shapes generate");
    for expected in [
        "pub bytes: [u8; 2]",
        "#[serde(with = \"uiharu_core::hex_u64\")]",
        "serializer.serialize_u64(self.value())",
        "Device(u16)",
        "rom: 18446744073709551615",
        "clause: Clause::C22",
        "address: Address::Device(7)",
        "bytes: [1, 2]",
        "Address::Device(n)",
        ".value()",
        "pub const CLAUSE: Clause = Clause::C22;",
        "pub const BROADCAST: Address = Address::Broadcast;",
        "if let Address::Device(n)",
        "format!(\"{}\", clause.value())",
    ] {
        assert!(rust.contains(expected), "missing {expected:?} in {rust}");
    }
    assert!(!rust.contains("pub bytes: Vec"));
}

#[test]
fn fixed_arrays_require_exact_literal_and_default_lengths() {
    let mut value = module();
    value["functions"][0]["body"][0]["value"]["items"] = json!([{"expr": "lit", "value": 1}]);
    assert!(generate_value(value).is_err());
    let mut value = module();
    value["decoder"]["config"]["defaults"]["bytes"] = json!([1, 2, 3]);
    assert!(generate_value(value).is_err());
}

#[test]
fn fixed_arrays_do_not_silently_accept_bounded_array_variables() {
    let mut value = module();
    value["functions"][0]["params"] =
        json!([{"name": "input", "type": {"array": "u8", "bound": 2}}]);
    value["functions"][0]["body"][0]["value"] = json!({"expr": "local", "name": "input"});
    assert!(generate_value(value).is_err());
}

#[test]
fn external_unit_default_objects_follow_native_serde() {
    let mut value = module();
    value["decoder"]["config"]["defaults"]["address"] = json!({"broadcast": null});
    let rust = generate_value(value).expect("serde accepts null for an externally tagged unit");
    assert!(rust.contains("address: Address::Broadcast"));
}

#[test]
fn text_enum_messages_use_the_wire_name_as_the_interpreter_does() {
    let mut value = module();
    value["types"].as_array_mut().expect("types").push(json!({
        "name": "Mode", "type": "enum", "variants": ["active-high", "active-low"]
    }));
    let message = value["functions"]
        .as_array_mut()
        .expect("functions")
        .iter_mut()
        .find(|function| function["name"] == "message")
        .expect("message function");
    message["params"][0]["type"] = json!("Mode");
    let rust = generate_value(value).expect("text enum formatting generates");
    assert!(rust.contains("format!(\"{}\", clause.name())"));
}
