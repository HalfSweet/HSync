//! Keeping the events of one stream and answering the interface's questions about them.

use serde::Serialize;
use serde_json::Value;
use uiharu_core::{Tick, TickRange};

use crate::search::Needle;

/// One event: where it lies and where its payload is in the text.
#[derive(Debug, Clone, Copy)]
struct Entry {
    start: u64,
    end: u64,
    offset: usize,
    len: u32,
    kind: u16,
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
}

/// An event as the interface reads it.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Row {
    /// Its position in the stream's time order.
    pub index: u32,
    pub start: u64,
    pub end: u64,
    /// The contract's kind of event.
    pub kind: String,
    /// The event as the contract serializes it.
    pub payload: Value,
}

/// Something drawn on a lane.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum LaneItem {
    /// One event.
    Event(Row),
    /// Events too narrow to tell apart at this scale, packed together.
    Dense { start: u64, end: u64, count: u32 },
}

/// A lane over a viewport.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct View {
    pub items: Vec<LaneItem>,
    /// More items lay in the viewport than were returned.
    pub truncated: bool,
}

/// Part of a stream's rows.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Page {
    /// Rows in the requested scope.
    pub total: u32,
    /// Position of the first row returned within the scope.
    pub offset: u32,
    pub rows: Vec<Row>,
}

impl Store {
    /// An empty store.
    pub fn new() -> Self {
        Self::default()
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
    pub fn row(&self, index: usize) -> Option<Row> {
        let entry = self.entries.get(index)?;
        Some(Row {
            index: index as u32,
            start: entry.start,
            end: entry.end,
            kind: self.kinds[usize::from(entry.kind)].clone(),
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
    /// items.
    pub fn view(&self, range: TickRange, samples_per_pixel: f64, limit: usize) -> View {
        // Narrower than two pixels is too narrow to tell apart.
        let unit = (samples_per_pixel * 2.0).max(1.0) as u64;
        let mut view = View {
            items: Vec::new(),
            truncated: false,
        };
        let mut run: Option<(usize, u64, u64, u32)> = None;
        let flush = |run: Option<(usize, u64, u64, u32)>, view: &mut View| match run {
            Some((first, _, _, 1)) => view
                .items
                .push(LaneItem::Event(self.row(first).expect("the row is held"))),
            Some((_, start, end, count)) => {
                view.items.push(LaneItem::Dense { start, end, count });
            }
            None => {}
        };
        for index in self.overlapping(range) {
            if view.items.len() >= limit {
                view.truncated = true;
                return view;
            }
            let entry = &self.entries[index];
            if entry.reach() - entry.start >= unit {
                flush(run.take(), &mut view);
                view.items
                    .push(LaneItem::Event(self.row(index).expect("the row is held")));
                continue;
            }
            match &mut run {
                Some((_, _, end, count)) if entry.start <= *end + unit => {
                    *end = (*end).max(entry.reach());
                    *count += 1;
                }
                _ => {
                    flush(run.take(), &mut view);
                    run = Some((index, entry.start, entry.reach(), 1));
                }
            }
        }
        flush(run, &mut view);
        view
    }

    /// `count` rows from `offset`, of all rows or of those overlapping `scope`.
    pub fn page(&self, scope: Option<TickRange>, offset: usize, count: usize) -> Page {
        let indices: Vec<usize> = match scope {
            Some(scope) => self.overlapping(scope).collect(),
            None => (0..self.entries.len()).collect(),
        };
        let begin = offset.min(indices.len());
        let finish = begin.saturating_add(count).min(indices.len());
        Page {
            total: indices.len() as u32,
            offset: begin as u32,
            rows: indices[begin..finish]
                .iter()
                .map(|&index| self.row(index).expect("the row is held"))
                .collect(),
        }
    }

    /// The stretches of `range` holding events, those closer than `resolution` ticks joined.
    pub fn coverage(&self, range: TickRange, resolution: u64) -> Vec<TickRange> {
        let mut stretches: Vec<TickRange> = Vec::new();
        for index in self.overlapping(range) {
            let entry = &self.entries[index];
            match stretches.last_mut() {
                Some(last) if entry.start <= last.end.0 + resolution => {
                    last.end = Tick(last.end.0.max(entry.reach()));
                }
                _ => stretches.push(TickRange::new(Tick(entry.start), Tick(entry.reach()))),
            }
        }
        stretches
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
