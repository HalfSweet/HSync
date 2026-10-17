// SPDX-License-Identifier: Apache-2.0

//! The PWM decoder lowered by hand from `dsl/decoders/pwm.ts`: it reads, it round-trips and it
//! passes the checks; and the checks catch what they are there for.

use uiharu_ir::{BinaryOp, Expr, Function, Literal, Module, Op, Stmt, Type, TypeKind, validate};

fn pwm() -> Module {
    let json = include_str!("fixtures/pwm.json");
    serde_json::from_str(json).expect("the fixture parses")
}

fn method_mut<'a>(module: &'a mut Module, name: &str) -> &'a mut Function {
    module
        .decoder
        .methods
        .iter_mut()
        .find(|method| method.name == name)
        .expect("the method exists")
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
fn the_pwm_module_passes_the_checks() {
    let module = pwm();
    assert_eq!(validate(&module), Vec::new());
    assert_eq!(module.name, "pwm");
    assert_eq!(module.decoder.methods.len(), 3);
}

#[test]
fn the_pwm_module_round_trips_through_json() {
    let module = pwm();
    let json = serde_json::to_string_pretty(&module).expect("serializes");
    let back: Module = serde_json::from_str(&json).expect("parses again");
    assert_eq!(back, module);
}

#[test]
fn another_ir_version_is_refused() {
    let mut module = pwm();
    module.ir = 2;
    assert_reports(&module, "IR version 2 is not 1");
}

#[test]
fn collections_need_a_bound() {
    let mut module = pwm();
    if let TypeKind::Record { fields } = &mut module.types[2].kind {
        fields[0].ty = Type::Array {
            array: Box::new(Type::named("u8")),
            bound: 0,
        };
    }
    assert_reports(&module, "a collection has a bound of at least one item");
}

#[test]
fn unknown_types_are_named() {
    let mut module = pwm();
    module.functions[0].params[0].ty = Type::named("Polarities");
    assert_reports(&module, "type Polarities is not declared");
}

#[test]
fn a_logic_decoder_has_no_bus_operations() {
    let mut module = pwm();
    method_mut(&mut module, "run").body.insert(
        0,
        Stmt::Expr {
            expr: Expr::Await { op: Op::Item },
        },
    );
    assert_reports(&module, "item is a bus operation");
    let mut module = pwm();
    method_mut(&mut module, "run").body.insert(
        0,
        Stmt::Expr {
            expr: Expr::Intrinsic {
                name: "consumed".into(),
                args: Vec::new(),
            },
        },
    );
    assert_reports(&module, "consumed is for bus decoders");
}

#[test]
fn an_event_of_a_logic_decoder_states_its_provenance() {
    let mut module = pwm();
    method_mut(&mut module, "run").body.insert(
        0,
        Stmt::Expr {
            expr: Expr::Await {
                op: Op::Emit {
                    event: Box::new(Expr::Variant {
                        ty: "PwmEvent".into(),
                        kind: "cycle".into(),
                        fields: Default::default(),
                    }),
                    at: None,
                },
            },
        },
    );
    assert_reports(&module, "an event states its provenance");
}

#[test]
fn recursion_is_refused() {
    let mut module = pwm();
    module.functions[0].body.insert(
        0,
        Stmt::Expr {
            expr: Expr::Call {
                name: "isActive".into(),
                args: Vec::new(),
            },
        },
    );
    assert_reports(
        &module,
        "recursion is not allowed: fn:isActive → fn:isActive",
    );
}

#[test]
fn a_loop_waits_breaks_or_returns() {
    let mut module = pwm();
    method_mut(&mut module, "run").body.push(Stmt::Loop {
        label: None,
        body: vec![Stmt::Assign {
            target: Expr::SelfField {
                field: "leading".into(),
            },
            op: None,
            value: Expr::None,
        }],
    });
    assert_reports(
        &module,
        "a loop waits, breaks or returns in every iteration",
    );
    // A loop that breaks out of itself is fine; one whose only break belongs to a nested loop
    // is not.
    let mut module = pwm();
    method_mut(&mut module, "run").body.push(Stmt::Loop {
        label: Some("outer".into()),
        body: vec![Stmt::Loop {
            label: None,
            body: vec![Stmt::Break { label: None }],
        }],
    });
    assert_reports(
        &module,
        "a loop waits, breaks or returns in every iteration",
    );
    let mut module = pwm();
    method_mut(&mut module, "run").body.push(Stmt::Loop {
        label: Some("outer".into()),
        body: vec![Stmt::Loop {
            label: None,
            body: vec![Stmt::Break {
                label: Some("outer".into()),
            }],
        }],
    });
    assert_eq!(validate(&module), Vec::new());
}

#[test]
fn only_async_methods_await() {
    let mut module = pwm();
    method_mut(&mut module, "hold").body.insert(
        0,
        Stmt::Expr {
            expr: Expr::Await { op: Op::Sample },
        },
    );
    assert_reports(&module, "only an async method awaits");
}

#[test]
fn locals_must_be_in_scope() {
    let mut module = pwm();
    method_mut(&mut module, "hold").body.insert(
        0,
        Stmt::Expr {
            expr: Expr::Local {
                name: "nowhere".into(),
            },
        },
    );
    assert_reports(&module, "nowhere is not in scope");
    let mut module = pwm();
    method_mut(&mut module, "hold").body.insert(
        0,
        Stmt::Expr {
            expr: Expr::SelfField {
                field: "nowhere".into(),
            },
        },
    );
    assert_reports(&module, "self has no field nowhere");
}

#[test]
fn a_borrowed_field_keeps_self_out_of_reach() {
    let mut module = pwm();
    module.decoder.methods.push(Function {
        name: "touch".into(),
        is_async: false,
        params: Vec::new(),
        ret: None,
        body: vec![
            Stmt::LetRef {
                name: "leading".into(),
                place: Expr::SelfField {
                    field: "leading".into(),
                },
            },
            Stmt::Expr {
                expr: Expr::Method {
                    name: "hold".into(),
                    args: Vec::new(),
                },
            },
        ],
    });
    assert_reports(
        &module,
        "no method of self is called while a local borrows a field of self",
    );
}

#[test]
fn hooks_have_the_shape_their_kind_calls_for() {
    let mut module = pwm();
    method_mut(&mut module, "hold").ret = Some(Type::named("u64"));
    assert_reports(&module, "decoder.methods.hold: returns the wrong type");
    let mut module = pwm();
    method_mut(&mut module, "run").name = "go".into();
    assert_reports(&module, "a run method is required");
    let mut module = pwm();
    method_mut(&mut module, "run").is_async = false;
    assert_reports(&module, "decoder.methods.run: must be async");
}

#[test]
fn values_and_enums_resolve() {
    let mut module = pwm();
    method_mut(&mut module, "hold").body.insert(
        0,
        Stmt::Expr {
            expr: Expr::Enum {
                ty: "Polarity".into(),
                value: "sideways".into(),
            },
        },
    );
    assert_reports(&module, "Polarity has no variant sideways");
    let mut module = pwm();
    method_mut(&mut module, "hold").body.insert(
        0,
        Stmt::Expr {
            expr: Expr::Binary {
                op: BinaryOp::Add,
                left: Box::new(Expr::Lit {
                    value: Literal::Int(1),
                }),
                right: Box::new(Expr::Text {
                    value: "text".into(),
                }),
            },
        },
    );
    assert_reports(&module, "text is for messages");
}
