//! Duration annotations derived from JTAG state transitions, separate from their provenance.

use serde_json::{Value, json};
use uiharu_core::TickRange;

use crate::store::EventRow;

#[derive(Debug)]
struct State {
    start: u64,
    name: usize,
}

/// A small, time-ordered index of state transitions. Names are interned, and queries bisect
/// this index rather than visiting all events before the viewport.
#[derive(Debug, Default)]
pub(crate) struct StateIndex {
    states: Vec<State>,
    names: Vec<String>,
}

impl StateIndex {
    pub(crate) fn push(&mut self, start: u64, payload: &Value) {
        let Some(name) = payload.get("state").and_then(Value::as_str) else {
            return;
        };
        let name = self
            .names
            .iter()
            .position(|held| held == name)
            .unwrap_or_else(|| {
                self.names.push(name.to_owned());
                self.names.len() - 1
            });
        let position = self.states.partition_point(|state| state.start <= start);
        self.states.insert(position, State { start, name });
    }

    fn end(&self, index: usize, horizon: u64, gaps: &[TickRange]) -> u64 {
        let state = &self.states[index];
        let end = self.states.get(index + 1).map_or_else(
            || horizon.max(state.start.saturating_add(1)),
            |next| next.start,
        );
        // A gap ends the known state. It does not resume after the missing samples;
        // a later transition must establish a state again.
        let gap = gaps.partition_point(|gap| gap.end.0 <= state.start);
        gaps.get(gap).map_or(end, |gap| end.min(gap.start.0))
    }

    pub(crate) fn intervals<'a>(
        &'a self,
        range: TickRange,
        horizon: u64,
        gaps: &'a [TickRange],
    ) -> impl Iterator<Item = (usize, u64, u64, usize)> + 'a {
        let first = self
            .states
            .partition_point(|state| state.start <= range.start.0)
            .saturating_sub(1);
        self.states[first..]
            .iter()
            .enumerate()
            .take_while(move |(_, state)| state.start < range.end.0)
            .filter_map(move |(offset, state)| {
                let index = first + offset;
                let end = self.end(index, horizon, gaps);
                let continuity = gaps.partition_point(|gap| gap.end.0 <= state.start);
                (end > range.start.0 && end > state.start).then_some((
                    index,
                    state.start,
                    end,
                    continuity,
                ))
            })
    }

    pub(crate) fn row(&self, index: usize, start: u64, end: u64) -> EventRow {
        let state = &self.states[index];
        EventRow {
            index: index as u32,
            start,
            end,
            kind: "state".into(),
            payload: json!({ "kind": "state", "state": self.names[state.name] }),
        }
    }

    pub(crate) fn prune_before(&mut self, tick: u64, horizon: u64, gaps: &[TickRange]) {
        let next = self.states.partition_point(|state| state.start < tick);
        let keep = match next.checked_sub(1) {
            Some(previous) if self.end(previous, horizon, gaps) > tick => previous,
            _ => next,
        };
        self.states.drain(..keep);
    }
}
