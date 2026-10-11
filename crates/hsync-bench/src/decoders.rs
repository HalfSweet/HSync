//! The decoders under measurement, each run over a [`Source`] with the decoding alone timed:
//! building the samples in the decoder's layout happens before the clock starts, reading the
//! results happens before it stops, as an application would have to do both.

use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Instant;

use hsync_core::ChannelIndex;
use hsync_decode::spi;
use hsync_srd::{DecoderHost, Layer, Stack, run_stack};
use parking_lot::RwLock;
use serde_json::{Map, json};
use uiharu_core::{
    Exposed, Graph, Pipeline, PushError, SignalItem, SourceId, SourceSpec, Tick, TimeBase,
};
use uiharu_protocols::spi::{Spi, SpiEvent};
use uiharu_protocols::spi_nor::{NorEvent, SpiNor, SpiNorConfig};

use crate::workload::Source;

/// A decoder under measurement.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decoder {
    /// uiharu's SPI decoder.
    UiharuSpi,
    /// uiharu's SPI decoder with SPI NOR stacked on it.
    UiharuSpiNor,
    /// `hsync-decode`'s native SPI decoder, a Rust port of DSView's.
    HsyncDecodeSpi,
    /// DSView's own Python `1:spi` decoder in the embedded interpreter (`hsync-srd`): the
    /// sigrok decoder as the app runs it today.
    SigrokSpi,
    /// `sigrok-cli`, when it is installed.
    SigrokCliSpi,
}

impl Decoder {
    pub const ALL: [Decoder; 5] = [
        Decoder::UiharuSpi,
        Decoder::UiharuSpiNor,
        Decoder::HsyncDecodeSpi,
        Decoder::SigrokSpi,
        Decoder::SigrokCliSpi,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Decoder::UiharuSpi => "uiharu-spi",
            Decoder::UiharuSpiNor => "uiharu-spi-nor",
            Decoder::HsyncDecodeSpi => "hsync-decode-spi",
            Decoder::SigrokSpi => "sigrok-spi",
            Decoder::SigrokCliSpi => "sigrok-cli-spi",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|decoder| decoder.name() == name)
    }

    /// Whether the decoder walks the samples in Python and so gets a shorter workload.
    pub fn is_python(self) -> bool {
        matches!(self, Decoder::SigrokSpi | Decoder::SigrokCliSpi)
    }
}

/// What a decoder did on a workload.
#[derive(Debug)]
pub struct Measured {
    pub samples: u64,
    /// SPI words decoded.
    pub words: u64,
    /// Everything the decoder put out: uiharu's events, the native decoder's words and
    /// transfers, the Python decoder's annotations (which include one per bit).
    pub events: u64,
    pub seconds: f64,
    /// The resident set just before and just after decoding, where the platform tells.
    pub rss_before: Option<u64>,
    pub rss_after: Option<u64>,
}

/// Why a case has no measurement.
#[derive(Debug)]
pub enum Outcome {
    Measured(Measured),
    Skipped(String),
}

/// The resident set size of this process right now, where the platform tells.
pub fn current_rss() -> Option<u64> {
    #[cfg(target_os = "linux")]
    {
        let status = std::fs::read_to_string("/proc/self/status").ok()?;
        let line = status.lines().find(|line| line.starts_with("VmRSS:"))?;
        let kib: u64 = line.split_whitespace().nth(1)?.parse().ok()?;
        return Some(kib * 1024);
    }
    #[cfg(target_os = "macos")]
    {
        let output = Command::new("ps")
            .args(["-o", "rss=", "-p", &std::process::id().to_string()])
            .output()
            .ok()?;
        let kib: u64 = String::from_utf8_lossy(&output.stdout)
            .trim()
            .parse()
            .ok()?;
        return Some(kib * 1024);
    }
    #[allow(unreachable_code)]
    None
}

