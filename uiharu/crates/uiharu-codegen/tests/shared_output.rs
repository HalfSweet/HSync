// SPDX-License-Identifier: Apache-2.0

//! A producer can publish a contract owned by another generated module without defining a
//! second Rust payload or a second `EventSchema` implementation.

use serde_json::json;
use uiharu_codegen::generate;
use uiharu_ir::Module;

// The hand-authored PWM fixture keeps these unit tests independent of generated files.
fn producer() -> Module {
    let mut module: Module =
        serde_json::from_str(include_str!("../../uiharu-ir/tests/fixtures/pwm.json"))
            .expect("the PWM module parses");
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
fn an_independent_producer_reuses_the_contract_owners_rust_payload() {
    let module = producer();
    assert_eq!(uiharu_ir::validate(&module), Vec::new());
    let rust = generate(&module).expect("an imported output contract generates");
    assert!(rust.contains("use crate::pwm::{PwmEvent};"), "{rust}");
    assert!(rust.contains("pub fn events(node: NodeId) -> OutputRef<PwmEvent>"));
    assert!(rust.contains("type Event = PwmEvent;"));
    assert!(rust.contains("PwmEvent::Cycle"));
    assert!(!rust.contains("pub enum PwmEvent"));
    assert!(!rust.contains("impl EventSchema for PwmEvent"));
}

#[test]
fn an_import_of_another_version_does_not_satisfy_the_output_contract() {
    let mut module = producer();
    module.imports[0].contract.schema.version = 2;
    assert!(generate(&module).is_err());
}
