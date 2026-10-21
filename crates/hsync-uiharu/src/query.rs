//! The decoder's answers in the capture's sample positions. The stores count ticks from the
//! first sample the capture ever held; the capture, and so the interface, from the first it
//! still has. Every position going in or out is shifted by the samples dropped meanwhile.

use std::ops::Range;

use hsync_core::SampleIndex;
use serde::Serialize;
use ts_rs::TS;
use uiharu_core::{Tick, TickRange};

use crate::roles::RolePart;
use crate::run::Decoder;
use crate::search::Needle;
use crate::store::{EventPage, EventRow, LaneBlock, Store, StreamView};

/// Where a stream stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct StreamProgress {
    /// Events held.
    pub events: u64,
    /// The sample before which nothing more will be reported.
    pub settled: SampleIndex,
    /// Whether the stream has ended.
    pub ended: bool,
}

/// An event a search found.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Hit {
    /// The event's row in its stream.
    pub row: u32,
    pub start: SampleIndex,
    pub end: SampleIndex,
}

fn shift_row(row: &mut EventRow, dropped: SampleIndex) {
    row.start = row.start.saturating_sub(dropped);
    row.end = row.end.saturating_sub(dropped);
}

fn shift_view(view: &mut StreamView, dropped: SampleIndex) {
    let annotations = view
        .annotations
        .iter_mut()
        .flatten()
        .flat_map(|lane| &mut lane.blocks);
    for block in view.blocks.iter_mut().chain(annotations) {
        match block {
            LaneBlock::Event(row) => shift_row(row, dropped),
            LaneBlock::Dense {
                start, end, parts, ..
            } => {
                *start = start.saturating_sub(dropped);
                *end = end.saturating_sub(dropped);
                for part in parts {
                    part.start = part.start.saturating_sub(dropped);
                    part.end = part.end.saturating_sub(dropped);
                }
            }
        }
    }
}

impl Decoder {
    fn with_store<R>(&self, stream: usize, f: impl FnOnce(&Store, SampleIndex) -> R) -> Option<R> {
        let dropped = self.dropped();
        let stream = self.streams().get(stream)?;
        Some(f(&stream.store(), dropped))
    }

    fn absolute(range: Range<SampleIndex>, dropped: SampleIndex) -> TickRange {
        TickRange::new(
            Tick(range.start.saturating_add(dropped)),
            Tick(range.end.max(range.start).saturating_add(dropped)),
        )
    }

    /// Where stream `stream` stands; `None` for a stream the decoder does not have.
    pub fn progress(&self, stream: usize) -> Option<StreamProgress> {
        self.with_store(stream, |store, dropped| StreamProgress {
            events: store.len() as u64,
            settled: store.settled().0.saturating_sub(dropped),
            ended: store.ended().is_some(),
        })
    }

    /// The lanes of stream `stream` over `range` at `samples_per_pixel`, at most `limit`
    /// blocks per lane, including any derived annotations.
    pub fn view(
        &self,
        stream: usize,
        range: Range<SampleIndex>,
        samples_per_pixel: f64,
        limit: usize,
    ) -> Option<StreamView> {
        self.with_store(stream, |store, dropped| {
            let mut view = store.view(Self::absolute(range, dropped), samples_per_pixel, limit);
            shift_view(&mut view, dropped);
            view
        })
    }

    /// `count` rows of stream `stream` from `offset`, of all rows or of those overlapping
    /// `scope`.
    pub fn page(
        &self,
        stream: usize,
        scope: Option<Range<SampleIndex>>,
        offset: usize,
        count: usize,
    ) -> Option<EventPage> {
        self.with_store(stream, |store, dropped| {
            let scope = scope.map(|scope| Self::absolute(scope, dropped));
            let mut page = store.page(scope, offset, count);
            for row in &mut page.rows {
                shift_row(row, dropped);
            }
            page
        })
    }

    /// Events of stream `stream` lying wholly inside `range`.
    pub fn inside(&self, stream: usize, range: Range<SampleIndex>) -> Option<usize> {
        self.with_store(stream, |store, dropped| {
            store.inside(Self::absolute(range, dropped))
        })
    }

    /// Position in stream `stream` of the first event reaching past `sample`.
    pub fn row_at(&self, stream: usize, sample: SampleIndex) -> Option<usize> {
        self.with_store(stream, |store, dropped| {
            store.row_at(Tick(sample.saturating_add(dropped)))
        })
    }

    /// The events of stream `stream` matching `needle`, in time order, at most `limit`;
    /// whether more were found.
    pub fn search(&self, stream: usize, needle: &Needle, limit: usize) -> Option<(Vec<Hit>, bool)> {
        self.with_store(stream, |store, dropped| {
            let (rows, truncated) = store.search(needle, limit);
            let hits = rows
                .into_iter()
                .filter_map(|row| {
                    let span = store.span(row)?;
                    Some(Hit {
                        row: row as u32,
                        start: span.start.0.saturating_sub(dropped),
                        end: span.end.0.saturating_sub(dropped),
                    })
                })
                .collect();
            (hits, truncated)
        })
    }

    /// The stretches of `range` where stream `stream` has events, in columns of
    /// `resolution` samples, each in the role covering most of it (see [`Store::coverage`]).
    pub fn coverage(
        &self,
        stream: usize,
        range: Range<SampleIndex>,
        resolution: u64,
    ) -> Option<Vec<RolePart>> {
        self.with_store(stream, |store, dropped| {
            store
                .coverage(Self::absolute(range, dropped), resolution)
                .into_iter()
                .map(|part| RolePart {
                    start: part.start.saturating_sub(dropped),
                    end: part.end.saturating_sub(dropped),
                    ..part
                })
                .collect()
        })
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use uiharu_core::SchemaId;

    use super::*;

    #[test]
    fn retained_state_intervals_shift_to_the_captures_remaining_samples() {
        let mut store = Store::for_stream(SchemaId::new("jtag.events", 1), Vec::new());
        store.push_event(
            TickRange::one(Tick(20)),
            &json!({ "kind": "state", "state": "shift-ir" }),
        );
        store.push_event(
            TickRange::new(Tick(110), Tick(120)),
            &json!({ "kind": "shift", "tdi": [3] }),
        );
        store.push_end(Tick(130));
        store.prune_before(Tick(100));
        let mut view = store.view(TickRange::new(Tick(100), Tick(130)), 1.0, 10);
        shift_view(&mut view, 100);
        assert!(
            matches!(&view.blocks[..], [LaneBlock::Event(row)] if (row.start, row.end) == (10, 20))
        );
        let states = &view.annotations.expect("the state lane")[0];
        assert!(
            matches!(&states.blocks[..], [LaneBlock::Event(row)] if (row.start, row.end) == (0, 30))
        );
    }
}