/// Runs `decoder` over `source`; uiharu gets the samples in blocks of `chunk`.
pub fn run(decoder: Decoder, source: &Source, chunk: u64, root: &Path) -> Result<Outcome, String> {
    match decoder {
        Decoder::UiharuSpi => uiharu(source, chunk, false).map(Outcome::Measured),
        Decoder::UiharuSpiNor => uiharu(source, chunk, true).map(Outcome::Measured),
        Decoder::HsyncDecodeSpi => native(source).map(Outcome::Measured),
        Decoder::SigrokSpi => python(source, root).map(Outcome::Measured),
        Decoder::SigrokCliSpi => Ok(match sigrok_cli_version() {
            Some(version) => Outcome::Skipped(format!(
                "{version} is installed but the harness does not drive it yet"
            )),
            None => Outcome::Skipped("sigrok-cli is not installed".to_owned()),
        }),
    }
}

/// Reads everything `reader` holds, counting events and SPI words; stops at the end.
fn drain_spi(reader: &Exposed<SpiEvent>, ended: &mut bool, events: &mut u64, words: &mut u64) {
    while !*ended {
        let Some((end, is_event, is_word)) = reader.with_next(|item| {
            let event = item.event();
            (
                item.is_end(),
                event.is_some(),
                event.is_some_and(|event| matches!(event.payload, SpiEvent::Word { .. })),
            )
        }) else {
            break;
        };
        *events += u64::from(is_event);
        *words += u64::from(is_word);
        reader.advance();
        *ended = end;
    }
}

/// Reads everything `reader` holds, counting events; stops at the end.
fn drain_nor(reader: &Exposed<NorEvent>, ended: &mut bool, events: &mut u64) {
    while !*ended {
        let Some((end, is_event)) =
            reader.with_next(|item| (item.is_end(), item.event().is_some()))
        else {
            break;
        };
        *events += u64::from(is_event);
        reader.advance();
        *ended = end;
    }
}

/// Pushes `item`, running the pipeline and reading its outputs while the source is full.
fn push(
    pipeline: &mut Pipeline,
    source: SourceId,
    item: SignalItem,
    drain: &mut impl FnMut(&mut Pipeline),
) -> Result<(), String> {
    let mut item = item;
    loop {
        match pipeline.push(source, item) {
            Ok(()) => return Ok(()),
            Err(PushError::Full(back)) => {
                item = back;
                drain(pipeline);
            }
            Err(error) => return Err(error.to_string()),
        }
    }
}

fn uiharu(source: &Source, chunk: u64, nor: bool) -> Result<Measured, String> {
    let blocks = source.blocks(chunk);
    let config = source.bus.spi_config();
    let samples = source.samples();
    let rss_before = current_rss();
    let started = Instant::now();

    let mut graph = Graph::new();
    let logic = graph
        .source(SourceSpec::new(
            "logic",
            source.channels,
            TimeBase::new(source.rate_hz),
        ))
        .map_err(|error| error.to_string())?;
    let spi = graph.add(Spi::new(config).map_err(|error| error.to_string())?);
    graph
        .connect(logic.samples, Spi::input(spi))
        .map_err(|error| error.to_string())?;
    let spi_out = graph
        .expose("spi", Spi::events(spi))
        .map_err(|error| error.to_string())?;
    let nor_out = if nor {
        let node = graph.add(SpiNor::new(SpiNorConfig::default()));
        graph
            .connect(Spi::events(spi), SpiNor::input(node))
            .map_err(|error| error.to_string())?;
        Some(
            graph
                .expose("nor", SpiNor::events(node))
                .map_err(|error| error.to_string())?,
        )
    } else {
        None
    };
    let mut pipeline = graph.build().map_err(|error| error.to_string())?;
    let spi_reader = pipeline
        .exposed(spi_out)
        .ok_or("the spi output was exposed once")?;
    let nor_reader = nor_out.and_then(|out| pipeline.exposed(out));

    let (mut spi_ended, mut nor_ended) = (false, false);
    let (mut words, mut events) = (0u64, 0u64);
    let mut drain = |pipeline: &mut Pipeline| {
        pipeline.run();
        drain_spi(&spi_reader, &mut spi_ended, &mut events, &mut words);
        if let Some(reader) = &nor_reader {
            drain_nor(reader, &mut nor_ended, &mut events);
        }
    };
    for block in blocks {
        push(&mut pipeline, logic.id, SignalItem::Data(block), &mut drain)?;
        drain(&mut pipeline);
    }
    push(
        &mut pipeline,
        logic.id,
        SignalItem::End(Tick(samples)),
        &mut drain,
    )?;
    while !pipeline.is_finished() {
        drain(&mut pipeline);
    }
    let seconds = started.elapsed().as_secs_f64();
    Ok(Measured {
        samples,
        words,
        events,
        seconds,
        rss_before,
        rss_after: current_rss(),
    })
}

