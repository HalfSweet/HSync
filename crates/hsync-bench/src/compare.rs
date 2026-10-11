//! Cross-validation of uiharu's decoders against HSync's native decoders on a real capture:
//! DSView's protocol demo carries I²C, UART, CAN FD and SPI traffic from real devices, and
//! `hsync-decode` reads it with implementations that share nothing with uiharu. Both decode the
//! same channels; the results are reduced to a common shape and compared item by item.
//!
//! The two engines place their spans by their own conventions, so values are compared strictly
//! and spans are reported: the largest difference between the two engines' start and end of
//! matching items, in samples.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;
use std::sync::Arc;

use hsync_capture::LogicCapture;
use hsync_core::ChannelIndex;
use hsync_decode::{can, i2c, spi, uart};
use uiharu_core::{
    Event, EventSchema, ExposedRef, Graph, LogicBlock, Node, Pipeline, PushError, SignalItem,
    SourceHandle, SourceSpec, Tick, TimeBase,
};
use uiharu_protocols::can::{Can, CanConfig, CanEvent};
use uiharu_protocols::i2c::{EndReason as I2cEnd, I2c, I2cConfig, I2cEvent};
use uiharu_protocols::spi::{Spi, SpiConfig, SpiEvent};
use uiharu_protocols::uart::{Uart, UartConfig, UartEvent};

use crate::workload::Source;

/// Where the demo's buses are, as DSView's decoder entries in the file have them.
mod demo {
    pub const SDA: u8 = 0;
    pub const SCL: u8 = 1;
    pub const UART: u8 = 5;
    pub const CAN: u8 = 9;
    pub const CLK: u8 = 12;
    pub const CS: u8 = 13;
    pub const MOSI: u8 = 14;
    pub const MISO: u8 = 15;
    pub const UART_BAUD: u32 = 115_200;
    pub const CAN_BITRATE: u32 = 500_000;
    pub const CAN_FD_BITRATE: u32 = 2_000_000;
    pub const CAN_SAMPLE_POINT: u8 = 70;
}

/// One decoded item in the shape both engines are reduced to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub start: u64,
    pub end: u64,
    /// The item's meaning, spelt the same way by both engines.
    pub text: String,
}

/// How the two engines' results of one bus compare.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    pub bus: &'static str,
    pub uiharu: Vec<Item>,
    pub native: Vec<Item>,
    /// Items whose text differs, or that one engine has and the other lacks: `(index, uiharu,
    /// native)`.
    pub mismatches: Vec<(usize, Option<String>, Option<String>)>,
    /// The largest difference in samples between the two engines' span starts of matching items.
    pub max_start_delta: u64,
    /// The same for span ends.
    pub max_end_delta: u64,
}

impl Report {
    fn new(bus: &'static str, uiharu: Vec<Item>, native: Vec<Item>) -> Self {
        let mut mismatches = Vec::new();
        let (mut max_start_delta, mut max_end_delta) = (0, 0);
        for index in 0..uiharu.len().max(native.len()) {
            match (uiharu.get(index), native.get(index)) {
                (Some(a), Some(b)) if a.text == b.text => {
                    max_start_delta = max_start_delta.max(a.start.abs_diff(b.start));
                    max_end_delta = max_end_delta.max(a.end.abs_diff(b.end));
                }
                (a, b) => mismatches.push((
                    index,
                    a.map(|item| item.text.clone()),
                    b.map(|item| item.text.clone()),
                )),
            }
        }
        Self {
            bus,
            uiharu,
            native,
            mismatches,
            max_start_delta,
            max_end_delta,
        }
    }

    pub fn agrees(&self) -> bool {
        self.mismatches.is_empty()
    }

