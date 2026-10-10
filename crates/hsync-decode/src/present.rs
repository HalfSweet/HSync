//! What the timeline and the data stream show of a decoded bus (DESIGN.md §6.3, §9).
//!
//! Everything here is structure without language: the UI turns notes and tokens into text with
//! its translations. It lists what the bus carried and does not guess what it meant.

use serde::Serialize;
use std::ops::Range;
use ts_rs::TS;

use crate::search::{Found, Needle, Searchable, Sequence};
use crate::spec::{Bus, Decoded};
use crate::{can, i2c, spi, uart};

/// A start, repeated start or stop condition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum I2cCondition {
    Start,
    RepeatedStart,
    Stop,
}

/// An SPI data line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum DataLine {
    Miso,
    Mosi,
}

/// One block or mark on a decode lane.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(tag = "type", rename_all = "camelCase")]
#[ts(export)]
pub enum LaneItem {
    /// Items too narrow at this zoom to draw one by one.
    Dense {
        start: u64,
        end: u64,
    },
    I2cAddress {
        start: u64,
        end: u64,
        address: u8,
        read: bool,
    },
    I2cData {
        start: u64,
        end: u64,
        value: u8,
    },
    I2cAck {
        start: u64,
        end: u64,
        ack: bool,
    },
    I2cCondition {
        start: u64,
        end: u64,
        condition: I2cCondition,
    },
    UartFrame {
        start: u64,
        end: u64,
        value: u16,
        error: bool,
    },
    SpiWord {
        start: u64,
        end: u64,
        value: u32,
    },
    CanFrame {
        start: u64,
        end: u64,
        id: u32,
        extended: bool,
        fd: bool,
        length: u32,
        ok: bool,
    },
    /// An annotation of a decoder without a native presentation: its texts from the longest to
    /// the shortest, to fit the width it has, with `{$}` standing for `value` (hex digits)
    /// shown in the user's number format.
    Annotation {
        start: u64,
        end: u64,
        class: u32,
        texts: Vec<String>,
        value: Option<String>,
    },
}

impl LaneItem {
    pub fn start(&self) -> u64 {
        match *self {
            Self::Dense { start, .. }
            | Self::I2cAddress { start, .. }
            | Self::I2cData { start, .. }
            | Self::I2cAck { start, .. }
            | Self::I2cCondition { start, .. }
            | Self::UartFrame { start, .. }
            | Self::SpiWord { start, .. }
            | Self::CanFrame { start, .. }
            | Self::Annotation { start, .. } => start,
        }
    }

    pub fn end(&self) -> u64 {
        match *self {
            Self::Dense { end, .. }
            | Self::I2cAddress { end, .. }
            | Self::I2cData { end, .. }
            | Self::I2cAck { end, .. }
            | Self::I2cCondition { end, .. }
            | Self::UartFrame { end, .. }
            | Self::SpiWord { end, .. }
            | Self::CanFrame { end, .. }
            | Self::Annotation { end, .. } => end,
        }
    }
}

/// The items of one lane in a viewport.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
pub struct LaneView {
    /// `header` (the group title row), `miso` or `mosi`.
    pub lane: String,
    pub items: Vec<LaneItem>,
}

/// A decoder's lanes in a viewport.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
pub struct DecoderView {
    pub lanes: Vec<LaneView>,
    /// Nothing in view: where the nearest items before and after are.
    pub previous: Option<u64>,
    pub next: Option<u64>,
}

/// A note in the data stream instead of decoded content.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum StreamNote {
    /// An I²C transfer was under way when the capture began.
    I2cHeadCut,
    /// A UART frame was being sent when the capture began.
    UartHeadCut,
    /// A UART frame was still being sent when the capture ended.
    UartTailCut,
    /// A CAN frame was on the bus when the capture began or ended.
    CanCut,
}

