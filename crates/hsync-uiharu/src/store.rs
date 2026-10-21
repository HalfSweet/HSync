//! Keeping the events of one stream and answering the interface's questions about them.

use serde::Serialize;
use serde_json::Value;
use ts_rs::TS;
use uiharu_core::{SchemaId, Tick, TickRange};

use crate::roles::{EventRole, KindRoles, RolePainter, RolePart, ScanRegisters};
use crate::search::Needle;
use crate::states::StateIndex;

/// One event: where it lies and where its payload is in the text.
#[derive(Debug, Clone, Copy)]
struct Entry {
    start: u64,
    end: u64,
    offset: usize,
    len: u32,
    kind: u16,
    role: EventRole,
}

impl Entry {
    /// The first tick after the event, counting an event without length as one tick wide, so
    /// that a mark at one tick is found by the queries covering that tick.
    fn reach(&self) -> u64 {
        self.end.max(self.start + 1)
    }

    fn overlaps(&self, range: TickRange) -> bool {
        self.start < range.end.0 && self.reach() > range.start.0
    }
}

/// The events of one stream, in time order (by start, then by arrival), with their payloads
/// kept as compact JSON text.
#[derive(Debug, Default)]
pub struct Store {
    entries: Vec<Entry>,
    /// The greatest reach among the entries up to each position: non-decreasing, so that
    /// the first entry reaching past a tick is found by bisection.
    max_reach: Vec<u64>,
    text: Vec<u8>,
    kinds: Vec<String>,
    gaps: Vec<TickRange>,
    settled: u64,
    ended: Option<u64>,
    states: Option<StateIndex>,
    roles: KindRoles,
    scans: ScanRegisters,
}

/// An event as the interface reads it.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct EventRow {
    /// Its position in the stream's time order, or within a derived annotation lane.
    pub index: u32,
    pub start: u64,
    pub end: u64,
    /// The contract's kind of event.
    pub kind: String,
    /// What the event is, from its kind and, for an error condition, its fields.
    pub role: EventRole,
    /// The event as the contract serializes it.
    #[ts(type = "Record<string, unknown>")]
    pub payload: Value,
}

/// Something drawn on a lane.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(tag = "type", rename_all = "camelCase")]
#[ts(export)]
pub enum LaneBlock {
    /// One event.
    Event(EventRow),
    /// Events too narrow to tell apart at this scale, packed together: an error is packed
    /// only with other errors. `parts` gives the role covering most of every two pixels.
    Dense {
        start: u64,
        end: u64,
        count: u32,
        parts: Vec<RolePart>,
    },
}

/// A lane over a viewport.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct StreamView {
    pub blocks: Vec<LaneBlock>,
    /// More items lay in the viewport than were returned on this or an annotation lane.
    pub truncated: bool,
    /// Additional presentation lanes, without changing the stream's raw events.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub annotations: Option<Vec<AnnotationLane>>,
}

/// A presentation lane derived from a stream's events.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AnnotationLane {
    /// A stable presentation identifier; JTAG's TAP state intervals use `states`.
    pub id: String,
    /// The annotations overlapping the requested window, packed at its resolution.
    pub blocks: Vec<LaneBlock>,
    /// More items lay in the viewport than were returned on this lane.
    pub truncated: bool,
}

/// Part of a stream's rows.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct EventPage {
    /// Rows in the requested scope.
    pub total: u32,
    /// Position of the first row returned within the scope.
    pub offset: u32,
    pub rows: Vec<EventRow>,
}

#[derive(Clone, Copy)]
struct LaneSpan {
    index: usize,
    start: u64,
    end: u64,
    /// Different runs of known samples must not pack across a gap.
    continuity: usize,
    role: EventRole,
}

struct LaneRun {
    first: LaneSpan,
    end: u64,
    count: u32,
    painter: RolePainter,
}

impl LaneRun {
    fn new(span: LaneSpan, reach: u64, unit: u64) -> Self {
        let mut painter = RolePainter::new(unit);
        painter.add(span.start, reach, span.role);
        Self {
            first: span,
            end: reach,
            count: 1,
            painter,
        }
    }
}