    /// The report as lines for a terminal: a summary, then the first mismatches.
    pub fn render(&self) -> String {
        let mut out = String::new();
        let _ = writeln!(
            out,
            "{:<5} uiharu {:>5} items, native {:>5} items, {} mismatches, spans within \
             {} / {} samples",
            self.bus,
            self.uiharu.len(),
            self.native.len(),
            self.mismatches.len(),
            self.max_start_delta,
            self.max_end_delta
        );
        for (index, a, b) in self.mismatches.iter().take(20) {
            let _ = writeln!(
                out,
                "      #{index}: uiharu {}  |  native {}",
                a.as_deref().unwrap_or("-"),
                b.as_deref().unwrap_or("-")
            );
        }
        if self.mismatches.len() > 20 {
            let _ = writeln!(out, "      … {} more", self.mismatches.len() - 20);
        }
        out
    }
}

/// Runs `wire`'s node over the demo capture in blocks of `chunk` samples and collects the
/// events of the output it exposes.
fn run_uiharu<T: EventSchema + Clone>(
    source: &Source,
    chunk: u64,
    wire: impl FnOnce(&mut Graph, &SourceHandle) -> Result<ExposedRef<T>, String>,
) -> Result<Vec<Event<T>>, String> {
    let mut graph = Graph::new();
    let logic = graph
        .source(SourceSpec::new(
            "logic",
            source.channels,
            TimeBase::new(source.rate_hz),
        ))
        .map_err(|error| error.to_string())?;
    let exposed = wire(&mut graph, &logic)?;
    let mut pipeline = graph.build().map_err(|error| error.to_string())?;
    let reader = pipeline
        .exposed(exposed)
        .ok_or("the output was exposed once")?;
    let mut events = Vec::new();
    let mut drain = |pipeline: &mut Pipeline| {
        pipeline.run();
        while !reader.is_ended() {
            let Some(item) = reader.next() else {
                break;
            };
            events.extend(item.into_event());
        }
    };
    let push = |pipeline: &mut Pipeline,
                mut item: SignalItem,
                drain: &mut dyn FnMut(&mut Pipeline)| loop {
        match pipeline.push(logic.id, item) {
            Ok(()) => return Ok(()),
            Err(PushError::Full(back)) => {
                item = back;
                drain(pipeline);
            }
            Err(error) => return Err(error.to_string()),
        }
    };
    for block in source.blocks(chunk) {
        push(&mut pipeline, SignalItem::Data(block), &mut drain)?;
        drain(&mut pipeline);
    }
    push(
        &mut pipeline,
        SignalItem::End(Tick(source.samples())),
        &mut drain,
    )?;
    while !pipeline.is_finished() {
        drain(&mut pipeline);
    }
    Ok(events)
}

/// Adds `node` reading the source, exposing `output` of it.
fn single<N: Node + 'static, T: EventSchema>(
    graph: &mut Graph,
    logic: &SourceHandle,
    node: N,
    input: impl FnOnce(uiharu_core::NodeId) -> uiharu_core::InputRef<Arc<LogicBlock>>,
    output: impl FnOnce(uiharu_core::NodeId) -> uiharu_core::OutputRef<T>,
) -> Result<ExposedRef<T>, String> {
    let id = graph.add(node);
    graph
        .connect(logic.samples, input(id))
        .map_err(|error| error.to_string())?;
    graph
        .expose("events", output(id))
        .map_err(|error| error.to_string())
}

fn item<T>(event: &Event<T>, text: String) -> Item {
    Item {
        start: event.span.start.0,
        end: event.span.end.0,
        text,
    }
}

fn channel(capture: &LogicCapture, index: u8) -> Result<&hsync_capture::ChannelData, String> {
    capture
        .channel(ChannelIndex::from(index))
        .ok_or_else(|| format!("channel {index} is missing from the capture"))
}

fn hex(byte: u8) -> String {
    format!("{byte:02x}")
}