/// One element of the I²C bus order column, with the samples it spans so the stream can shade
/// the ones inside the segment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(tag = "type", rename_all = "camelCase")]
#[ts(export)]
pub enum I2cToken {
    Start {
        start: u64,
        end: u64,
    },
    RepeatedStart {
        start: u64,
        end: u64,
    },
    Stop {
        start: u64,
        end: u64,
    },
    Address {
        start: u64,
        end: u64,
        address: u8,
        read: bool,
    },
    Data {
        start: u64,
        end: u64,
        value: u8,
    },
    /// Not acknowledged; acknowledged bytes are not marked.
    Nack {
        start: u64,
        end: u64,
    },
}

/// What the data stream lists for a decoder, one row per item.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(tag = "type", rename_all = "camelCase")]
#[ts(export)]
pub enum StreamRow {
    Note {
        start: u64,
        end: u64,
        note: StreamNote,
    },
    /// One I²C transfer; `cut` when the capture ended inside it.
    I2c {
        start: u64,
        end: u64,
        tokens: Vec<I2cToken>,
        cut: bool,
    },
    /// Up to 80 characters of UART text, ending at a line feed.
    UartText { start: u64, end: u64, text: String },
    /// Up to 16 words in hex with their offset.
    Hex {
        start: u64,
        end: u64,
        line: Option<DataLine>,
        offset: u32,
        values: Vec<u32>,
        /// Bits per word.
        bits: u8,
    },
    Can {
        start: u64,
        end: u64,
        id: u32,
        extended: bool,
        fd: bool,
        brs: bool,
        data: Vec<u8>,
        ok: bool,
        acked: bool,
        error: bool,
    },
    /// A blank line between sections.
    Gap { start: u64, end: u64 },
    /// An annotation of a decoder without a native presentation and the row it belongs to.
    Annotation {
        start: u64,
        end: u64,
        /// Position of the row in the decoder's annotation rows.
        row: u32,
        /// Its longest text, `{$}` standing for `value`.
        text: String,
        value: Option<String>,
    },
}

impl StreamRow {
    pub fn start(&self) -> u64 {
        match *self {
            Self::Note { start, .. }
            | Self::I2c { start, .. }
            | Self::UartText { start, .. }
            | Self::Hex { start, .. }
            | Self::Can { start, .. }
            | Self::Gap { start, .. }
            | Self::Annotation { start, .. } => start,
        }
    }

    pub fn end(&self) -> u64 {
        match *self {
            Self::Note { end, .. }
            | Self::I2c { end, .. }
            | Self::UartText { end, .. }
            | Self::Hex { end, .. }
            | Self::Can { end, .. }
            | Self::Gap { end, .. }
            | Self::Annotation { end, .. } => end,
        }
    }
}

/// Text or hex listing of UART data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum StreamMode {
    Text,
    Hex,
}

/// Counts for the data stream's summary line.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct StreamCounts {
    /// I²C and SPI transfers.
    pub transfers: u32,
    /// UART and CAN frames.
    pub frames: u32,
    /// Bytes (words) carried.
    pub bytes: u32,
}

/// A decoder's result arranged for display.
#[derive(Debug)]
pub struct Presentation {
    bus: Bus,
    lanes: Vec<(String, Lane)>,
    text_rows: Vec<StreamRow>,
    hex_rows: Vec<StreamRow>,
    /// Byte-sized units with their spans, for counting inside a segment.
    units: Vec<(u64, u64)>,
    counts: StreamCounts,
    search: Searchable,
}

/// An annotation of a decoder without a native presentation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Annotation {
    pub start: u64,
    pub end: u64,
    /// Its class among the decoder's annotations.
    pub class: u32,
    /// Its row among the decoder's annotation rows.
    pub row: u32,
    /// From the longest to the shortest, as the decoder put them: `{$}` stands for `value`.
    pub texts: Vec<String>,
    /// DSView's number to show in the user's format, in hex digits (`'@4F'` or an integer).
    pub value: Option<String>,
}

