// SPDX-License-Identifier: Apache-2.0
//! Shared comparison helpers for the SPI device acceptance vectors.
#![allow(dead_code)]
use serde::Serialize;
use std::sync::Arc;
use uiharu_core::testing::Harness;
use uiharu_core::{Event, GraphConfig, Item, LogicBlock, TickRange, TimeBase};
use uiharu_interp::{DynEvent, Interpreter, Program};
pub fn program(name: &str) -> Arc<Program> {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join(format!("../../dsl/ir/{name}.json"));
    let contents = std::fs::read_to_string(path).expect("IR exists");
    let module = serde_json::from_str(&contents).expect("IR parses");
    Arc::new(Program::load(&module).expect("IR validates"))
}
pub fn as_dyn<T: Serialize>(items: &[Item<T>]) -> Vec<Item<DynEvent>> {
    items
        .iter()
        .map(|item| match item {
            Item::Event(event) => Item::Event(Event {
                seq: event.seq,
                span: event.span,
                settled: event.settled,
                payload: DynEvent(serde_json::to_value(&event.payload).expect("valid test vector")),
            }),
            Item::Gap(range) => Item::Gap(*range),
            Item::Progress(tick) => Item::Progress(*tick),
            Item::End(tick) => Item::End(*tick),
        })
        .collect()
}
pub fn assert_items<T: Serialize>(
    compiled: &[Item<T>],
    interpreted: &[Item<DynEvent>],
    name: &str,
) {
    let compiled = as_dyn(compiled);
    // A bridge adds a queue and may coalesce progress at different await points.
    // The progress promise must hold on both streams; all other items, including
    // exact event provenance/sequence, gaps and end, must match.
    uiharu_core::testing::assert_progress_promise(&compiled);
    uiharu_core::testing::assert_progress_promise(interpreted);
    assert_eq!(
        uiharu_core::testing::without_progress(&compiled),
        uiharu_core::testing::without_progress(interpreted),
        "compiled/interpreted {name}"
    );
}
pub fn logic_parity<T: Serialize>(
    name: &str,
    config: serde_json::Value,
    g: GraphConfig,
    block: &LogicBlock,
    gaps: &[TickRange],
    sizes: &[u64],
    compiled: &[Item<T>],
) {
    let mut h = Harness::<DynEvent>::build(8, TimeBase::new(1_000_000), g, |graph, source| {
        let node = graph.add(Interpreter::new(program(name), &config).expect("interpreted config"));
        graph.connect(source.samples, Interpreter::logic_input(node))?;
        Ok(Interpreter::events(node))
    })
    .expect("interpreted graph");
    h.feed_with_gaps(block, gaps, sizes);
    h.end();
    assert_items(compiled, &h.items, name)
}
