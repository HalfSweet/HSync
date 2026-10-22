// SPDX-License-Identifier: Apache-2.0

//! Native payload representations remain explicit and checked in the IR.

use serde_json::{Value, json};
use uiharu_ir::{Module, validate};

fn module() -> Value {
    json!({
        "ir": 1, "name": "native-shapes",
        "types": [
            {"name": "Clause", "type": "numeric-enum", "variants": [
                {"name": "c22", "value": 22}, {"name": "c45", "value": 45}]},
            {"name": "Address", "type": "external-enum", "variants": [
                {"name": "broadcast"}, {"name": "device", "payload": "u16"}]},
            {"name": "Event", "type": "union", "variants": [{"name": "data", "fields": [
                {"name": "bytes", "type": {"fixedArray": "u8", "length": 4}},
                {"name": "rom", "type": "u64", "json": "hex-u64"},
                {"name": "clause", "type": "Clause"}, {"name": "address", "type": "Address"}]}]}
        ],
        "contracts": [{"schema": {"name": "native.events", "version": 1}, "event": "Event"}],
        "functions": [{"name": "device", "params": [], "ret": "Address", "body": [
            {"stmt": "return", "value": {"expr": "variant", "type": "Address", "kind": "device",
                "fields": {"value": {"expr": "lit", "value": 7}}}}]}]
    })
}

fn diagnostics(value: Value) -> Vec<String> {
    let module: Module = serde_json::from_value(value).expect("the module parses");
    validate(&module).iter().map(ToString::to_string).collect()
}

#[test]
fn native_payload_representations_round_trip_and_validate() {
    let original = module();
    let module: Module = serde_json::from_value(original.clone()).expect("the module parses");
    assert_eq!(validate(&module), Vec::new());
    let encoded = serde_json::to_value(module).expect("the module serializes");
    assert_eq!(encoded["types"], original["types"]);
}

#[test]
fn fixed_arrays_stay_within_the_native_serde_length_capability() {
    for length in [0, 33] {
        let mut value = module();
        value["types"][2]["variants"][0]["fields"][0]["type"]["length"] = json!(length);
        let messages = diagnostics(value);
        assert!(
            messages
                .iter()
                .any(|message| message.contains("native array lengths supported by serde")),
            "{messages:?}"
        );
    }
}

#[test]
fn roles_match_numeric_enum_wire_values_and_hex_field_strings() {
    let mut value = module();
    value["contracts"][0]["roles"] = json!({ "data": { "role": "data", "errorWhen": [
        {"clause": 22}, {"rom": "0xffffffffffffffff"}
    ] }});
    assert_eq!(diagnostics(value.clone()), Vec::<String>::new());
    value["contracts"][0]["roles"]["data"]["errorWhen"][0]["clause"] = json!("c22");
    let messages = diagnostics(value);
    assert!(
        messages
            .iter()
            .any(|message| message.contains("clause is never")),
        "{messages:?}"
    );
}

#[test]
fn numeric_enum_wire_values_and_logical_names_are_unique() {
    for replacement in [
        json!({"name": "other", "value": 22}),
        json!({"name": "c22", "value": 45}),
    ] {
        let mut value = module();
        value["types"][0]["variants"][1] = replacement;
        let messages = diagnostics(value);
        assert!(
            messages
                .iter()
                .any(|message| message.contains("declared twice")),
            "{messages:?}"
        );
    }
}

#[test]
fn hex_serialization_only_marks_u64_fields() {
    let mut value = module();
    value["types"][2]["variants"][0]["fields"][1]["type"] = json!("u32");
    let messages = diagnostics(value);
    assert!(
        messages
            .iter()
            .any(|message| message.contains("hex-u64 requires a u64 field")),
        "{messages:?}"
    );
}

#[test]
fn external_enum_constructors_require_the_single_payload_field() {
    for fields in [json!({}), json!({"other": {"expr": "lit", "value": 7}})] {
        let mut value = module();
        value["functions"][0]["body"][0]["value"]["fields"] = fields;
        let messages = diagnostics(value);
        assert!(
            messages
                .iter()
                .any(|message| message.contains("requires exactly the value payload")),
            "{messages:?}"
        );
    }
    let mut value = module();
    value["functions"][0]["body"][0]["value"]["kind"] = json!("broadcast");
    let messages = diagnostics(value);
    assert!(
        messages
            .iter()
            .any(|message| message.contains("unit variant takes no payload")),
        "{messages:?}"
    );
}