/// Packs only spans first: parsing a payload is unnecessary for an event in a dense run.
/// Errors pack only with errors, so that a run of other events never hides one.
fn pack_lane(
    spans: impl Iterator<Item = LaneSpan>,
    samples_per_pixel: f64,
    limit: usize,
    row: impl Fn(LaneSpan) -> EventRow,
) -> StreamView {
    let unit = (samples_per_pixel * 2.0).max(1.0) as u64;
    let mut view = StreamView {
        blocks: Vec::new(),
        truncated: false,
        annotations: None,
    };
    let mut run: Option<LaneRun> = None;
    let flush = |run: Option<LaneRun>, view: &mut StreamView| match run {
        Some(run) if run.count == 1 => view.blocks.push(LaneBlock::Event(row(run.first))),
        Some(run) => view.blocks.push(LaneBlock::Dense {
            start: run.first.start,
            end: run.end,
            count: run.count,
            parts: run.painter.finish(run.first.start, run.end),
        }),
        None => {}
    };
    for span in spans {
        if view.blocks.len() >= limit {
            view.truncated = true;
            return view;
        }
        let reach = span.end.max(span.start.saturating_add(1));
        if reach - span.start >= unit {
            flush(run.take(), &mut view);
            if view.blocks.len() >= limit {
                view.truncated = true;
                return view;
            }
            view.blocks.push(LaneBlock::Event(row(span)));
            continue;
        }
        let error = span.role == EventRole::Error;
        match &mut run {
            Some(run)
                if span.continuity == run.first.continuity
                    && error == (run.first.role == EventRole::Error)
                    && span.start <= run.end.saturating_add(unit) =>
            {
                run.end = run.end.max(reach);
                run.count = run.count.saturating_add(1);
                run.painter.add(span.start, reach, span.role);
            }
            _ => {
                flush(run.take(), &mut view);
                if view.blocks.len() >= limit {
                    view.truncated = true;
                    return view;
                }
                run = Some(LaneRun::new(span, reach, unit));
            }
        }
    }
    flush(run, &mut view);
    view
}

impl Store {
    /// An empty store.
    pub fn new() -> Self {
        Self::default()
    }

    /// A store for a stream of `schema` whose kinds of event are `events`, as the manifest
    /// of its decoder lists them with their roles.
    pub(crate) fn for_stream(schema: SchemaId, events: Vec<uiharu_registry::EventKind>) -> Self {
        Self {
            states: (schema.name == "jtag.events").then(StateIndex::default),
            roles: KindRoles::new(events),
            ..Self::default()
        }
    }

    /// Events held.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether no event is held.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The tick before which nothing more will be reported.
    pub fn settled(&self) -> Tick {
        Tick(self.settled)
    }

    /// Where the stream ended, once it has.
    pub fn ended(&self) -> Option<Tick> {
        self.ended.map(Tick)
    }

    /// The ticks samples were missing for, in order.
    pub fn gaps(&self) -> &[TickRange] {
        &self.gaps
    }

    /// Bytes of payload text held.
    pub fn text_bytes(&self) -> usize {
        self.text.len()
    }

    /// Adds an event.
    pub fn push_event(&mut self, span: TickRange, payload: &Value) {
        let kind = payload
            .get("kind")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if kind == "state"
            && let Some(states) = &mut self.states
        {
            states.push(span.start.0, payload);
        }
        let mut role = self.roles.of(kind, payload);
        if self.states.is_some() {
            role = self.scans.role(kind, payload, role);
        }
        let kind = match self.kinds.iter().position(|known| known == kind) {
            Some(index) => index,
            None => {
                self.kinds.push(kind.to_string());
                self.kinds.len() - 1
            }
        };
        let offset = self.text.len();
        // A contract's events are plain data; serializing them cannot fail.
        serde_json::to_writer(&mut self.text, payload).unwrap_or_default();
        let entry = Entry {
            start: span.start.0,
            end: span.end.0,
            offset,
            len: u32::try_from(self.text.len() - offset).unwrap_or(u32::MAX),
            kind: u16::try_from(kind).unwrap_or(u16::MAX),
            role,
        };
        let position = self
            .entries
            .partition_point(|held| held.start <= entry.start);
        self.entries.insert(position, entry);
        self.max_reach.truncate(position);
        self.extend_reach(position);
    }