/// SPI: every word on both lines.
pub fn spi(source: &Source, capture: &LogicCapture, chunk: u64) -> Result<Report, String> {
    let config = SpiConfig {
        clk: demo::CLK,
        cs: Some(demo::CS),
        mosi: Some(demo::MOSI),
        miso: Some(demo::MISO),
        ..SpiConfig::default()
    };
    let events = run_uiharu(source, chunk, |graph, logic| {
        single(
            graph,
            logic,
            Spi::new(config.clone()).map_err(|error| error.to_string())?,
            Spi::input,
            Spi::events,
        )
    })?;
    let ours = events
        .iter()
        .filter_map(|event| match event.payload {
            SpiEvent::Word { mosi, miso, .. } => Some(item(
                event,
                format!(
                    "mosi {} miso {}",
                    mosi.map_or("-".to_owned(), hex),
                    miso.map_or("-".to_owned(), hex)
                ),
            )),
            _ => None,
        })
        .collect();
    let lines = spi::Lines {
        clk: channel(capture, demo::CLK)?,
        miso: Some(channel(capture, demo::MISO)?),
        mosi: Some(channel(capture, demo::MOSI)?),
        cs: Some(channel(capture, demo::CS)?),
    };
    let trace = spi::decode(&lines, 0..capture.len(), &spi::Options::default());
    let theirs = trace
        .words
        .iter()
        .map(|word| Item {
            start: word.start,
            end: word.end,
            text: format!(
                "mosi {} miso {}",
                word.mosi.map_or("-".to_owned(), |v| hex(v as u8)),
                word.miso.map_or("-".to_owned(), |v| hex(v as u8))
            ),
        })
        .collect();
    Ok(Report::new("spi", ours, theirs))
}

/// I²C: conditions, addresses and data with their acknowledges, as one token each.
///
/// `hsync-decode` lists the acknowledge bit as an item of its own after each byte; uiharu
/// carries it in the byte's event. Both are folded into `addr 50 w ack` / `data 12 nack`.
pub fn i2c(source: &Source, capture: &LogicCapture, chunk: u64) -> Result<Report, String> {
    let config = I2cConfig {
        scl: demo::SCL,
        sda: demo::SDA,
    };
    let events = run_uiharu(source, chunk, |graph, logic| {
        single(
            graph,
            logic,
            I2c::new(config).map_err(|error| error.to_string())?,
            I2c::input,
            I2c::events,
        )
    })?;
    let ack = |ack: bool| if ack { "ack" } else { "nack" };
    let ours = events
        .iter()
        .filter_map(|event| {
            let text = match &event.payload {
                I2cEvent::Begin { partial: false, .. } => "start".to_owned(),
                I2cEvent::Begin { partial: true, .. } => return None,
                I2cEvent::Restart { .. } => "restart".to_owned(),
                I2cEvent::Address {
                    address,
                    read,
                    ack: acked,
                    ..
                } => format!(
                    "addr {address:02x} {} {}",
                    if *read { "r" } else { "w" },
                    ack(*acked)
                ),
                I2cEvent::Data {
                    byte, ack: acked, ..
                } => format!("data {} {}", hex(*byte), ack(*acked)),
                I2cEvent::End {
                    reason: I2cEnd::Stop,
                    ..
                } => "stop".to_owned(),
                I2cEvent::End { .. } => return None,
            };
            Some(item(event, text))
        })
        .collect();
    let trace = i2c::decode(
        channel(capture, demo::SCL)?,
        channel(capture, demo::SDA)?,
        0..capture.len(),
    );
    let mut theirs: Vec<Item> = Vec::new();
    for entry in &trace.items {
        let text = match entry.kind {
            i2c::Kind::Start => "start".to_owned(),
            i2c::Kind::RepeatedStart => "restart".to_owned(),
            i2c::Kind::Stop => "stop".to_owned(),
            i2c::Kind::AddressWrite => format!("addr {:02x} w", entry.address()),
            i2c::Kind::AddressRead => format!("addr {:02x} r", entry.address()),
            i2c::Kind::DataWrite | i2c::Kind::DataRead => format!("data {}", hex(entry.byte)),
            i2c::Kind::Ack | i2c::Kind::Nack => {
                if let Some(last) = theirs.last_mut() {
                    last.text.push(' ');
                    last.text
                        .push_str(ack(matches!(entry.kind, i2c::Kind::Ack)));
                    last.end = entry.end;
                }
                continue;
            }
        };
        theirs.push(Item {
            start: entry.start,
            end: entry.end,
            text,
        });
    }
    Ok(Report::new("i2c", ours, theirs))
}

