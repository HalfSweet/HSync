// SPDX-License-Identifier: Apache-2.0

//! A module of contracts alone, for a decoder written by hand, passes the checks without a
//! decoder; and the checks on a framing hold: a frame carries at least one kind of item, the
//! item kinds are events naming the frame, the normal ends are reasons. Roles name every kind
//! of event, and their error conditions name fields with values the fields can take.

use std::collections::BTreeMap;

use uiharu_ir::{
    Contract, EnumVariant, EventRole, Field, Framing, IR_VERSION, Module, Role, SchemaRef, Type,
    TypeDef, TypeKind, UnionVariant, validate,
};

fn field(name: &str, ty: &str) -> Field {
    Field {
        name: name.into(),
        ty: Type::named(ty),
        channel: false,
        json: None,
    }
}

fn role(role: Role) -> EventRole {
    EventRole {
        role,
        error_when: Vec::new(),
    }
}

fn variant(name: &str, fields: Vec<Field>) -> UnionVariant {
    UnionVariant {
        name: name.into(),
        fields,
    }
}

/// A contract in the shape of `i2c.events`: a frame carries addresses and data bytes; a STOP
/// ends it normally, and so does the START of the next transaction.
fn i2c() -> Module {
    Module {
        ir: IR_VERSION,
        name: "i2c".into(),
        types: vec![
            TypeDef {
                name: "EndReason".into(),
                kind: TypeKind::Enum {
                    variants: ["stop", "restart", "aborted"]
                        .map(|name| EnumVariant(name.into()))
                        .to_vec(),
                },
            },
            TypeDef {
                name: "I2cEvent".into(),
                kind: TypeKind::Union {
                    variants: vec![
                        variant("begin", vec![field("tx", "u64"), field("partial", "bool")]),
                        variant(
                            "address",
                            vec![
                                field("tx", "u64"),
                                field("address", "u16"),
                                field("read", "bool"),
                                field("ack", "bool"),
                            ],
                        ),
                        variant("data", vec![field("tx", "u64"), field("byte", "u8")]),
                        variant(
                            "end",
                            vec![field("tx", "u64"), field("reason", "EndReason")],
                        ),
                    ],
                },
            },
        ],
        contracts: vec![Contract {
            schema: SchemaRef {
                name: "i2c.events".into(),
                version: 1,
            },
            event: "I2cEvent".into(),
            framing: Some(Framing {
                begin: "begin".into(),
                items: vec!["address".into(), "data".into()],
                end: "end".into(),
                id: "tx".into(),
                partial: Some("partial".into()),
                reason: "reason".into(),
                normal: vec!["stop".into(), "restart".into()],
                unframed: None,
            }),
            roles: BTreeMap::from([
                ("begin".into(), role(Role::Framing)),
                (
                    "address".into(),
                    EventRole {
                        role: Role::Address,
                        error_when: vec![BTreeMap::from([("ack".into(), false.into())])],
                    },
                ),
                ("data".into(), role(Role::Data)),
                ("end".into(), role(Role::Framing)),
            ]),
        }],
        imports: Vec::new(),
        consts: Vec::new(),
        functions: Vec::new(),
        decoder: None,
    }
}

fn framing_mut(module: &mut Module) -> &mut Framing {
    module.contracts[0]
        .framing
        .as_mut()
        .expect("a framed contract")
}

fn messages(module: &Module) -> Vec<String> {
    validate(module)
        .into_iter()
        .map(|diagnostic| diagnostic.to_string())
        .collect()
}

fn assert_reports(module: &Module, needle: &str) {
    let found = messages(module);
    assert!(
        found.iter().any(|message| message.contains(needle)),
        "expected a diagnostic containing {needle:?}, got {found:#?}"
    );
}

#[test]
fn a_module_of_contracts_passes_without_a_decoder() {
    let module = i2c();
    assert_eq!(validate(&module), Vec::new());
    let json = serde_json::to_value(&module).expect("serializes");
    assert!(
        json.get("decoder").is_none(),
        "no decoder is written: {json}"
    );
    let back: Module = serde_json::from_value(json).expect("parses again");
    assert_eq!(back, module);
}

#[test]
fn a_module_declares_a_decoder_or_a_contract() {
    let mut module = i2c();
    module.contracts.clear();
    assert_reports(&module, "a module declares a decoder or a contract");
}