impl Annotation {
    /// The texts to show: `{$}` replaced by the value, or the value alone without texts.
    pub fn shown(&self) -> Vec<String> {
        let value = self.value.as_deref();
        let texts: Vec<String> = self
            .texts
            .iter()
            .map(|text| match value {
                Some(value) => text.replace("{$}", value),
                None => text.clone(),
            })
            .collect();
        if texts.is_empty() {
            value.map(str::to_owned).into_iter().collect()
        } else {
            texts
        }
    }
}

/// Items of one lane, sorted by start, with the running maximum of their ends.
#[derive(Debug, Default)]
struct Lane {
    items: Vec<LaneItem>,
    max_end: Vec<u64>,
}

impl Lane {
    fn new(mut items: Vec<LaneItem>) -> Self {
        items.sort_by_key(LaneItem::start);
        let mut max_end = Vec::with_capacity(items.len());
        let mut running = 0;
        for item in &items {
            running = running.max(item.end());
            max_end.push(running);
        }
        Self { items, max_end }
    }

    /// Items overlapping `start..end`, those narrower than `min_width` samples merged into
    /// dense stretches.
    fn view(&self, start: u64, end: u64, min_width: u64) -> Vec<LaneItem> {
        let first = self.max_end.partition_point(|&max| max < start);
        let mut out: Vec<LaneItem> = Vec::new();
        for item in self.items[first..]
            .iter()
            .take_while(|item| item.start() < end)
        {
            // Marks (zero width) count where they are; blocks while they reach into view.
            let inside = if item.start() == item.end() {
                item.start() >= start
            } else {
                item.end() > start
            };
            if !inside {
                continue;
            }
            let narrow = item.end().saturating_sub(item.start()) < min_width;
            if narrow {
                if let Some(LaneItem::Dense { end: dense_end, .. }) = out.last_mut()
                    && item.start() <= *dense_end + min_width
                {
                    *dense_end = (*dense_end).max(item.end());
                    continue;
                }
                // A lone narrow item stays itself; it turns dense once a neighbour joins.
                if let Some(last) = out.last()
                    && last.end().saturating_sub(last.start()) < min_width
                    && item.start() <= last.end() + min_width
                {
                    let start = last.start();
                    out.pop();
                    out.push(LaneItem::Dense {
                        start,
                        end: item.end().max(start),
                    });
                    continue;
                }
            }
            out.push(item.clone());
        }
        out
    }

    fn previous(&self, before: u64) -> Option<u64> {
        let at = self.items.partition_point(|item| item.start() < before);
        self.items[..at]
            .iter()
            .rev()
            .find(|item| item.end() <= before)
            .map(LaneItem::start)
    }

    fn next(&self, after: u64) -> Option<u64> {
        let at = self.items.partition_point(|item| item.start() < after);
        self.items.get(at).map(LaneItem::start)
    }
}

/// Words per hex row.
const HEX_WIDTH: usize = 16;
/// Characters per UART text row.
const TEXT_WIDTH: usize = 80;

/// Everything a presentation holds, for tools that replay it elsewhere (the UI's browser mock).
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot<'a> {
    pub bus: Bus,
    /// Every item of every lane, by lane name.
    pub lanes: Vec<(&'a str, &'a [LaneItem])>,
    pub text_rows: &'a [StreamRow],
    pub hex_rows: &'a [StreamRow],
    pub units: &'a [(u64, u64)],
    pub counts: StreamCounts,
    /// What a search looks through.
    pub sequences: &'a [Sequence],
    pub ids: &'a [(u32, u64, u64)],
}