/// UART: every character with its parity and framing verdicts.
pub fn uart(source: &Source, capture: &LogicCapture, chunk: u64) -> Result<Report, String> {
    let config = UartConfig {
        line: demo::UART,
        baud: demo::UART_BAUD,
        ..UartConfig::default()
    };
    let events = run_uiharu(source, chunk, |graph, logic| {
        single(
            graph,
            logic,
            Uart::new(config).map_err(|error| error.to_string())?,
            Uart::input,
            Uart::events,
        )
    })?;
    let ours = events
        .iter()
        .filter_map(|event| match &event.payload {
            UartEvent::Frame {
                value, framing_ok, ..
            } => Some(item(
                event,
                format!(
                    "{:02x}{}",
                    value,
                    if *framing_ok { "" } else { " framing-error" }
                ),
            )),
            UartEvent::Break { .. } => None,
        })
        .collect();
    let options = uart::Options {
        baudrate: f64::from(demo::UART_BAUD),
        ..uart::Options::default()
    };
    let trace = uart::decode(
        channel(capture, demo::UART)?,
        0..capture.len(),
        capture.samplerate(),
        &options,
    );
    let theirs = trace
        .frames
        .iter()
        .map(|frame| Item {
            start: frame.start,
            end: frame.end,
            text: format!(
                "{:02x}{}",
                frame.value,
                if frame.framing_error {
                    " framing-error"
                } else {
                    ""
                }
            ),
        })
        .collect();
    Ok(Report::new("uart", ours, theirs))
}

/// A CAN frame as both engines describe it, assembled from uiharu's events per frame.
#[derive(Debug, Default)]
struct CanFrame {
    start: u64,
    end: u64,
    id: Option<(u32, bool)>,
    control: Option<(bool, bool, bool, bool, u8)>,
    data: Vec<u8>,
    crc_ok: Option<bool>,
    acked: Option<bool>,
    reason: Option<String>,
}

impl CanFrame {
    fn text(&self) -> String {
        let (id, extended) = self.id.unwrap_or((0, false));
        let (remote, fd, brs, esi, dlc) = self.control.unwrap_or_default();
        let mut text = format!(
            "id {id:x}{}{}{}{}{} dlc {dlc} data",
            if extended { " ext" } else { "" },
            if remote { " rtr" } else { "" },
            if fd { " fd" } else { "" },
            if brs { " brs" } else { "" },
            if esi { " esi" } else { "" }
        );
        for byte in &self.data {
            text.push(' ');
            text.push_str(&hex(*byte));
        }
        let _ = write!(
            text,
            " crc {} ack {}",
            match self.crc_ok {
                Some(true) => "ok",
                Some(false) => "bad",
                None => "-",
            },
            match self.acked {
                Some(true) => "yes",
                Some(false) => "no",
                None => "-",
            }
        );
        if let Some(reason) = &self.reason {
            let _ = write!(text, " end {reason}");
        }
        text
    }
}

