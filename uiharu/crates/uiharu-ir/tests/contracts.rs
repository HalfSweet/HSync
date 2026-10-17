// SPDX-License-Identifier: Apache-2.0

//! A module of contracts alone, for a decoder written by hand, passes the checks without a
//! decoder; and the checks on a framing hold: a frame carries at least one kind of item, the
//! item kinds are events naming the frame, the normal ends are reasons.

use uiharu_ir::{
    Contract, EnumVariant, Field, Framing, IR_VERSION, Module, SchemaRef, Type, TypeDef, TypeKind,
    UnionVariant, validate,
};

fn field(name: &str, ty: &str) -> Field {
    Field {
        name: name.into(),
        ty: Type::named(ty),
        channel: false,
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