#[test]
fn an_import_and_local_contract_cannot_disagree_about_one_schema_identity() {
    let mut module = i2c();
    let mut imported = module.contracts[0].clone();
    imported.roles.clear();
    module.imports.push(
        serde_json::from_value(serde_json::json!({
            "module": "other-i2c", "types": [], "contract": imported
        }))
        .expect("the import parses"),
    );
    assert_reports(
        &module,
        "i2c.events@1 has conflicting contract declarations",
    );
    module.imports[0].contract.schema.version = 2;
    assert_eq!(validate(&module), Vec::new());
}

#[test]
fn repeated_identical_contract_metadata_is_unambiguous() {
    let mut module = i2c();
    module.imports.push(
        serde_json::from_value(serde_json::json!({
            "module": "other-i2c", "types": [], "contract": module.contracts[0]
        }))
        .expect("the import parses"),
    );
    assert_eq!(validate(&module), Vec::new());
}

#[test]
fn a_frame_carries_at_least_one_kind_of_item() {
    let mut module = i2c();
    framing_mut(&mut module).items.clear();
    assert_reports(
        &module,
        "framing.items: a frame carries at least one kind of item",
    );
}

#[test]
fn the_item_kinds_are_events_naming_the_frame() {
    let mut module = i2c();
    framing_mut(&mut module).items.push("ack".into());
    assert_reports(&module, "framing.items: ack is not a variant");

    let mut module = i2c();
    if let TypeKind::Union { variants } = &mut module.types[1].kind {
        variants.push(variant("noise", vec![field("bits", "u8")]));
    }
    framing_mut(&mut module).items.push("noise".into());
    assert_reports(&module, "framing.id: noise has no field tx");
}

#[test]
fn the_normal_ends_are_reasons() {
    let mut module = i2c();
    framing_mut(&mut module).normal.push("timeout".into());
    assert_reports(&module, "framing.normal: timeout is not a reason");

    let mut module = i2c();
    framing_mut(&mut module).normal.clear();
    assert_reports(&module, "framing.normal: a frame has a normal end");
}

fn roles_mut(module: &mut Module) -> &mut BTreeMap<String, EventRole> {
    &mut module.contracts[0].roles
}

#[test]
fn every_kind_of_event_has_a_role() {
    let mut module = i2c();
    roles_mut(&mut module).remove("data");
    assert_reports(&module, "roles: data has no role");
}

#[test]
fn roles_name_kinds_of_event() {
    let mut module = i2c();
    roles_mut(&mut module).insert("stop".into(), role(Role::Framing));
    assert_reports(&module, "roles: stop is not a variant");
}

#[test]
fn a_module_without_roles_still_passes() {
    let mut module = i2c();
    roles_mut(&mut module).clear();
    assert_eq!(validate(&module), Vec::new());
}

#[test]
fn error_conditions_name_fields_of_the_kind() {
    let mut module = i2c();
    let address = roles_mut(&mut module).get_mut("address").expect("a role");
    address.error_when = vec![BTreeMap::from([("nack".into(), true.into())])];
    assert_reports(
        &module,
        "roles.address.errorWhen: address has no field nack",
    );
}

#[test]
fn error_conditions_take_values_their_fields_can_have() {
    let mut module = i2c();
    let roles = roles_mut(&mut module);
    roles.get_mut("address").expect("a role").error_when =
        vec![BTreeMap::from([("ack".into(), "no".into())])];
    roles.get_mut("end").expect("a role").error_when =
        vec![BTreeMap::from([("reason".into(), "aborted".into())])];
    let found = messages(&module);
    assert!(
        found
            .iter()
            .any(|message| message.contains("roles.address.errorWhen: ack is never \"no\"")),
        "{found:#?}"
    );
    assert!(
        !found.iter().any(|message| message.contains("roles.end")),
        "an enum field takes the name of a variant: {found:#?}"
    );
}

#[test]
fn an_event_that_meets_a_condition_is_an_error() {
    let module = i2c();
    let address = &module.contracts[0].roles["address"];
    let acked = serde_json::json!({ "kind": "address", "tx": 1, "address": 80, "ack": true });
    let refused = serde_json::json!({ "kind": "address", "tx": 1, "address": 80, "ack": false });
    assert_eq!(address.of(&acked), Role::Address);
    assert_eq!(address.of(&refused), Role::Error);
}