impl Presentation {
    /// Everything this presentation holds.
    pub fn snapshot(&self) -> Snapshot<'_> {
        Snapshot {
            bus: self.bus,
            lanes: self
                .lanes
                .iter()
                .map(|(name, lane)| (name.as_str(), lane.items.as_slice()))
                .collect(),
            text_rows: &self.text_rows,
            hex_rows: &self.hex_rows,
            units: &self.units,
            counts: self.counts,
            sequences: self.search.sequences(),
            ids: self.search.ids(),
        }
    }

    /// Annotations as they are: a lane per row (named by `rows`), and in the data stream one
    /// line per annotation with its row, in time order.
    pub fn annotations(rows: &[String], mut annotations: Vec<Annotation>) -> Self {
        annotations.sort_by_key(|annotation| (annotation.start, annotation.row));
        let mut lanes: Vec<(String, Vec<LaneItem>)> =
            rows.iter().map(|row| (row.clone(), Vec::new())).collect();
        let mut stream = Vec::with_capacity(annotations.len());
        let mut units = Vec::with_capacity(annotations.len());
        for annotation in &annotations {
            stream.push(StreamRow::Annotation {
                start: annotation.start,
                end: annotation.end,
                row: annotation.row,
                text: annotation.texts.first().cloned().unwrap_or_default(),
                value: annotation.value.clone(),
            });
            if let Some((_, items)) = lanes.get_mut(annotation.row as usize) {
                items.push(LaneItem::Annotation {
                    start: annotation.start,
                    end: annotation.end,
                    class: annotation.class,
                    texts: annotation.texts.clone(),
                    value: annotation.value.clone(),
                });
            }
            units.push((annotation.start, annotation.end));
        }
        units.sort_unstable();
        let search = Searchable::annotations(&stream);
        Presentation {
            bus: Bus::Other,
            lanes: lanes
                .into_iter()
                .map(|(name, items)| (name, Lane::new(items)))
                .collect(),
            hex_rows: stream.clone(),
            text_rows: stream,
            units,
            counts: StreamCounts {
                transfers: 0,
                frames: annotations.len() as u32,
                bytes: 0,
            },
            search,
        }
    }

    /// Adds lanes named `rows` for `annotations` of decoders stacked on this one (their `row`
    /// counts within `rows`); the data stream keeps listing this decoder.
    pub fn append_lanes(&mut self, rows: &[String], mut annotations: Vec<Annotation>) {
        annotations.sort_by_key(|annotation| annotation.start);
        let mut lanes: Vec<Vec<LaneItem>> = rows.iter().map(|_| Vec::new()).collect();
        for annotation in annotations {
            if let Some(items) = lanes.get_mut(annotation.row as usize) {
                items.push(LaneItem::Annotation {
                    start: annotation.start,
                    end: annotation.end,
                    class: annotation.class,
                    texts: annotation.texts,
                    value: annotation.value,
                });
            }
        }
        self.lanes.extend(
            rows.iter()
                .cloned()
                .zip(lanes)
                .map(|(name, items)| (name, Lane::new(items))),
        );
    }

    pub fn new(decoded: &Decoded) -> Self {
        let mut presentation = match decoded {
            Decoded::I2c { trace, shifted } => present_i2c(trace, *shifted),
            Decoded::Uart(trace) => present_uart(trace),
            Decoded::Spi(trace) => present_spi(trace),
            Decoded::Can(trace) => present_can(trace),
        };
        presentation.search =
            Searchable::new(decoded, &presentation.text_rows, &presentation.hex_rows);
        presentation
    }

    /// What `needle` finds, in time order, with the rows listing it in `mode`.
    pub fn search(&self, needle: &Needle, mode: StreamMode) -> Vec<Found> {
        self.search.find(needle, mode, self.rows(mode))
    }

    pub fn bus(&self) -> Bus {
        self.bus
    }

    /// The lanes over `start..end` drawn `samples_per_pixel` samples to a pixel.
    pub fn view(&self, start: u64, end: u64, samples_per_pixel: f64) -> DecoderView {
        self.view_around(start..end, start..end, samples_per_pixel)
    }

    /// The lanes over `window` — the samples in view and some either side, fetched ahead so
    /// panning has them at hand — with, when no item is in `focus` (the samples in view), the
    /// frames either side.
    pub fn view_around(
        &self,
        window: Range<u64>,
        focus: Range<u64>,
        samples_per_pixel: f64,
    ) -> DecoderView {
        let (start, end) = (window.start, window.end);
        // Narrower than 3 px is unreadable.
        let min_width = (samples_per_pixel * 3.0).ceil() as u64;
        let lanes: Vec<LaneView> = self
            .lanes
            .iter()
            .map(|(name, lane)| LaneView {
                lane: name.clone(),
                items: lane.view(start, end, min_width),
            })
            .collect();
        let empty = lanes.iter().all(|lane| {
            lane.items
                .iter()
                .all(|item| item.end() <= focus.start || item.start() >= focus.end)
        });
        let header = self.lanes.first().map(|(_, lane)| lane);
        DecoderView {
            lanes,
            previous: if empty {
                header.and_then(|lane| lane.previous(focus.start))
            } else {
                None
            },
            next: if empty {
                header.and_then(|lane| lane.next(focus.end))
            } else {
                None
            },
        }
    }

    /// Every row of the data stream in `mode`.
    pub fn rows(&self, mode: StreamMode) -> &[StreamRow] {
        match mode {
            StreamMode::Text => &self.text_rows,
            StreamMode::Hex => &self.hex_rows,
        }
    }

    pub fn counts(&self) -> StreamCounts {
        self.counts
    }

    /// Bytes (I²C, SPI, UART) or frames (CAN) entirely inside `start..end`.
    pub fn count_inside(&self, start: u64, end: u64) -> u32 {
        let first = self.units.partition_point(|&(from, _)| from < start);
        self.units[first..]
            .iter()
            .take_while(|&&(from, _)| from < end)
            .filter(|&&(_, to)| to <= end)
            .count() as u32
    }
}

