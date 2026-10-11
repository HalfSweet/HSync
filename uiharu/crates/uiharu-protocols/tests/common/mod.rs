// SPDX-License-Identifier: Apache-2.0

//! The SPI → SPI NOR pipeline on the generic test harness, with the events as readable lines.

#![allow(dead_code)]

use uiharu_core::testing::{self, Tap};
use uiharu_core::{
    GraphConfig, GraphError, Item, LogicBlock, Pipeline, SignalItem, Tick, TickRange, TimeBase,
};
use uiharu_protocols::spi::{Spi, SpiConfig, SpiEvent};
use uiharu_protocols::spi_nor::{NorEvent, SpiNorRead24};

#[allow(unused_imports)]
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
        Self::build(config, false, GraphConfig::default()).expect("builds")
    }

    /// Source → SPI → SPI NOR.
    pub fn nor(config: SpiConfig) -> Self {
        Self::build(config, true, GraphConfig::default()).expect("builds")
    }

    pub fn build(
        config: SpiConfig,
        nor: bool,
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
                if nor {
                    let nor = graph.add(SpiNorRead24::new());
                    graph.connect(Spi::events(spi), SpiNorRead24::input(nor))?;
                    nor_ref = Some(graph.expose("nor", SpiNorRead24::operations(nor))?);
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

    pub fn spi_lines(&self) -> Vec<String> {
        describe_spi(self.spi_items())
    }

    pub fn nor_lines(&self) -> Vec<String> {
        describe_nor(&self.nor_items())
    }
}

fn hex(value: Option<u8>) -> String {
    value.map_or("--".to_owned(), |value| format!("{value:02x}"))
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
                    NorEvent::Address { op, address } => {
                        format!("address op{} {address:06x}", op.0)
                    }
                    NorEvent::Data { op, offset, bytes } => {
                        let bytes: Vec<String> = bytes.iter().map(|b| format!("{b:02x}")).collect();
                        format!("data op{} +{offset} {}", op.0, bytes.join(""))
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
