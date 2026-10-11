// SPDX-License-Identifier: Apache-2.0

//! The decode pipeline the commands share: a source of `channels` channels feeding SPI, with
//! SPI NOR on top when asked, both outputs exposed.

use uiharu_core::{
    Exposed, Graph, GraphConfig, GraphError, Pipeline, SourceId, SourceSpec, TimeBase,
};
use uiharu_protocols::spi::{BitOrder, Spi, SpiConfig, SpiConfigError, SpiEvent};
use uiharu_protocols::spi_nor::{NorEvent, SpiNorRead24};

use crate::args::{Args, UsageError};

/// Which decoders the pipeline runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// SPI alone.
    Spi,
    /// SPI with SPI NOR on top.
    SpiNor,
}

/// The settings every command takes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Setup {
    pub kind: Kind,
    pub channels: u8,
    pub rate_hz: u64,
    pub spi: SpiConfig,
    pub graph: GraphConfig,
}

/// The options [`Setup::from_args`] reads.
pub const OPTIONS: &[&str] = &[
    "pipeline",
    "channels",
    "rate",
    "clk",
    "cs",
    "mosi",
    "miso",
    "mode",
    "word-bits",
    "lsb-first",
    "cs-active-high",
    "queue",
    "budget",
];

/// The flags among [`OPTIONS`].
pub const FLAGS: &[&str] = &["lsb-first", "cs-active-high"];

/// The options, as the usage text lists them.
pub const USAGE: &str = "\
      --pipeline <spi|spi-nor>  the decoders to run (default spi-nor)
      --channels <N>            channels in the capture, 1 to 64 (default 4)
      --rate <HZ>               sample rate (default 1000000)
      --clk <CH>                clock channel (default 0)
      --cs <CH|none>            chip select channel (default 1)
      --mosi <CH|none>          MOSI channel (default 2)
      --miso <CH|none>          MISO channel (default 3)
      --mode <0-3>              SPI mode (default 0)
      --word-bits <1-8>         bits per word (default 8)
      --lsb-first               shift the least significant bit first
      --cs-active-high          chip select is asserted high
      --queue <N>               items a queue between decoders holds (default 256)
      --budget <N>              operations a decoder runs per turn (default 1024)";

impl Setup {
    /// Reads the shared options.
    pub fn from_args(args: &Args) -> Result<Self, UsageError> {
        let kind = match args.value("pipeline").unwrap_or("spi-nor") {
            "spi" => Kind::Spi,
            "spi-nor" | "nor" => Kind::SpiNor,
            other => {
                return Err(UsageError(format!(
                    "--pipeline {other}: expected spi or spi-nor"
                )));
            }
        };
        let channels: u8 = args.parsed("channels", 4)?;
        if !(1..=64).contains(&channels) {
            return Err(UsageError(format!(
                "--channels {channels}: expected 1 to 64"
            )));
        }
        let spi = SpiConfig {
            clk: args.parsed("clk", 0)?,
            cs: args.channel("cs", Some(1))?,
            mosi: args.channel("mosi", Some(2))?,
            miso: args.channel("miso", Some(3))?,
            mode: args.parsed("mode", 0)?,
            word_bits: args.parsed("word-bits", 8)?,
            bit_order: if args.flag("lsb-first") {
                BitOrder::LsbFirst
            } else {
                BitOrder::MsbFirst
            },
            cs_active_low: !args.flag("cs-active-high"),
        };
        spi.validate()
            .map_err(|error: SpiConfigError| UsageError(error.to_string()))?;
        if let Some(channel) = spi.channels().find(|channel| *channel >= channels) {
            return Err(UsageError(format!(
                "channel {channel} is used but the capture has {channels} channels"
            )));
        }
        let graph = GraphConfig {
            queue_capacity: args.parsed("queue", 256)?,
            budget: args.parsed("budget", 1024)?,
        };
        Ok(Self {
            kind,
            channels,
            rate_hz: args.parsed("rate", 1_000_000)?,
            spi,
            graph,
        })
    }

    /// Builds the pipeline.
    pub fn build(&self) -> Result<Built, GraphError> {
        let mut graph = Graph::with_config(self.graph);
        let source = graph.source(SourceSpec::new(
            "logic",
            self.channels,
            TimeBase::new(self.rate_hz),
        ))?;
        let spi =
            graph.add(
                Spi::new(self.spi.clone()).map_err(|error| GraphError::Requirement {
                    from: "logic".into(),
                    to: "spi".into(),
                    key: "config",
                    requirement: error.to_string(),
                    found: String::new(),
                })?,
            );
        graph.connect(source.samples, Spi::input(spi))?;
        let spi_out = graph.expose("spi", Spi::events(spi))?;
        let nor_out = match self.kind {
            Kind::Spi => None,
            Kind::SpiNor => {
                let nor = graph.add(SpiNorRead24::new());
                graph.connect(Spi::events(spi), SpiNorRead24::input(nor))?;
                Some(graph.expose("nor", SpiNorRead24::operations(nor))?)
            }
        };
        let mut pipeline = graph.build()?;
        let spi = pipeline
            .exposed(spi_out)
            .unwrap_or_else(|| unreachable!("exposed once"));
        let nor = nor_out.map(|nor| {
            pipeline
                .exposed(nor)
                .unwrap_or_else(|| unreachable!("exposed once"))
        });
        Ok(Built {
            pipeline,
            source: source.id,
            spi,
            nor,
        })
    }
}

/// A built pipeline with its readers.
pub struct Built {
    pub pipeline: Pipeline,
    pub source: SourceId,
    pub spi: Exposed<SpiEvent>,
    pub nor: Option<Exposed<NorEvent>>,
}

impl std::fmt::Debug for Built {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Built").finish_non_exhaustive()
    }
}
