// SPDX-License-Identifier: Apache-2.0

//! The reference modules and the comparison of an interpreted decoder's items with a compiled
//! decoder's.

#![allow(dead_code)]

use std::path::PathBuf;
use std::sync::Arc;

use serde::Serialize;
use uiharu_core::{Event, Item};
use uiharu_interp::{DynEvent, Program};
use uiharu_ir::Module;

/// The freshly compiled TS intermediate for `name`; prepare with `pnpm dsl:compile`.
pub fn module(name: &str) -> Module {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!("../../dsl/ir/{name}.json"));
    let json = std::fs::read_to_string(&path).unwrap_or_else(|error| {
        panic!(
            "{} reads: {error}; run `pnpm dsl:compile` before Cargo tests",
            path.display()
        )
    });
    serde_json::from_str(&json).expect("the module parses")
}

/// The module `name`, loaded.
pub fn program(name: &str) -> Arc<Program> {
    Arc::new(Program::load(&module(name)).expect("the module loads"))
}

/// The items of a compiled decoder as an interpreted one would emit them: the events as JSON.
pub fn as_dyn<T: Serialize + Clone>(items: &[Item<T>]) -> Vec<Item<DynEvent>> {
    items
        .iter()
        .map(|item| match item {
            Item::Event(event) => Item::Event(Event {
                seq: event.seq,
                span: event.span,
                settled: event.settled,
                payload: DynEvent(serde_json::to_value(&event.payload).expect("serializes")),
            }),
            Item::Gap(range) => Item::Gap(*range),
            Item::Progress(tick) => Item::Progress(*tick),
            Item::End(tick) => Item::End(*tick),
        })
        .collect()
}
