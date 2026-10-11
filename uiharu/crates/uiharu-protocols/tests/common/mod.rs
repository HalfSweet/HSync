// SPDX-License-Identifier: Apache-2.0

//! A harness around a source → SPI (→ SPI NOR) pipeline: feeds samples in blocks of any size,
//! with gaps, and collects what the exposed outputs carry.

#![allow(dead_code)]

use std::sync::Arc;

use uiharu_core::{
    Exposed, Graph, GraphConfig, GraphError, Item, LogicBlock, Pipeline, PushError, SignalItem,
    SourceId, SourceSpec, Tick, TickRange, TimeBase,
};
use uiharu_protocols::spi::{Spi, SpiConfig, SpiEvent};
use uiharu_protocols::spi_nor::{NorEvent, SpiNorRead24};

pub const CHANNELS: u8 = 4;
pub const RATE: u64 = 1_000_000;

pub struct Harness {
    pub pipeline: Pipeline,
    pub source: SourceId,
    pub spi: Exposed<SpiEvent>,
    pub nor: Option<Exposed<NorEvent>>,
    pub spi_items: Vec<Item<SpiEvent>>,
    pub nor_items: Vec<Item<NorEvent>>,
}

impl Harness {
    /// Source → SPI, with the SPI events exposed.
    pub fn spi(config: SpiConfig) -> Self {
        Self::build(config, false, GraphConfig::default()).expect("builds")
    }

    /// Source → SPI → SPI NOR, with both outputs exposed.
    pub fn nor(config: SpiConfig) -> Self {
        Self::build(config, true, GraphConfig::default()).expect("builds")
    }

    pub fn build(
        config: SpiConfig,
        nor: bool,
        graph_config: GraphConfig,
    ) -> Result<Self, GraphError> {
        let mut graph = Graph::with_config(graph_config);
        let source = graph.source(SourceSpec::new("logic", CHANNELS, TimeBase::new(RATE)))?;
        let spi = graph.add(Spi::new(config).expect("a valid configuration"));
        graph.connect(source.samples, Spi::input(spi))?;
        let spi_out = graph.expose("spi", Spi::events(spi))?;
        let nor_out = if nor {
            let nor = graph.add(SpiNorRead24::new());
            graph.connect(Spi::events(spi), SpiNorRead24::input(nor))?;
            Some(graph.expose("nor", SpiNorRead24::operations(nor))?)
        } else {
            None
        };
        let mut pipeline = graph.build()?;
        let spi = pipeline.exposed(spi_out).expect("the spi reader");
        let nor = nor_out.map(|nor| pipeline.exposed(nor).expect("the nor reader"));
        Ok(Self {
            pipeline,
            source: source.id,
            spi,
            nor,
            spi_items: Vec::new(),
            nor_items: Vec::new(),
        })
    }

    /// Runs the pipeline and collects everything the outputs hold.
    pub fn collect(&mut self) {
        self.pipeline.run();
        collect_into(&self.spi, &mut self.spi_items);
        if let Some(nor) = &self.nor {
            collect_into(nor, &mut self.nor_items);
        }
    }

    /// Pushes `item`, running and collecting first when the source is full.
    pub fn push(&mut self, item: SignalItem) {
        let mut item = item;
        loop {
            match self.pipeline.push(self.source, item) {
                Ok(()) => return,
                Err(PushError::Full(back)) => {
                    item = back;
                    self.collect();
                }
                Err(other) => panic!("push failed: {other}"),
            }
        }
    }

    /// Feeds `block` in blocks of `chunk` samples.
    pub fn feed(&mut self, block: &LogicBlock, chunk: u64) {
        let mut at = block.start();
        while at < block.end() {
            let end = at.offset(chunk).min(block.end());
            let piece = block.slice(TickRange::new(at, end)).expect("a slice");
            self.push(SignalItem::Data(Arc::new(piece)));
            at = end;
        }
    }

    /// Feeds `block` in blocks whose sizes cycle through `sizes`.
    pub fn feed_sizes(&mut self, block: &LogicBlock, sizes: &[u64]) {
        let mut at = block.start();
        let mut cut = 0;
        while at < block.end() {
            let size = sizes[cut % sizes.len()].max(1);
            cut += 1;
            let end = at.offset(size).min(block.end());
            let piece = block.slice(TickRange::new(at, end)).expect("a slice");
            self.push(SignalItem::Data(Arc::new(piece)));
            at = end;
        }
    }

    /// Feeds `block` leaving out `gap`, which is announced as such.
    pub fn feed_with_gap(&mut self, block: &LogicBlock, gap: TickRange, chunk: u64) {
        let before = block.slice(TickRange::new(block.start(), gap.start));
        let after = block.slice(TickRange::new(gap.end, block.end()));
        if let Some(before) = before {
            self.feed(&before, chunk);
        }
        self.push(SignalItem::Gap(gap));
        if let Some(after) = after {
            self.feed(&after, chunk);
        }
    }

    /// Ends the samples where they stand and runs to the end.
    pub fn end(&mut self) {
        let at = self.pipeline.next_tick(self.source).expect("the source");
        self.push(SignalItem::End(at));
        self.collect();
        assert!(self.pipeline.is_finished(), "{:?}", self.pipeline.run());
    }

    /// Feeds the whole block in one piece and ends.
    pub fn run_all(&mut self, block: &LogicBlock) {
        self.push(SignalItem::Data(Arc::new(block.clone())));
        self.end();
    }

    pub fn spi_lines(&self) -> Vec<String> {
        describe_spi(&self.spi_items)
    }

    pub fn nor_lines(&self) -> Vec<String> {
        describe_nor(&self.nor_items)
    }
}

pub fn collect_into<T: Clone>(exposed: &Exposed<T>, into: &mut Vec<Item<T>>) {
    while let Some(item) = exposed.next() {
        let end = item.is_end();
        into.push(item);
        if end {
            break;
        }
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

/// Drops progress items, which depend on how the samples were cut.
pub fn without_progress<T: Clone>(items: &[Item<T>]) -> Vec<Item<T>> {
    items
        .iter()
        .filter(|item| !matches!(item, Item::Progress(_)))
        .cloned()
        .collect()
}