    /// Recomputes the reach from `from` on, after `max_reach` was cut there.
    fn extend_reach(&mut self, from: usize) {
        let mut reach = from.checked_sub(1).map_or(0, |last| self.max_reach[last]);
        for entry in &self.entries[from..] {
            reach = reach.max(entry.reach());
            self.max_reach.push(reach);
        }
    }

    /// Notes samples that were missing.
    pub fn push_gap(&mut self, gap: TickRange) {
        match self.gaps.last_mut() {
            Some(last) if last.end >= gap.start => last.end = last.end.max(gap.end),
            _ => self.gaps.push(gap),
        }
        self.settled = self.settled.max(gap.end.0);
    }

    /// Notes that nothing more will be reported before `tick`.
    pub fn push_progress(&mut self, tick: Tick) {
        self.settled = self.settled.max(tick.0);
    }

    /// Notes that the stream ended at `tick`.
    pub fn push_end(&mut self, tick: Tick) {
        self.settled = self.settled.max(tick.0);
        self.ended = Some(tick.0);
    }

    /// Forgets the events and gaps lying wholly before `tick`.
    pub fn prune_before(&mut self, tick: Tick) {
        if let Some(states) = &mut self.states {
            states.prune_before(tick.0, self.ended.unwrap_or(u64::MAX), &self.gaps);
        }
        let mut text = Vec::new();
        let mut entries = Vec::with_capacity(self.entries.len());
        for entry in &self.entries {
            if entry.reach() <= tick.0 {
                continue;
            }
            let offset = text.len();
            text.extend_from_slice(&self.text[entry.offset..entry.offset + entry.len as usize]);
            entries.push(Entry { offset, ..*entry });
        }
        self.entries = entries;
        self.text = text;
        self.max_reach.clear();
        self.extend_reach(0);
        self.gaps.retain(|gap| gap.end.0 > tick.0);
    }

    /// The span of event `index`.
    pub fn span(&self, index: usize) -> Option<TickRange> {
        self.entries
            .get(index)
            .map(|entry| TickRange::new(Tick(entry.start), Tick(entry.end)))
    }

    /// Event `index` as a row.
    pub fn row(&self, index: usize) -> Option<EventRow> {
        let entry = self.entries.get(index)?;
        Some(EventRow {
            index: index as u32,
            start: entry.start,
            end: entry.end,
            kind: self.kinds[usize::from(entry.kind)].clone(),
            role: entry.role,
            payload: serde_json::from_slice(self.payload_text(entry)).unwrap_or(Value::Null),
        })
    }

    fn payload_text(&self, entry: &Entry) -> &[u8] {
        &self.text[entry.offset..entry.offset + entry.len as usize]
    }

