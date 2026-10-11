// SPDX-License-Identifier: Apache-2.0

//! The SPI → SPI NOR pipeline on the generic test harness, with the events as readable lines.

#![allow(dead_code)]

use uiharu_core::testing::{self, Tap};
use uiharu_core::{
    GraphConfig, GraphError, Item, LogicBlock, Pipeline, SignalItem, Tick, TickRange, TimeBase,
};
use uiharu_protocols::spi::{Spi, SpiConfig, SpiEvent};
use uiharu_protocols::spi_nor::{Direction, NorEvent, SpiNor, SpiNorConfig};

pub use uiharu_core::testing::without_progress;

pub const CHANNELS: u8 = 4;
pub const RATE: u64 = 1_000_000;

/// Source → SPI, and SPI → SPI NOR when asked; the SPI events are the output, the NOR
/// operations a tap.
pub struct Harness {
    inner: testing::Harness<SpiEvent>,
    nor: Option<Tap<NorEvent>>,
}

impl Harness {
    /// Source → SPI.
    pub fn spi(config: SpiConfig) -> Self {
        Self::build(config, None, GraphConfig::default()).expect("builds")
    }

    /// Source → SPI → SPI NOR, addresses three bytes wide.
    pub fn nor(config: SpiConfig) -> Self {
        Self::nor_with(config, SpiNorConfig::default())
    }

    /// Source → SPI → SPI NOR configured as `nor`.
    pub fn nor_with(config: SpiConfig, nor: SpiNorConfig) -> Self {
        Self::build(config, Some(nor), GraphConfig::default()).expect("builds")
    }

    pub fn build(
        config: SpiConfig,
        nor: Option<SpiNorConfig>,
        graph_config: GraphConfig,
    ) -> Result<Self, GraphError> {
        let mut nor_ref = None;
        let mut inner = testing::Harness::build(
            CHANNELS,
            TimeBase::new(RATE),
            graph_config,
            |graph, source| {
                let spi = graph.add(Spi::new(config).expect("a valid configuration"));
                graph.connect(source.samples, Spi::input(spi))?;
                if let Some(nor) = nor {
                    let nor = graph.add(SpiNor::new(nor));
                    graph.connect(Spi::events(spi), SpiNor::input(nor))?;
                    nor_ref = Some(graph.expose("nor", SpiNor::events(nor))?);
                }
                Ok(Spi::events(spi))
            },
        )?;
        let nor = nor_ref.map(|exposed| inner.tap(exposed));
        Ok(Self { inner, nor })
    }

    /// How many items a push reads when the source is full.
    pub fn drain_on_full(&mut self, count: usize) {
        self.inner.drain_on_full = count;
    }

    pub fn pipeline(&self) -> &Pipeline {
        self.inner.pipeline()
    }

    pub fn collect(&mut self) {
        self.inner.collect();
    }

    pub fn collect_some(&mut self, count: usize) {
        self.inner.collect_some(count);
    }

    pub fn push(&mut self, item: SignalItem) {
        self.inner.push(item);
    }

    /// Feeds `block` in blocks of `chunk` samples.
    pub fn feed(&mut self, block: &LogicBlock, chunk: u64) {
        self.inner.feed(block, &[chunk]);
    }

    /// Feeds `block` in blocks whose sizes cycle through `sizes`.
    pub fn feed_sizes(&mut self, block: &LogicBlock, sizes: &[u64]) {
        self.inner.feed(block, sizes);
    }

    /// Feeds `block` leaving out `gap`, which is announced as such.
    pub fn feed_with_gap(&mut self, block: &LogicBlock, gap: TickRange, chunk: u64) {
        self.inner.feed_with_gaps(block, &[gap], &[chunk]);
    }

    pub fn end(&mut self) {
        self.inner.end();
    }

    pub fn run_all(&mut self, block: &LogicBlock) {
        self.inner.run_all(block);
    }