fn present_i2c(trace: &i2c::Trace, shifted: bool) -> Presentation {
    let mut bytes = Vec::new();
    let mut rows = Vec::new();
    let mut units = Vec::new();
    let mut counts = StreamCounts::default();
    if let Some(at) = trace.cut_head {
        rows.push(StreamRow::Note {
            start: 0,
            end: at,
            note: StreamNote::I2cHeadCut,
        });
    }
    let shown = |item: &i2c::Item| if shifted { item.address() } else { item.byte };

    // Transfers run from a start condition to the stop (or the end of the capture).
    let mut index = 0;
    let items = &trace.items;
    while index < items.len() {
        let first = &items[index];
        if first.kind != i2c::Kind::Start {
            index += 1;
            continue;
        }
        let mut last = index;
        while last + 1 < items.len() && items[last + 1].kind != i2c::Kind::Start {
            last += 1;
            if items[last].kind == i2c::Kind::Stop {
                break;
            }
        }
        let transfer = &items[index..=last];
        let cut = transfer
            .last()
            .is_none_or(|item| item.kind != i2c::Kind::Stop);
        let start = first.start;
        let end = transfer.iter().map(|item| item.end).max().unwrap_or(start);
        let mut tokens = Vec::new();
        for item in transfer {
            match item.kind {
                i2c::Kind::Start => {
                    tokens.push(I2cToken::Start {
                        start: item.start,
                        end: item.end,
                    });
                    bytes.push(LaneItem::I2cCondition {
                        start: item.start,
                        end: item.end,
                        condition: I2cCondition::Start,
                    });
                }
                i2c::Kind::RepeatedStart => {
                    tokens.push(I2cToken::RepeatedStart {
                        start: item.start,
                        end: item.end,
                    });
                    bytes.push(LaneItem::I2cCondition {
                        start: item.start,
                        end: item.end,
                        condition: I2cCondition::RepeatedStart,
                    });
                }
                i2c::Kind::Stop => {
                    tokens.push(I2cToken::Stop {
                        start: item.start,
                        end: item.end,
                    });
                    bytes.push(LaneItem::I2cCondition {
                        start: item.start,
                        end: item.end,
                        condition: I2cCondition::Stop,
                    });
                }
                i2c::Kind::AddressWrite | i2c::Kind::AddressRead => {
                    let read = item.kind == i2c::Kind::AddressRead;
                    tokens.push(I2cToken::Address {
                        start: item.start,
                        end: item.end,
                        address: shown(item),
                        read,
                    });
                    bytes.push(LaneItem::I2cAddress {
                        start: item.start,
                        end: item.end,
                        address: shown(item),
                        read,
                    });
                    units.push((item.start, item.end));
                    counts.bytes += 1;
                }
                i2c::Kind::DataWrite | i2c::Kind::DataRead => {
                    tokens.push(I2cToken::Data {
                        start: item.start,
                        end: item.end,
                        value: item.byte,
                    });
                    bytes.push(LaneItem::I2cData {
                        start: item.start,
                        end: item.end,
                        value: item.byte,
                    });
                    units.push((item.start, item.end));
                    counts.bytes += 1;
                }
                i2c::Kind::Ack | i2c::Kind::Nack => {
                    let ack = item.kind == i2c::Kind::Ack;
                    if !ack {
                        tokens.push(I2cToken::Nack {
                            start: item.start,
                            end: item.end,
                        });
                    }
                    bytes.push(LaneItem::I2cAck {
                        start: item.start,
                        end: item.end,
                        ack,
                    });
                }
            }
        }
        rows.push(StreamRow::I2c {
            start,
            end,
            tokens,
            cut,
        });
        counts.transfers += 1;
        index = last + 1;
    }
    Presentation {
        bus: Bus::I2c,
        lanes: vec![("header".to_owned(), Lane::new(bytes))],
        hex_rows: rows.clone(),
        text_rows: rows,
        units,
        counts,
        search: Searchable::default(),
    }
}