    /// The positions of the events overlapping `range`, in time order.
    pub fn overlapping(&self, range: TickRange) -> impl Iterator<Item = usize> + '_ {
        let first = self
            .max_reach
            .partition_point(|&reach| reach <= range.start.0);
        self.entries[first..]
            .iter()
            .enumerate()
            .take_while(move |(_, entry)| entry.start < range.end.0)
            .filter(move |(_, entry)| entry.overlaps(range))
            .map(move |(offset, _)| first + offset)
    }

    /// Events lying wholly inside `range`.
    pub fn inside(&self, range: TickRange) -> usize {
        self.overlapping(range)
            .filter(|&index| {
                let entry = &self.entries[index];
                entry.start >= range.start.0 && entry.reach() <= range.end.0
            })
            .count()
    }

    /// Position of the first event reaching past `tick`: the one holding it, or the next.
    pub fn row_at(&self, tick: Tick) -> usize {
        self.max_reach.partition_point(|&reach| reach <= tick.0)
    }

    /// The lane over `range` at `samples_per_pixel`: every event wide enough to be told
    /// apart on its own, runs of narrower ones packed into dense items; at most `limit`
    /// items per lane. JTAG state transitions appear as a separate duration annotation lane.
    pub fn view(&self, range: TickRange, samples_per_pixel: f64, limit: usize) -> StreamView {
        let spans = self.overlapping(range).filter_map(|index| {
            let entry = &self.entries[index];
            (self.states.is_none() || self.kinds[usize::from(entry.kind)] != "state").then_some(
                LaneSpan {
                    index,
                    start: entry.start,
                    end: entry.end,
                    continuity: 0,
                    role: entry.role,
                },
            )
        });
        let mut view = pack_lane(spans, samples_per_pixel, limit, |span| {
            self.row(span.index).expect("the row is held")
        });
        if let Some(states) = &self.states {
            let spans = states
                .intervals(range, self.ended.unwrap_or(self.settled), &self.gaps)
                .map(|(index, start, end, continuity)| LaneSpan {
                    index,
                    start,
                    end,
                    continuity,
                    role: states.role(index),
                });
            let annotations = pack_lane(spans, samples_per_pixel, limit, |span| {
                states.row(span.index, span.start, span.end)
            });
            view.truncated |= annotations.truncated;
            view.annotations = Some(vec![AnnotationLane {
                id: "states".into(),
                blocks: annotations.blocks,
                truncated: annotations.truncated,
            }]);
        }
        view
    }

    /// `count` rows from `offset`, of all rows or of those overlapping `scope`.
    pub fn page(&self, scope: Option<TickRange>, offset: usize, count: usize) -> EventPage {
        let indices: Vec<usize> = match scope {
            Some(scope) => self.overlapping(scope).collect(),
            None => (0..self.entries.len()).collect(),
        };
        let begin = offset.min(indices.len());
        let finish = begin.saturating_add(count).min(indices.len());
        EventPage {
            total: indices.len() as u32,
            offset: begin as u32,
            rows: indices[begin..finish]
                .iter()
                .map(|&index| self.row(index).expect("the row is held"))
                .collect(),
        }
    }

    /// The stretches of `range` holding events, in columns of `resolution` ticks: each
    /// column in the role covering most of it, or an error's when one lies there, and
    /// neighbouring columns of one role joined.
    pub fn coverage(&self, range: TickRange, resolution: u64) -> Vec<RolePart> {
        let mut painter = RolePainter::new(resolution);
        for index in self.overlapping(range) {
            let entry = &self.entries[index];
            painter.add(entry.start, entry.reach(), entry.role);
        }
        painter.finish(range.start.0, range.end.0)
    }

    /// The positions of the events matching `needle`, in time order, at most `limit` of
    /// them; whether more were found.
    pub fn search(&self, needle: &Needle, limit: usize) -> (Vec<usize>, bool) {
        let mut hits = Vec::new();
        for (index, entry) in self.entries.iter().enumerate() {
            if needle.matches(self.payload_text(entry)) {
                if hits.len() >= limit {
                    return (hits, true);
                }
                hits.push(index);
            }
        }
        (hits, false)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn range(start: u64, end: u64) -> TickRange {
        TickRange::new(Tick(start), Tick(end))
    }

    /// Frames at 10-tick steps, 8 ticks wide, with values `0..count`.
    fn frames(count: u64) -> Store {
        let mut store = Store::new();
        for value in 0..count {
            store.push_event(
                range(value * 10, value * 10 + 8),
                &json!({ "kind": "frame", "value": value }),
            );
        }
        store
    }

    #[test]
    fn events_are_kept_in_time_order_whatever_their_arrival() {
        let mut store = Store::new();
        store.push_event(range(100, 180), &json!({ "kind": "begin", "tx": 1 }));
        store.push_event(range(100, 120), &json!({ "kind": "address", "tx": 1 }));
        store.push_event(range(120, 140), &json!({ "kind": "data", "tx": 1 }));
        store.push_event(range(50, 60), &json!({ "kind": "late" }));
        let kinds: Vec<String> = (0..store.len())
            .map(|index| store.row(index).expect("a row").kind)
            .collect();
        assert_eq!(kinds, ["late", "begin", "address", "data"]);
        assert_eq!(store.max_reach, [60, 180, 180, 180]);
        assert_eq!(
            store.row(1),
            Some(EventRow {
                index: 1,
                start: 100,
                end: 180,
                kind: "begin".into(),
                role: EventRole::Data,
                payload: json!({ "kind": "begin", "tx": 1 }),
            })
        );
    }

    fn kind(name: &str, role: uiharu_registry::Role) -> uiharu_registry::EventKind {
        uiharu_registry::EventKind {
            kind: name.into(),
            role,
            fields: Vec::new(),
            error_when: Vec::new(),
        }
    }

    #[test]
    fn events_take_the_role_of_their_kind_and_errors_their_conditions() {
        let i2c = uiharu_registry::manifests()
            .iter()
            .find(|manifest| manifest.name == "i2c")
            .expect("the I²C manifest");
        let mut store = Store::for_stream(SchemaId::new("i2c.events", 1), i2c.events.clone());
        let address = |ack: bool| json!({ "kind": "address", "tx": 1, "address": 80, "ack": ack });
        store.push_event(range(0, 9), &address(true));
        store.push_event(
            range(10, 19),
            &json!({ "kind": "data", "tx": 1, "ack": false }),
        );
        store.push_event(range(20, 29), &address(false));
        store.push_event(range(30, 31), &json!({ "kind": "unheard-of" }));
        let roles: Vec<EventRole> = (0..store.len())
            .map(|index| store.row(index).expect("a row").role)
            .collect();
        assert_eq!(
            roles,
            [
                EventRole::Address,
                EventRole::Data,
                EventRole::Error,
                EventRole::Data
            ]
        );
    }

    #[test]
    fn overlap_queries_find_long_events_and_marks() {
        let mut store = Store::new();
        store.push_event(range(0, 1000), &json!({ "kind": "long" }));
        store.push_event(range(10, 20), &json!({ "kind": "short" }));
        store.push_event(range(500, 500), &json!({ "kind": "mark" }));
        store.push_event(range(2000, 2010), &json!({ "kind": "far" }));
        let overlapping = |start, end| store.overlapping(range(start, end)).collect::<Vec<_>>();
        assert_eq!(overlapping(900, 950), [0]);
        assert_eq!(overlapping(15, 16), [0, 1]);
        assert_eq!(overlapping(500, 501), [0, 2]);
        assert_eq!(overlapping(499, 500), [0]);
        assert_eq!(overlapping(1000, 2000), [] as [usize; 0]);
        assert_eq!(overlapping(1000, 2001), [3]);
        assert_eq!(store.inside(range(0, 1000)), 3);
        assert_eq!(store.inside(range(10, 600)), 2);
        assert_eq!(store.row_at(Tick(0)), 0);
        assert_eq!(store.row_at(Tick(1000)), 3);
        assert_eq!(store.row_at(Tick(3000)), 4);
    }

    #[test]
    fn narrow_events_pack_into_dense_items() {
        let store = frames(100);
        // Each frame is 8 ticks: at 10 ticks per pixel, all pack together.
        let view = store.view(range(0, 1000), 10.0, 100);
        assert_eq!(
            view.blocks,
            [LaneBlock::Dense {
                start: 0,
                end: 998,
                count: 100,
                parts: vec![RolePart {
                    start: 0,
                    end: 998,
                    role: EventRole::Data
                }],
            }]
        );
        // At one tick per pixel every frame stands on its own.
        let view = store.view(range(0, 35), 1.0, 100);
        assert_eq!(view.blocks.len(), 4);
        assert!(matches!(&view.blocks[3], LaneBlock::Event(row) if row.index == 3));
        assert!(!view.truncated);
        // A lone narrow event is drawn as itself; runs with a wide gap are split.
        let mut store = frames(3);
        store.push_event(range(500, 508), &json!({ "kind": "frame", "value": 50 }));
        let view = store.view(range(0, 1000), 10.0, 100);
        assert_eq!(view.blocks.len(), 2);
        assert!(matches!(&view.blocks[0], LaneBlock::Dense { count: 3, .. }));
        assert!(matches!(&view.blocks[1], LaneBlock::Event(row) if row.start == 500));
        let view = store.view(range(0, 1000), 1.0, 2);
        assert_eq!(view.blocks.len(), 2);
        assert!(view.truncated);
    }

    #[test]
    fn pages_and_scopes() {
        let store = frames(10);
        let page = store.page(None, 8, 5);
        assert_eq!((page.total, page.offset), (10, 8));
        assert_eq!(page.rows.len(), 2);
        assert_eq!(page.rows[1].payload, json!({ "kind": "frame", "value": 9 }));
        let page = store.page(Some(range(25, 55)), 0, 10);
        assert_eq!(page.total, 4);
        assert_eq!(
            page.rows.iter().map(|row| row.index).collect::<Vec<_>>(),
            [2, 3, 4, 5]
        );
        let page = store.page(None, 100, 5);
        assert_eq!((page.total, page.offset, page.rows.len()), (10, 10, 0));
    }

    fn part(start: u64, end: u64, role: EventRole) -> RolePart {
        RolePart { start, end, role }
    }

    /// Two transactions of an address and three data bytes, 8 ticks each at 10-tick steps.
    fn transactions() -> Store {
        let mut store = Store::for_stream(
            SchemaId::new("test.events", 1),
            vec![
                kind("address", uiharu_registry::Role::Address),
                kind("data", uiharu_registry::Role::Data),
                kind("error", uiharu_registry::Role::Error),
            ],
        );
        for (index, name) in ["address", "data", "data", "data"]
            .iter()
            .cycle()
            .take(8)
            .enumerate()
        {
            let start = index as u64 * 10;
            store.push_event(range(start, start + 8), &json!({ "kind": name }));
        }
        store
    }

    #[test]
    fn dense_items_carry_the_role_of_every_column() {
        let view = transactions().view(range(0, 100), 5.0, 100);
        let [LaneBlock::Dense { count, parts, .. }] = &view.blocks[..] else {
            panic!("one dense item: {:?}", view.blocks);
        };
        assert_eq!(*count, 8);
        assert_eq!(
            parts,
            &[
                part(0, 10, EventRole::Address),
                part(10, 40, EventRole::Data),
                part(40, 50, EventRole::Address),
                part(50, 78, EventRole::Data),
            ]
        );
    }

    #[test]
    fn errors_are_never_packed_with_other_events() {
        let mut store = frames(100);
        store.roles = KindRoles::new(vec![kind("error", uiharu_registry::Role::Error)]);
        store.push_event(range(505, 506), &json!({ "kind": "error" }));
        let view = store.view(range(0, 1000), 10.0, 100);
        assert!(
            matches!(
                &view.blocks[..],
                [
                    LaneBlock::Dense { end: 508, count: 51, .. },
                    LaneBlock::Event(row),
                    LaneBlock::Dense { start: 510, count: 49, .. },
                ] if row.role == EventRole::Error && row.start == 505
            ),
            "{:?}",
            view.blocks
        );
        // Errors close together pack with one another, as errors.
        store.push_event(range(507, 508), &json!({ "kind": "error" }));
        let view = store.view(range(0, 1000), 10.0, 100);
        assert!(
            matches!(
                &view.blocks[1],
                LaneBlock::Dense { count: 2, parts, .. }
                    if parts.iter().all(|part| part.role == EventRole::Error)
            ),
            "{:?}",
            view.blocks
        );
    }

    #[test]
    fn coverage_paints_columns_by_role() {
        let mut store = frames(3);
        store.push_event(range(100, 108), &json!({ "kind": "frame", "value": 10 }));
        assert_eq!(
            store.coverage(range(0, 200), 5),
            [
                part(0, 30, EventRole::Data),
                part(100, 110, EventRole::Data)
            ]
        );
        assert_eq!(
            store.coverage(range(0, 200), 100),
            [part(0, 200, EventRole::Data)]
        );
        assert_eq!(
            store.coverage(range(104, 200), 1),
            [part(104, 108, EventRole::Data)]
        );
        // Columns of 20 ticks: data holds most of each, and the last holds an error.
        let mut store = transactions();
        store.push_event(range(75, 76), &json!({ "kind": "error" }));
        assert_eq!(
            store.coverage(range(0, 100), 20),
            [part(0, 60, EventRole::Data), part(60, 80, EventRole::Error)]
        );
    }

    #[test]
    fn searches_in_time_order_up_to_a_limit() {
        let store = frames(20);
        let needle = Needle::parse("frame").expect("a query");
        assert_eq!(store.search(&needle, 100), ((0..20).collect(), false));
        assert_eq!(store.search(&needle, 5), ((0..5).collect(), true));
        let needle = Needle::parse("0x11").expect("a query");
        assert_eq!(store.search(&needle, 100), (vec![17], false));
    }

    #[test]
    fn progress_gaps_and_the_end_are_noted() {
        let mut store = Store::new();
        store.push_progress(Tick(50));
        store.push_gap(range(60, 70));
        store.push_gap(range(70, 80));
        store.push_gap(range(90, 100));
        assert_eq!(store.gaps(), [range(60, 80), range(90, 100)]);
        assert_eq!(store.settled(), Tick(100));
        assert_eq!(store.ended(), None);
        store.push_end(Tick(120));
        assert_eq!(
            (store.settled(), store.ended()),
            (Tick(120), Some(Tick(120)))
        );
    }

    #[test]
    fn pruning_forgets_what_lies_before_a_tick() {
        let mut store = frames(10);
        store.push_event(range(0, 95), &json!({ "kind": "long" }));
        store.push_gap(range(30, 40));
        store.push_gap(range(60, 62));
        let bytes = store.text_bytes();
        store.prune_before(Tick(50));
        assert_eq!(store.len(), 6);
        assert_eq!(store.row(0).expect("a row").kind, "long");
        assert_eq!(store.row(1).expect("a row").payload["value"], 5);
        assert_eq!(store.max_reach, [95, 95, 95, 95, 95, 98]);
        assert_eq!(store.gaps(), [range(60, 62)]);
        assert!(store.text_bytes() < bytes);
        assert_eq!(
            store.overlapping(range(0, 100)).collect::<Vec<_>>(),
            [0, 1, 2, 3, 4, 5]
        );
    }

    fn jtag() -> Store {
        Store::for_stream(SchemaId::new("jtag.events", 1), Vec::new())
    }

    #[test]
    fn jtag_scans_take_the_role_of_their_register_and_states_of_their_column() {
        let mut store = jtag();
        state(&mut store, 0, "run-test-idle");
        state(&mut store, 10, "shift-ir");
        state(&mut store, 20, "shift-dr");
        store.push_event(
            range(10, 14),
            &json!({ "kind": "scan-begin", "scan": 1, "register": "ir" }),
        );
        store.push_event(range(10, 14), &json!({ "kind": "shift", "scan": 1 }));
        store.push_event(
            range(20, 30),
            &json!({ "kind": "scan-begin", "scan": 2, "register": "dr" }),
        );
        store.push_event(range(20, 30), &json!({ "kind": "shift", "scan": 2 }));
        store.push_end(Tick(40));
        let view = store.view(range(0, 40), 1.0, 100);
        let scans: Vec<(String, EventRole)> = view
            .blocks
            .iter()
            .filter_map(|block| match block {
                LaneBlock::Event(row) => Some((row.kind.clone(), row.role)),
                LaneBlock::Dense { .. } => None,
            })
            .collect();
        let kind = |name: &str, role| (name.to_owned(), role);
        assert_eq!(
            scans,
            [
                kind("scan-begin", EventRole::Command),
                kind("shift", EventRole::Command),
                kind("scan-begin", EventRole::Data),
                kind("shift", EventRole::Data),
            ]
        );
        let states: Vec<EventRole> = view.annotations.expect("the state lane")[0]
            .blocks
            .iter()
            .map(|block| match block {
                LaneBlock::Event(row) => row.role,
                LaneBlock::Dense { .. } => panic!("the states fit separately"),
            })
            .collect();
        assert_eq!(
            states,
            [EventRole::Framing, EventRole::Command, EventRole::Data]
        );
    }

    fn state(store: &mut Store, tick: u64, name: &str) {
        store.push_event(
            range(tick, tick + 1),
            &json!({ "kind": "state", "state": name }),
        );
    }

    fn state_spans(view: &StreamView) -> Vec<(u64, u64, String)> {
        let lane = &view.annotations.as_ref().expect("the state lane")[0];
        assert_eq!(lane.id, "states");
        lane.blocks
            .iter()
            .map(|block| match block {
                LaneBlock::Event(row) => (
                    row.start,
                    row.end,
                    row.payload["state"].as_str().expect("the state").to_owned(),
                ),
                LaneBlock::Dense { .. } => panic!("the states fit separately"),
            })
            .collect()
    }

    #[test]
    fn jtag_states_cover_their_duration_without_changing_raw_events() {
        let mut store = jtag();
        state(&mut store, 10, "capture-ir");
        state(&mut store, 90, "exit1-ir");
        store.push_event(range(15, 25), &json!({ "kind": "shift", "tdi": [3] }));
        state(&mut store, 30, "shift-ir");
        store.push_event(range(30, 31), &json!({ "kind": "scan-begin", "scan": 1 }));
        store.push_end(Tick(120));

        let view = store.view(range(0, 120), 1.0, 100);
        assert_eq!(
            state_spans(&view),
            [
                (10, 30, "capture-ir".into()),
                (30, 90, "shift-ir".into()),
                (90, 120, "exit1-ir".into())
            ]
        );
        assert_eq!(view.blocks.len(), 2);
        assert!(
            view.blocks
                .iter()
                .all(|block| matches!(block, LaneBlock::Event(row) if row.kind != "state"))
        );
        assert_eq!(
            state_spans(&store.view(range(40, 80), 1.0, 100)),
            [(30, 90, "shift-ir".into())]
        );
        let raw = store.page(None, 0, 100);
        assert_eq!(raw.total, 5);
        assert!(
            raw.rows
                .iter()
                .filter(|row| row.kind == "state")
                .all(|row| row.end == row.start + 1)
        );
        let (hits, _) = store.search(&Needle::parse("shift-ir").expect("a query"), 100);
        assert_eq!(store.span(hits[0]), Some(range(30, 31)));
    }

    #[test]
    fn a_live_state_reaches_only_the_settled_position_and_then_the_end() {
        let mut store = jtag();
        state(&mut store, 10, "run-test-idle");
        store.push_progress(Tick(50));
        assert_eq!(
            state_spans(&store.view(range(30, 100), 1.0, 10)),
            [(10, 50, "run-test-idle".into())]
        );
        store.push_end(Tick(80));
        assert_eq!(
            state_spans(&store.view(range(30, 100), 1.0, 10)),
            [(10, 80, "run-test-idle".into())]
        );
        assert!(state_spans(&store.view(range(80, 100), 1.0, 10)).is_empty());
    }

    #[test]
    fn state_annotations_stop_at_gaps_and_survive_retention_only_while_active() {
        let mut store = jtag();
        state(&mut store, 10, "shift-dr");
        state(&mut store, 90, "test-logic-reset");
        store.push_gap(range(50, 70));
        store.push_end(Tick(120));
        assert_eq!(
            state_spans(&store.view(range(0, 120), 1.0, 10)),
            [
                (10, 50, "shift-dr".into()),
                (90, 120, "test-logic-reset".into())
            ]
        );
        assert!(state_spans(&store.view(range(70, 90), 1.0, 10)).is_empty());
        let packed = store.view(range(0, 120), 100.0, 10);
        assert_eq!(packed.annotations.expect("states")[0].blocks.len(), 2);
        store.prune_before(Tick(40));
        assert_eq!(store.page(None, 0, 10).total, 1);
        assert_eq!(
            state_spans(&store.view(range(40, 45), 1.0, 10)),
            [(10, 50, "shift-dr".into())]
        );
        store.prune_before(Tick(80));
        assert!(store.gaps().is_empty());
        assert!(state_spans(&store.view(range(80, 90), 1.0, 10)).is_empty());
        store.prune_before(Tick(100));
        assert!(store.is_empty());
        assert_eq!(
            state_spans(&store.view(range(100, 120), 1.0, 10)),
            [(90, 120, "test-logic-reset".into())]
        );
    }

    #[test]
    fn each_jtag_lane_packs_and_limits_its_own_blocks() {
        let mut store = jtag();
        state(&mut store, 0, "capture-ir");
        state(&mut store, 10, "shift-ir");
        state(&mut store, 20, "exit1-ir");
        store.push_event(range(5, 8), &json!({ "kind": "shift", "tdi": [1] }));
        store.push_end(Tick(30));
        let view = store.view(range(0, 30), 1.0, 1);
        assert_eq!(view.blocks.len(), 1);
        let states = &view.annotations.as_ref().expect("states")[0];
        assert_eq!(states.blocks.len(), 1);
        assert!(states.truncated && view.truncated);
        let view = store.view(range(0, 30), 10.0, 1);
        assert!(!view.truncated);
        assert!(matches!(
            &view.annotations.as_ref().expect("states")[0].blocks[..],
            [LaneBlock::Dense { count: 3, .. }]
        ));
    }

    #[test]
    fn other_schemas_keep_state_events_on_the_original_lane() {
        let mut store = Store::new();
        state(&mut store, 10, "idle");
        store.push_end(Tick(50));
        let view = store.view(range(0, 50), 1.0, 10);
        assert!(view.annotations.is_none());
        assert!(matches!(&view.blocks[..], [LaneBlock::Event(row)] if row.end == 11));
    }
}