/// CAN FD: every frame with its identifier, control bits, data, CRC and acknowledge.
pub fn can(source: &Source, capture: &LogicCapture, chunk: u64) -> Result<Report, String> {
    let config = CanConfig {
        rx: demo::CAN,
        bitrate: demo::CAN_BITRATE,
        fd_bitrate: Some(demo::CAN_FD_BITRATE),
        sample_point: demo::CAN_SAMPLE_POINT,
        ..CanConfig::default()
    };
    let events = run_uiharu(source, chunk, |graph, logic| {
        single(
            graph,
            logic,
            Can::new(config.clone()).map_err(|error| error.to_string())?,
            Can::input,
            Can::events,
        )
    })?;
    let mut frames: BTreeMap<u64, CanFrame> = BTreeMap::new();
    for event in &events {
        let (frame_id, partial) = match &event.payload {
            CanEvent::Begin { frame, partial } => (frame.0, *partial),
            CanEvent::Id { frame, .. }
            | CanEvent::Control { frame, .. }
            | CanEvent::Data { frame, .. }
            | CanEvent::Crc { frame, .. }
            | CanEvent::Ack { frame, .. }
            | CanEvent::End { frame, .. } => (frame.0, false),
        };
        if partial {
            continue;
        }
        let frame = frames.entry(frame_id).or_default();
        match &event.payload {
            CanEvent::Begin { .. } => frame.start = event.span.start.0,
            CanEvent::Id { id, extended, .. } => frame.id = Some((*id, *extended)),
            CanEvent::Control {
                remote,
                fd,
                brs,
                esi,
                dlc,
                ..
            } => frame.control = Some((*remote, *fd, *brs, *esi, *dlc)),
            CanEvent::Data { bytes, .. } => frame.data.extend_from_slice(bytes),
            CanEvent::Crc { ok, .. } => frame.crc_ok = Some(*ok),
            CanEvent::Ack { acked, .. } => frame.acked = Some(*acked),
            CanEvent::End { reason, .. } => {
                frame.end = event.span.end.0;
                frame.reason = Some(
                    serde_json::to_value(reason)
                        .ok()
                        .and_then(|value| value.as_str().map(str::to_owned))
                        .unwrap_or_default(),
                );
            }
        }
    }
    let ours = frames
        .values()
        .filter(|frame| frame.id.is_some())
        .map(|frame| Item {
            start: frame.start,
            end: frame.end,
            text: frame.text(),
        })
        .collect();
    let options = can::Options {
        nominal_bitrate: f64::from(demo::CAN_BITRATE),
        fast_bitrate: f64::from(demo::CAN_FD_BITRATE),
        sample_point: f64::from(demo::CAN_SAMPLE_POINT),
    };
    let trace = can::decode(
        channel(capture, demo::CAN)?,
        0..capture.len(),
        capture.samplerate(),
        &options,
    );
    let theirs = trace
        .frames
        .iter()
        .map(|frame| {
            let shaped = CanFrame {
                start: frame.start,
                end: frame.end,
                id: Some((frame.id, frame.extended)),
                control: Some((frame.remote, frame.fd, frame.brs, frame.esi, frame.dlc)),
                data: frame.data.clone(),
                crc_ok: Some(frame.crc_ok),
                acked: Some(frame.acked),
                reason: Some(match frame.error {
                    None => "eof".to_owned(),
                    Some(can::FrameError::Stuff) => "stuff-error".to_owned(),
                    Some(can::FrameError::Form) => "form-error".to_owned(),
                }),
            };
            Item {
                start: shaped.start,
                end: shaped.end,
                text: shaped.text(),
            }
        })
        .collect();
    Ok(Report::new("can", ours, theirs))
}

/// Compares every bus of the demo capture, uiharu fed in blocks of `chunk` samples.
pub fn demo(root: &Path, chunk: u64) -> Result<Vec<Report>, String> {
    let source = Source::demo(root, 1)?;
    let capture = source.capture();
    Ok(vec![
        spi(&source, &capture, chunk)?,
        i2c(&source, &capture, chunk)?,
        uart(&source, &capture, chunk)?,
        can(&source, &capture, chunk)?,
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    fn report(bus: &str) -> Report {
        let reports = demo(&root(), 4096).expect("the demo capture decodes");
        reports
            .into_iter()
            .find(|report| report.bus == bus)
            .expect("the bus is compared")
    }

    #[test]
    fn spi_agrees_with_the_native_decoder() {
        let report = report("spi");
        assert!(report.agrees(), "{}", report.render());
        assert!(report.uiharu.len() > 100, "{}", report.render());
    }

    #[test]
    fn i2c_agrees_with_the_native_decoder() {
        let report = report("i2c");
        assert!(report.agrees(), "{}", report.render());
        assert!(report.uiharu.len() > 10, "{}", report.render());
    }

    #[test]
    fn uart_agrees_with_the_native_decoder() {
        let report = report("uart");
        assert!(report.agrees(), "{}", report.render());
        assert!(report.uiharu.len() > 10, "{}", report.render());
    }

    #[test]
    fn can_agrees_with_the_native_decoder() {
        let report = report("can");
        assert!(report.agrees(), "{}", report.render());
        assert!(!report.uiharu.is_empty(), "{}", report.render());
    }
}