fn present_uart(trace: &uart::Trace) -> Presentation {
    let frames = &trace.frames;
    let items: Vec<LaneItem> = frames
        .iter()
        .map(|frame| LaneItem::UartFrame {
            start: frame.start,
            end: frame.end,
            value: frame.value,
            error: frame.framing_error || frame.parity_ok == Some(false),
        })
        .collect();
    let mut text_rows = Vec::new();
    let mut hex_rows = Vec::new();
    if trace.cut_head.is_some() {
        let note = StreamRow::Note {
            start: 0,
            end: frames.first().map_or(0, |frame| frame.start),
            note: StreamNote::UartHeadCut,
        };
        text_rows.push(note.clone());
        hex_rows.push(note);
    }
    // Text: break after line feeds and at 80 characters.
    let mut line_start = 0;
    for (index, frame) in frames.iter().enumerate() {
        let full = index + 1 - line_start >= TEXT_WIDTH;
        if frame.value == u16::from(b'\n') || full || index + 1 == frames.len() {
            let slice = &frames[line_start..=index];
            text_rows.push(StreamRow::UartText {
                start: slice[0].start,
                end: frame.end,
                text: slice
                    .iter()
                    .map(|f| char::from_u32(u32::from(f.value)).unwrap_or('\u{fffd}'))
                    .collect(),
            });
            line_start = index + 1;
        }
    }
    for (row, chunk) in frames.chunks(HEX_WIDTH).enumerate() {
        hex_rows.push(StreamRow::Hex {
            start: chunk[0].start,
            end: chunk[chunk.len() - 1].end,
            line: None,
            offset: (row * HEX_WIDTH) as u32,
            values: chunk.iter().map(|f| u32::from(f.value)).collect(),
            bits: 8,
        });
    }
    if let Some(at) = trace.cut_tail {
        let note = StreamRow::Note {
            start: at,
            end: at,
            note: StreamNote::UartTailCut,
        };
        text_rows.push(note.clone());
        hex_rows.push(note);
    }
    Presentation {
        bus: Bus::Uart,
        lanes: vec![("header".to_owned(), Lane::new(items))],
        text_rows,
        hex_rows,
        units: frames.iter().map(|f| (f.start, f.end)).collect(),
        counts: StreamCounts {
            transfers: 0,
            frames: frames.len() as u32,
            bytes: frames.len() as u32,
        },
        search: Searchable::default(),
    }
}