fn native(source: &Source) -> Result<Measured, String> {
    let capture = source.capture();
    let bus = source.bus;
    let channel = |index: ChannelIndex| {
        capture
            .channel(index)
            .ok_or_else(|| format!("channel {index} is missing"))
    };
    let lines = spi::Lines {
        clk: channel(bus.clk)?,
        miso: Some(channel(bus.miso)?),
        mosi: Some(channel(bus.mosi)?),
        cs: Some(channel(bus.cs)?),
    };
    // Mode 0, eight-bit words, most significant bit first, chip select active low.
    let options = spi::Options::default();
    let rss_before = current_rss();
    let started = Instant::now();
    let trace = spi::decode(&lines, 0..capture.len(), &options);
    let seconds = started.elapsed().as_secs_f64();
    Ok(Measured {
        samples: capture.len(),
        words: trace.words.len() as u64,
        events: (trace.words.len() + trace.transfers.len()) as u64,
        seconds,
        rss_before,
        rss_after: current_rss(),
    })
}

fn python(source: &Source, root: &Path) -> Result<Measured, String> {
    let capture = Arc::new(RwLock::new(source.capture()));
    let len = capture.read().len();
    let host = DecoderHost::new(root.join("src-tauri/resources/decoders"));
    // Starts the interpreter and imports every decoder, before the clock.
    host.catalog().map_err(|error| error.to_string())?;
    let info = host.info("1:spi").map_err(|error| error.to_string())?;
    let mosi_data = info
        .annotations
        .iter()
        .position(|class| class.id == "mosi-data");
    let mut options = Map::new();
    options.insert("bitorder".to_owned(), json!("msb-first"));
    options.insert("cpha".to_owned(), json!(0));
    options.insert("cpol".to_owned(), json!(0));
    options.insert("cs_polarity".to_owned(), json!("active-low"));
    options.insert("frame".to_owned(), json!("yes"));
    options.insert("wordsize".to_owned(), json!(8));
    let bus = source.bus;
    let channels: BTreeMap<String, ChannelIndex> = [
        ("clk", bus.clk),
        ("cs", bus.cs),
        ("mosi", bus.mosi),
        ("miso", bus.miso),
    ]
    .into_iter()
    .map(|(role, index)| (role.to_owned(), index))
    .collect();
    let stack = Stack {
        layers: vec![Layer {
            id: "1:spi".to_owned(),
            options,
        }],
        channels,
    };
    let cancel = Arc::new(AtomicBool::new(false));
    let rss_before = current_rss();
    let started = Instant::now();
    let outcome =
        run_stack(&host, &capture, 0..len, &stack, &cancel).map_err(|error| error.to_string())?;
    let seconds = started.elapsed().as_secs_f64();
    if let Some(error) = outcome.error {
        return Err(error.to_string());
    }
    let layer = outcome
        .layers
        .first()
        .ok_or("the decoder produced no annotations")?;
    if let Some(error) = &layer.error {
        return Err(error.clone());
    }
    let words = mosi_data.map_or(0, |class| {
        layer
            .annotations
            .iter()
            .filter(|annotation| annotation.class as usize == class)
            .count() as u64
    });
    Ok(Measured {
        samples: len,
        words,
        events: layer.annotations.len() as u64,
        seconds,
        rss_before,
        rss_after: current_rss(),
    })
}

/// The first line of `sigrok-cli --version`, when it is installed.
fn sigrok_cli_version() -> Option<String> {
    let output = Command::new("sigrok-cli").arg("--version").output().ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    text.lines().next().map(str::trim).map(str::to_owned)
}