    pub fn spi_items(&self) -> &[Item<SpiEvent>] {
        &self.inner.items
    }

    pub fn nor_items(&self) -> Vec<Item<NorEvent>> {
        self.nor.as_ref().map(Tap::items).unwrap_or_default()
    }

    /// The SPI items as lines, progress left out: it depends on how the samples were cut.
    pub fn spi_lines(&self) -> Vec<String> {
        describe_spi(&without_progress(self.spi_items()))
    }

    /// The SPI items as lines, progress included.
    pub fn spi_lines_with_progress(&self) -> Vec<String> {
        describe_spi(self.spi_items())
    }

    /// The SPI NOR items as lines, progress left out.
    pub fn nor_lines(&self) -> Vec<String> {
        describe_nor(&without_progress(&self.nor_items()))
    }
}

fn hex(value: Option<u8>) -> String {
    value.map_or("--".to_owned(), |value| format!("{value:02x}"))
}

/// Bytes as hex digits, two per byte.
pub fn hex_bytes(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn at(span: TickRange, settled: Tick) -> String {
    if span.is_empty() {
        format!("@{settled}")
    } else {
        format!("[{},{})@{settled}", span.start, span.end)
    }
}

/// One line per item, readable in assertions.
pub fn describe_spi(items: &[Item<SpiEvent>]) -> Vec<String> {
    items
        .iter()
        .map(|item| match item {
            Item::Event(event) => {
                let what = match event.payload {
                    SpiEvent::Begin { tx, partial } => {
                        format!("begin tx{}{}", tx.0, if partial { " partial" } else { "" })
                    }
                    SpiEvent::Word { tx, mosi, miso } => {
                        format!("word tx{} {}/{}", tx.0, hex(mosi), hex(miso))
                    }
                    SpiEvent::End { tx, reason } => format!("end tx{} {reason:?}", tx.0),
                };
                format!("{} {}", what, at(event.span, event.settled))
            }
            Item::Gap(range) => format!("gap {range}"),
            Item::Progress(tick) => format!("progress {tick}"),
            Item::End(tick) => format!("eof {tick}"),
        })
        .collect()
}

pub fn describe_nor(items: &[Item<NorEvent>]) -> Vec<String> {
    items
        .iter()
        .map(|item| match item {
            Item::Event(event) => {
                let what = match &event.payload {
                    NorEvent::Begin { op, tx, command } => {
                        format!("begin op{} tx{} {command:?}", op.0, tx.0)
                    }
                    NorEvent::Address { op, address, bytes } => {
                        let digits = 2 * usize::from(*bytes);
                        format!("address op{} {address:0digits$x}", op.0)
                    }
                    NorEvent::Dummy { op, bytes } => format!("dummy op{} {bytes}", op.0),
                    NorEvent::Data {
                        op,
                        direction,
                        offset,
                        bytes,
                    } => {
                        let direction = match direction {
                            Direction::In => "in",
                            Direction::Out => "out",
                        };
                        format!("data op{} {direction} +{offset} {}", op.0, hex_bytes(bytes))
                    }
                    NorEvent::Status {
                        op,
                        register,
                        value,
                        written,
                    } => {
                        let how = if *written { "write" } else { "read" };
                        format!("status op{} {how} sr{register} {value:02x}", op.0)
                    }
                    NorEvent::Id { op, scheme, bytes } => {
                        format!("id op{} {scheme:?} {}", op.0, hex_bytes(bytes))
                    }
                    NorEvent::End { op, reason } => format!("end op{} {reason:?}", op.0),
                    NorEvent::UnknownCommand { tx, opcode } => {
                        format!("unknown tx{} {opcode:02x}", tx.0)
                    }
                };
                format!("{} {}", what, at(event.span, event.settled))
            }
            Item::Gap(range) => format!("gap {range}"),
            Item::Progress(tick) => format!("progress {tick}"),
            Item::End(tick) => format!("eof {tick}"),
        })
        .collect()
}