fn present_spi(trace: &spi::Trace) -> Presentation {
    let mut lanes: [(DataLine, Vec<LaneItem>); 2] =
        [(DataLine::Miso, Vec::new()), (DataLine::Mosi, Vec::new())];
    let mut rows = Vec::new();
    let bits = 8u8;
    for transfer in &trace.transfers {
        let words = &trace.words[transfer.words.clone()];
        if words.is_empty() {
            continue;
        }
        for (line, lane) in &mut lanes {
            let values: Vec<u32> = words
                .iter()
                .filter_map(|w| match line {
                    DataLine::Miso => w.miso,
                    DataLine::Mosi => w.mosi,
                })
                .collect();
            if values.len() != words.len() {
                continue;
            }
            lane.extend(
                words
                    .iter()
                    .zip(&values)
                    .map(|(word, &value)| LaneItem::SpiWord {
                        start: word.start,
                        end: word.end,
                        value,
                    }),
            );
            // Stream: 16 words a row, offsets counted within the transfer.
            if !rows.is_empty() {
                rows.push(StreamRow::Gap {
                    start: transfer.start,
                    end: transfer.start,
                });
            }
            for (row, (chunk, values)) in words
                .chunks(HEX_WIDTH)
                .zip(values.chunks(HEX_WIDTH))
                .enumerate()
            {
                rows.push(StreamRow::Hex {
                    start: chunk[0].start,
                    end: chunk[chunk.len() - 1].end,
                    line: Some(*line),
                    offset: (row * HEX_WIDTH) as u32,
                    values: values.to_vec(),
                    bits,
                });
            }
        }
    }
    let [(_, miso), (_, mosi)] = lanes;
    let words = trace.transfers.iter().map(|t| t.words.len()).sum::<usize>();
    Presentation {
        bus: Bus::Spi,
        lanes: vec![
            ("miso".to_owned(), Lane::new(miso)),
            ("mosi".to_owned(), Lane::new(mosi)),
        ],
        // SPI lists hex only.
        text_rows: rows.clone(),
        hex_rows: rows,
        units: {
            let mut units: Vec<(u64, u64)> = trace
                .transfers
                .iter()
                .flat_map(|t| {
                    trace.words[t.words.clone()]
                        .iter()
                        .map(|w| (w.start, w.end))
                })
                .collect();
            units.sort_unstable();
            units
        },
        counts: StreamCounts {
            transfers: trace.transfers.len() as u32,
            frames: 0,
            bytes: words as u32,
        },
        search: Searchable::default(),
    }
}

fn present_can(trace: &can::Trace) -> Presentation {
    let mut rows = Vec::new();
    if trace.cut_head.is_some() {
        rows.push(StreamRow::Note {
            start: 0,
            end: trace.frames.first().map_or(0, |f| f.start),
            note: StreamNote::CanCut,
        });
    }
    let items = trace
        .frames
        .iter()
        .map(|frame| LaneItem::CanFrame {
            start: frame.start,
            end: frame.end,
            id: frame.id,
            extended: frame.extended,
            fd: frame.fd,
            length: frame.data.len() as u32,
            ok: frame.crc_ok && frame.error.is_none(),
        })
        .collect();
    for frame in &trace.frames {
        rows.push(StreamRow::Can {
            start: frame.start,
            end: frame.end,
            id: frame.id,
            extended: frame.extended,
            fd: frame.fd,
            brs: frame.brs,
            data: frame.data.clone(),
            ok: frame.crc_ok,
            acked: frame.acked,
            error: frame.error.is_some(),
        });
    }
    if let Some(at) = trace.cut_tail {
        rows.push(StreamRow::Note {
            start: at,
            end: at,
            note: StreamNote::CanCut,
        });
    }
    Presentation {
        bus: Bus::Can,
        lanes: vec![("header".to_owned(), Lane::new(items))],
        hex_rows: rows.clone(),
        text_rows: rows,
        units: trace.frames.iter().map(|f| (f.start, f.end)).collect(),
        counts: StreamCounts {
            transfers: 0,
            frames: trace.frames.len() as u32,
            bytes: trace.frames.iter().map(|f| f.data.len() as u32).sum(),
        },
        search: Searchable::default(),
    }
}
