//! What the timeline and the data stream show of a decoded bus (DESIGN.md §7.1, §9.6, §9.9).
//!
//! Everything here is structure without language: the UI turns meanings, notes and tokens into
//! text with its translations.

use serde::Serialize;
use ts_rs::TS;

use crate::search::{Found, Needle, Searchable, Sequence};
use crate::spec::{Bus, Decoded};
use crate::{can, i2c, spi, uart};

/// What an I²C transfer did, in the terms of a register-based target.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(tag = "type", rename_all = "camelCase")]
#[ts(export)]
pub enum I2cMeaning {
    /// Write the register number, repeated start, read: "read 4F from 0x04".
    ReadRegister {
        register: u8,
        data: Vec<u8>,
    },
    /// Write the register number and data: "write 12 to 0x04".
    WriteRegister {
        register: u8,
        data: Vec<u8>,
    },
    Read {
        data: Vec<u8>,
    },
    Write {
        data: Vec<u8>,
    },
    /// Only the address, e.g. a bus scan.
    Probe,
}

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
    I2cTransfer {
        start: u64,
        end: u64,
        meaning: I2cMeaning,
        /// The capture ended before the stop condition.
        cut: bool,
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
    SpiTransfer {
        start: u64,
        end: u64,
        cut: bool,
        /// Offsets of the first and last words in view; filled in by [`Presentation::view`].
        first: u32,
        last: u32,
    },
    /// Characters of one text line, each centred on its word.
    SpiText {
        start: u64,
        end: u64,
        text: String,
        centers: Vec<u64>,
    },
    /// The same word repeated: `FF × 224`, unchanged from `since`.
    SpiRun {
        start: u64,
        end: u64,
        value: u32,
        count: u32,
        since: u32,
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
}

impl LaneItem {
    pub fn start(&self) -> u64 {
        match *self {
            Self::Dense { start, .. }
            | Self::I2cTransfer { start, .. }
            | Self::I2cAddress { start, .. }
            | Self::I2cData { start, .. }
            | Self::I2cAck { start, .. }
            | Self::I2cCondition { start, .. }
            | Self::UartFrame { start, .. }
            | Self::SpiTransfer { start, .. }
            | Self::SpiText { start, .. }
            | Self::SpiRun { start, .. }
            | Self::SpiWord { start, .. }
            | Self::CanFrame { start, .. } => start,
        }
    }

    pub fn end(&self) -> u64 {
        match *self {
            Self::Dense { end, .. }
            | Self::I2cTransfer { end, .. }
            | Self::I2cAddress { end, .. }
            | Self::I2cData { end, .. }
            | Self::I2cAck { end, .. }
            | Self::I2cCondition { end, .. }
            | Self::UartFrame { end, .. }
            | Self::SpiTransfer { end, .. }
            | Self::SpiText { end, .. }
            | Self::SpiRun { end, .. }
            | Self::SpiWord { end, .. }
            | Self::CanFrame { end, .. } => end,
        }
    }
}

/// The items of one lane in a viewport.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
pub struct LaneView {
    /// `header` (the group title row), `bytes`, `miso` or `mosi`.
    pub lane: String,
    pub items: Vec<LaneItem>,
}

/// A decoder's lanes in a viewport.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
pub struct DecoderView {
    pub lanes: Vec<LaneView>,
    /// I²C: the target most transfers in view address.
    pub address: Option<u8>,
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
        meaning: I2cMeaning,
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
    /// SPI text with the offset of its first byte.
    SpiText {
        start: u64,
        end: u64,
        line: DataLine,
        offset: u32,
        text: String,
    },
    /// A word repeated `count` times.
    Run {
        start: u64,
        end: u64,
        line: Option<DataLine>,
        offset: u32,
        value: u32,
        count: u32,
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
}

impl StreamRow {
    pub fn start(&self) -> u64 {
        match *self {
            Self::Note { start, .. }
            | Self::I2c { start, .. }
            | Self::UartText { start, .. }
            | Self::Hex { start, .. }
            | Self::SpiText { start, .. }
            | Self::Run { start, .. }
            | Self::Can { start, .. }
            | Self::Gap { start, .. } => start,
        }
    }

    pub fn end(&self) -> u64 {
        match *self {
            Self::Note { end, .. }
            | Self::I2c { end, .. }
            | Self::UartText { end, .. }
            | Self::Hex { end, .. }
            | Self::SpiText { end, .. }
            | Self::Run { end, .. }
            | Self::Can { end, .. }
            | Self::Gap { end, .. } => end,
        }
    }
}

/// Text or hex listing of UART and SPI data.
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
    /// I²C transfers by span and target, for the lane label.
    addresses: Vec<(u64, u64, u8)>,
    /// SPI transfers by start with their words' spans, for the offsets in view.
    transfer_words: Vec<(u64, Vec<(u64, u64)>)>,
    search: Searchable,
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

/// Characters shown for a byte in SPI text: printable ASCII, else a middle dot.
fn shown_char(value: u32) -> char {
    match u8::try_from(value) {
        Ok(byte) if (0x20..0x7f).contains(&byte) => char::from(byte),
        _ => '·',
    }
}

fn printable(value: u32) -> bool {
    u8::try_from(value).is_ok_and(|byte| (0x20..0x7f).contains(&byte))
}

/// Printable or a line break or tab, which text listings show as escapes.
fn texty(value: u32) -> bool {
    printable(value) || matches!(value, 0x09 | 0x0a | 0x0d)
}

/// Runs this long collapse into one item.
const MIN_RUN: usize = 8;
/// Printable stretches this long count as text.
const MIN_TEXT: usize = 4;
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
    pub addresses: &'a [(u64, u64, u8)],
    pub transfer_words: &'a [(u64, Vec<(u64, u64)>)],
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
            addresses: &self.addresses,
            transfer_words: &self.transfer_words,
            sequences: self.search.sequences(),
            ids: self.search.ids(),
        }
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
        // Narrower than 3 px is unreadable.
        let min_width = (samples_per_pixel * 3.0).ceil() as u64;
        let mut lanes: Vec<LaneView> = self
            .lanes
            .iter()
            .map(|(name, lane)| LaneView {
                lane: name.clone(),
                items: lane.view(start, end, min_width),
            })
            .collect();
        // SPI transfers name the offsets of their words in view.
        for item in lanes.iter_mut().flat_map(|lane| lane.items.iter_mut()) {
            if let LaneItem::SpiTransfer {
                start: transfer_start,
                first,
                last,
                ..
            } = item
                && let Some((_, words)) = self
                    .transfer_words
                    .iter()
                    .find(|(at, _)| at == transfer_start)
            {
                let from = words.partition_point(|&(_, word_end)| word_end <= start);
                let to = words.partition_point(|&(word_start, _)| word_start < end);
                if to > from {
                    *first = from as u32;
                    *last = (to - 1) as u32;
                }
            }
        }
        let empty = lanes.iter().all(|lane| lane.items.is_empty());
        let header = self.lanes.first().map(|(_, lane)| lane);
        let address = {
            let mut counts: std::collections::BTreeMap<u8, usize> = Default::default();
            for &(from, to, address) in &self.addresses {
                if from < end && to > start {
                    *counts.entry(address).or_default() += 1;
                }
            }
            counts
                .into_iter()
                .max_by_key(|&(_, count)| count)
                .map(|(address, _)| address)
        };
        DecoderView {
            lanes,
            address,
            previous: if empty {
                header.and_then(|lane| lane.previous(start))
            } else {
                None
            },
            next: if empty {
                header.and_then(|lane| lane.next(end))
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
    let mut header = Vec::new();
    let mut bytes = Vec::new();
    let mut rows = Vec::new();
    let mut units = Vec::new();
    let mut addresses = Vec::new();
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
        // Data bytes after each address byte.
        let mut segments: Vec<(bool, Vec<u8>)> = Vec::new();
        let mut target = None;
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
                    target.get_or_insert(shown(item));
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
                    segments.push((read, Vec::new()));
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
                    if let Some((_, data)) = segments.last_mut() {
                        data.push(item.byte);
                    }
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
        let meaning = match segments.as_slice() {
            [(false, write), (true, read), ..] if write.len() == 1 => I2cMeaning::ReadRegister {
                register: write[0],
                data: read.clone(),
            },
            [(false, write)] if write.len() >= 2 => I2cMeaning::WriteRegister {
                register: write[0],
                data: write[1..].to_vec(),
            },
            [(true, read)] if !read.is_empty() => I2cMeaning::Read { data: read.clone() },
            [(false, write)] if !write.is_empty() => I2cMeaning::Write {
                data: write.clone(),
            },
            [(true, read), ..] => I2cMeaning::Read { data: read.clone() },
            [(false, write), ..] if !write.is_empty() => I2cMeaning::Write {
                data: write.clone(),
            },
            _ => I2cMeaning::Probe,
        };
        if let Some(target) = target {
            addresses.push((start, end, target));
        }
        header.push(LaneItem::I2cTransfer {
            start,
            end,
            meaning: meaning.clone(),
            cut,
        });
        rows.push(StreamRow::I2c {
            start,
            end,
            meaning,
            tokens,
            cut,
        });
        counts.transfers += 1;
        index = last + 1;
    }
    Presentation {
        bus: Bus::I2c,
        lanes: vec![
            ("header".to_owned(), Lane::new(header)),
            ("bytes".to_owned(), Lane::new(bytes)),
        ],
        hex_rows: rows.clone(),
        text_rows: rows,
        units,
        counts,
        addresses,
        transfer_words: Vec::new(),
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
        addresses: Vec::new(),
        transfer_words: Vec::new(),
        search: Searchable::default(),
    }
}

/// Splits `words` (offsets from `base`) into runs of one value, printable text and the rest.
enum Piece {
    Run { from: usize, to: usize },
    Text { from: usize, to: usize },
    Other { from: usize, to: usize },
}

fn pieces(values: &[u32], text: bool) -> Vec<Piece> {
    let mut out = Vec::new();
    let mut at = 0;
    let mut other_from = None;
    while at < values.len() {
        let run_end = values[at..]
            .iter()
            .position(|&v| v != values[at])
            .map_or(values.len(), |n| at + n);
        let run = run_end - at >= MIN_RUN && (!text || !texty(values[at]));
        let text_end = values[at..]
            .iter()
            .position(|&v| !texty(v))
            .map_or(values.len(), |n| at + n);
        let is_text = text && text_end - at >= MIN_TEXT;
        if run || is_text {
            if let Some(from) = other_from.take() {
                out.push(Piece::Other { from, to: at });
            }
            if run {
                out.push(Piece::Run {
                    from: at,
                    to: run_end,
                });
                at = run_end;
            } else {
                out.push(Piece::Text {
                    from: at,
                    to: text_end,
                });
                at = text_end;
            }
        } else {
            other_from.get_or_insert(at);
            at += 1;
        }
    }
    if let Some(from) = other_from {
        out.push(Piece::Other {
            from,
            to: values.len(),
        });
    }
    out
}

fn present_spi(trace: &spi::Trace) -> Presentation {
    let mut header = Vec::new();
    let mut transfer_words: Vec<(u64, Vec<(u64, u64)>)> = Vec::new();
    let mut lanes: [(DataLine, Vec<LaneItem>); 2] =
        [(DataLine::Miso, Vec::new()), (DataLine::Mosi, Vec::new())];
    let mut text_rows = Vec::new();
    let mut hex_rows = Vec::new();
    let bits = 8u8;
    for transfer in &trace.transfers {
        let words = &trace.words[transfer.words.clone()];
        let Some(last_word) = words.last() else {
            continue;
        };
        let end = transfer.end.unwrap_or(last_word.end);
        header.push(LaneItem::SpiTransfer {
            start: transfer.start,
            end,
            cut: transfer.cut_head || transfer.end.is_none(),
            first: 0,
            last: words.len().saturating_sub(1) as u32,
        });
        transfer_words.push((
            transfer.start,
            words.iter().map(|w| (w.start, w.end)).collect(),
        ));
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
            let mostly_text = values.iter().filter(|&&v| printable(v)).count() * 2 >= values.len();
            // Lane: text lines as characters, or runs and single words.
            if mostly_text {
                let mut from = 0;
                for (index, &value) in values.iter().enumerate() {
                    if value == u32::from(b'\n') || index + 1 == values.len() {
                        let slice = &words[from..=index];
                        lane.push(LaneItem::SpiText {
                            start: slice[0].start,
                            end: slice[slice.len() - 1].end,
                            text: values[from..=index]
                                .iter()
                                .map(|&v| shown_char(v))
                                .collect(),
                            centers: slice.iter().map(|w| (w.start + w.end) / 2).collect(),
                        });
                        from = index + 1;
                    }
                }
            } else {
                for piece in pieces(&values, false) {
                    match piece {
                        Piece::Run { from, to } => lane.push(LaneItem::SpiRun {
                            start: words[from].start,
                            end: words[to - 1].end,
                            value: values[from],
                            count: (to - from) as u32,
                            since: from as u32,
                        }),
                        Piece::Text { from, to } | Piece::Other { from, to } => {
                            for index in from..to {
                                lane.push(LaneItem::SpiWord {
                                    start: words[index].start,
                                    end: words[index].end,
                                    value: values[index],
                                });
                            }
                        }
                    }
                }
            }
            // Stream rows for both modes.
            for (mode_rows, text) in [(&mut text_rows, true), (&mut hex_rows, false)] {
                if !mode_rows.is_empty() {
                    mode_rows.push(StreamRow::Gap {
                        start: transfer.start,
                        end: transfer.start,
                    });
                }
                for piece in pieces(&values, text) {
                    match piece {
                        Piece::Run { from, to } => mode_rows.push(StreamRow::Run {
                            start: words[from].start,
                            end: words[to - 1].end,
                            line: Some(*line),
                            offset: from as u32,
                            value: values[from],
                            count: (to - from) as u32,
                            bits,
                        }),
                        Piece::Text { from, to } => {
                            // One row per text line.
                            let mut row_from = from;
                            for index in from..to {
                                if values[index] == u32::from(b'\n') || index + 1 == to {
                                    mode_rows.push(StreamRow::SpiText {
                                        start: words[row_from].start,
                                        end: words[index].end,
                                        line: *line,
                                        offset: row_from as u32,
                                        text: values[row_from..=index]
                                            .iter()
                                            .map(|&v| char::from_u32(v).unwrap_or('\u{fffd}'))
                                            .collect(),
                                    });
                                    row_from = index + 1;
                                }
                            }
                        }
                        Piece::Other { from, to } => {
                            let mut row_from = from;
                            while row_from < to {
                                // Rows align to multiples of 16 within the transfer.
                                let row_to = ((row_from / HEX_WIDTH + 1) * HEX_WIDTH).min(to);
                                mode_rows.push(StreamRow::Hex {
                                    start: words[row_from].start,
                                    end: words[row_to - 1].end,
                                    line: Some(*line),
                                    offset: row_from as u32,
                                    values: values[row_from..row_to].to_vec(),
                                    bits,
                                });
                                row_from = row_to;
                            }
                        }
                    }
                }
            }
        }
    }
    let [(_, miso), (_, mosi)] = lanes;
    let words = trace.transfers.iter().map(|t| t.words.len()).sum::<usize>();
    Presentation {
        bus: Bus::Spi,
        lanes: vec![
            ("header".to_owned(), Lane::new(header)),
            ("miso".to_owned(), Lane::new(miso)),
            ("mosi".to_owned(), Lane::new(mosi)),
        ],
        text_rows,
        hex_rows,
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
        addresses: Vec::new(),
        transfer_words,
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
        addresses: Vec::new(),
        transfer_words: Vec::new(),
        search: Searchable::default(),
    }
}
