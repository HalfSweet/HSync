// SPDX-License-Identifier: Apache-2.0

//! Decode pipelines described in a file — the hand-written form of the graph description the
//! DSL compiler will produce:
//!
//! ```json
//! {
//!   "source": { "channels": 4, "rate": 25000000 },
//!   "nodes": {
//!     "spi": { "type": "spi", "config": { "clk": 0, "cs": 1, "mosi": 2, "miso": 3, "mode": 0 } },
//!     "flash": { "type": "spi-nor" }
//!   },
//!   "connections": [
//!     { "from": "source", "to": "spi.logic" },
//!     { "from": "spi.events", "to": "flash.bus" }
//!   ],
//!   "expose": { "spi": "spi.events", "nor": "flash.events" }
//! }
//! ```
//!
//! Ports are named as the nodes declare them; `source` is the source's samples. The node types
//! the command line knows are listed in [`KINDS`]: a decoder becomes available here by adding
//! its entry — how to build it from its configuration and how to write its output.

use std::collections::BTreeMap;

use serde::Deserialize;
use serde::Serialize;
use serde::de::DeserializeOwned;
use uiharu_core::{
    DynExposedRef, DynOutput, Graph, GraphConfig, Node, NodeId, Pipeline, SourceId, SourceSpec,
    TimeBase,
};
use uiharu_protocols::can::{Can, CanEvent};
use uiharu_protocols::eeprom24xx::{Eeprom24xx, EepromEvent};
use uiharu_protocols::i2c::{I2c, I2cEvent};
use uiharu_protocols::i2s::{I2s, I2sEvent};
use uiharu_protocols::jtag::{Jtag, JtagEvent};
use uiharu_protocols::modbus_rtu::{ModbusEvent, ModbusRtu};
use uiharu_protocols::parallel::{Parallel, ParallelEvent};
use uiharu_protocols::pwm::{Pwm, PwmEvent};
use uiharu_protocols::sdcard_spi::{SdCardSpi, SdEvent};
use uiharu_protocols::spi::{Spi, SpiEvent};
use uiharu_protocols::spi_nor::{NorEvent, SpiNor, SpiNorConfig};
use uiharu_protocols::swd::{Swd, SwdEvent};
use uiharu_protocols::timing::{Timing, TimingEvent};
use uiharu_protocols::uart::{Uart, UartEvent};

use crate::Failure;
use crate::ndjson::{StreamWriter, Writer};

/// A pipeline as a file describes it.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Description {
    /// The source of samples.
    pub source: SourceDescription,
    /// The nodes, by name.
    pub nodes: BTreeMap<String, NodeDescription>,
    /// What reads what: `from` is `source` or `node.port`, `to` is `node.port`.
    #[serde(default)]
    pub connections: Vec<Connection>,
    /// The outputs to write, by the name they are written under: `node.port`.
    #[serde(default)]
    pub expose: BTreeMap<String, String>,
    /// Items a queue between nodes holds (default 256).
    #[serde(default)]
    pub queue: Option<usize>,
    /// Operations a node runs per turn (default 1024).
    #[serde(default)]
    pub budget: Option<u32>,
}

/// The source of a described pipeline.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SourceDescription {
    /// Channels in the capture, 1 to 64.
    pub channels: u8,
    /// Samples per second (default 1 000 000).
    #[serde(default = "default_rate")]
    pub rate: u64,
    /// Blocks the source holds before the reader must run the pipeline (default 4).
    #[serde(default)]
    pub blocks: Option<usize>,
}

fn default_rate() -> u64 {
    1_000_000
}

/// A node of a described pipeline.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct NodeDescription {
    /// The node type, one of [`KINDS`].
    #[serde(rename = "type")]
    pub kind: String,
    /// The node's configuration, as its type reads it; absent fields take their defaults.
    #[serde(default)]
    pub config: serde_json::Value,
}

/// One connection of a described pipeline.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Connection {
    /// `source`, or `node.port` of an output.
    pub from: String,
    /// `node.port` of an input.
    pub to: String,
}

/// A node type the command line can build.
pub struct Kind {
    /// The name a description uses.
    pub name: &'static str,
    /// Builds the node from its configuration.
    pub build: fn(&serde_json::Value) -> Result<Box<dyn Node>, String>,
    /// Makes the writer of the node's output, or `None` when the output carries another type.
    pub writer: fn(&mut Pipeline, DynExposedRef, String) -> Option<Box<dyn Writer>>,
}

impl std::fmt::Debug for Kind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Kind")
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}

/// Reads a configuration; nothing stands for an empty object.
fn config<T: DeserializeOwned>(value: &serde_json::Value) -> Result<T, String> {
    let value = match value {
        serde_json::Value::Null => serde_json::Value::Object(serde_json::Map::new()),
        other => other.clone(),
    };
    serde_json::from_value(value).map_err(|error| error.to_string())
}

/// Builds a node from its configuration with `make`, which validates it.
fn build_node<C, N, E>(
    value: &serde_json::Value,
    make: fn(C) -> Result<N, E>,
) -> Result<Box<dyn Node>, String>
where
    C: DeserializeOwned,
    N: Node,
    E: std::fmt::Display,
{
    let config: C = config(value)?;
    make(config)
        .map(|node| Box::new(node) as Box<dyn Node>)
        .map_err(|error| error.to_string())
}

/// Makes the writer of an output carrying `T` events.
fn write_as<T: Serialize + 'static>(
    pipeline: &mut Pipeline,
    exposed: DynExposedRef,
    name: String,
) -> Option<Box<dyn Writer>> {
    Some(Box::new(StreamWriter::new(
        name,
        pipeline.exposed_dyn::<T>(exposed)?,
    )))
}

fn build_spi(value: &serde_json::Value) -> Result<Box<dyn Node>, String> {
    build_node(value, Spi::new)
}

/// Checks that a node without configuration was given none.
fn no_config(value: &serde_json::Value) -> Result<(), String> {
    if !matches!(value, serde_json::Value::Null) && value != &serde_json::json!({}) {
        return Err("takes no configuration".to_owned());
    }
    Ok(())
}

fn build_spi_nor(value: &serde_json::Value) -> Result<Box<dyn Node>, String> {
    let config: SpiNorConfig = config(value)?;
    Ok(Box::new(SpiNor::new(config)))
}

fn build_sdcard_spi(value: &serde_json::Value) -> Result<Box<dyn Node>, String> {
    build_node(value, SdCardSpi::new)
}

fn build_i2s(value: &serde_json::Value) -> Result<Box<dyn Node>, String> {
    build_node(value, I2s::new)
}

fn build_pwm(value: &serde_json::Value) -> Result<Box<dyn Node>, String> {
    build_node(value, Pwm::new)
}

fn build_timing(value: &serde_json::Value) -> Result<Box<dyn Node>, String> {
    build_node(value, Timing::new)
}

fn build_parallel(value: &serde_json::Value) -> Result<Box<dyn Node>, String> {
    build_node(value, Parallel::new)
}

fn build_can(value: &serde_json::Value) -> Result<Box<dyn Node>, String> {
    build_node(value, Can::new)
}

fn build_i2c(value: &serde_json::Value) -> Result<Box<dyn Node>, String> {
    build_node(value, I2c::new)
}

fn build_eeprom24xx(value: &serde_json::Value) -> Result<Box<dyn Node>, String> {
    build_node(value, Eeprom24xx::new)
}

fn build_jtag(value: &serde_json::Value) -> Result<Box<dyn Node>, String> {
    build_node(value, Jtag::new)
}

fn build_swd(value: &serde_json::Value) -> Result<Box<dyn Node>, String> {
    build_node(value, Swd::new)
}

fn build_uart(value: &serde_json::Value) -> Result<Box<dyn Node>, String> {
    build_node(value, Uart::new)
}

fn build_modbus_rtu(value: &serde_json::Value) -> Result<Box<dyn Node>, String> {
    no_config(value)?;
    Ok(Box::new(ModbusRtu::new()))
}

/// The node types a description may use, each with the configuration type it reads.
pub const KINDS: &[Kind] = &[
    Kind {
        name: "spi",
        build: build_spi,
        writer: write_as::<SpiEvent>,
    },
    Kind {
        name: "spi-nor",
        build: build_spi_nor,
        writer: write_as::<NorEvent>,
    },
    Kind {
        name: "i2s",
        build: build_i2s,
        writer: write_as::<I2sEvent>,
    },
    Kind {
        name: "pwm",
        build: build_pwm,
        writer: write_as::<PwmEvent>,
    },
    Kind {
        name: "timing",
        build: build_timing,
        writer: write_as::<TimingEvent>,
    },
    Kind {
        name: "parallel",
        build: build_parallel,
        writer: write_as::<ParallelEvent>,
    },
    Kind {
        name: "uart",
        build: build_uart,
        writer: write_as::<UartEvent>,
    },
    Kind {
        name: "modbus-rtu",
        build: build_modbus_rtu,
        writer: write_as::<ModbusEvent>,
    },
    Kind {
        name: "i2c",
        build: build_i2c,
        writer: write_as::<I2cEvent>,
    },
    Kind {
        name: "eeprom24xx",
        build: build_eeprom24xx,
        writer: write_as::<EepromEvent>,
    },
    Kind {
        name: "jtag",
        build: build_jtag,
        writer: write_as::<JtagEvent>,
    },
    Kind {
        name: "swd",
        build: build_swd,
        writer: write_as::<SwdEvent>,
    },
    Kind {
        name: "sdcard-spi",
        build: build_sdcard_spi,
        writer: write_as::<SdEvent>,
    },
    Kind {
        name: "can",
        build: build_can,
        writer: write_as::<CanEvent>,
    },
];

/// A pipeline built from a description, with a writer for every exposed output.
pub struct Built {
    /// The pipeline.
    pub pipeline: Pipeline,
    /// Its source.
    pub source: SourceId,
    /// The source's channels.
    pub channels: u8,
    /// The exposed outputs, in the order the description names them.
    pub writers: Vec<Box<dyn Writer>>,
}

impl std::fmt::Debug for Built {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Built")
            .field("channels", &self.channels)
            .finish_non_exhaustive()
    }
}

fn usage(message: String) -> Failure {
    Failure::Usage(message)
}

/// Splits `node.port`.
fn split_port(reference: &str) -> Option<(&str, &str)> {
    let (node, port) = reference.rsplit_once('.')?;
    (!node.is_empty() && !port.is_empty()).then_some((node, port))
}

/// Builds the pipeline a description asks for.
pub fn build(description: &Description) -> Result<Built, Failure> {
    let channels = description.source.channels;
    if !(1..=64).contains(&channels) {
        return Err(usage(format!(
            "source: {channels} channels; expected 1 to 64"
        )));
    }
    let mut graph = Graph::with_config(GraphConfig {
        queue_capacity: description.queue.unwrap_or(256),
        budget: description.budget.unwrap_or(1024),
    });
    let mut spec = SourceSpec::new("source", channels, TimeBase::new(description.source.rate));
    if let Some(blocks) = description.source.blocks {
        spec = spec.with_capacity(blocks);
    }
    let source = graph.source(spec)?;

    let mut nodes: BTreeMap<&str, (NodeId, &'static Kind)> = BTreeMap::new();
    for (name, node) in &description.nodes {
        if name == "source" || name.contains('.') {
            return Err(usage(format!(
                "node {name}: a node is not named source and has no dot"
            )));
        }
        let kind = KINDS
            .iter()
            .find(|kind| kind.name == node.kind)
            .ok_or_else(|| usage(format!("node {name}: unknown type {}", node.kind)))?;
        let built =
            (kind.build)(&node.config).map_err(|error| usage(format!("node {name}: {error}")))?;
        nodes.insert(name, (graph.add(built), kind));
    }

    let output = |graph: &Graph, reference: &str| -> Result<DynOutput, Failure> {
        if reference == "source" {
            return Ok(graph.source_output(source.id)?);
        }
        let (node, port) = split_port(reference)
            .ok_or_else(|| usage(format!("{reference}: expected source or node.port")))?;
        let (id, _) = nodes
            .get(node)
            .ok_or_else(|| usage(format!("{reference}: no node named {node}")))?;
        Ok(graph.output(*id, port)?)
    };
    for connection in &description.connections {
        let from = output(&graph, &connection.from)?;
        let (node, port) = split_port(&connection.to)
            .ok_or_else(|| usage(format!("{}: expected node.port", connection.to)))?;
        let (id, _) = nodes
            .get(node)
            .ok_or_else(|| usage(format!("{}: no node named {node}", connection.to)))?;
        let to = graph.input(*id, port)?;
        graph.connect_dyn(from, to)?;
    }

    let mut exposures = Vec::new();
    for (name, reference) in &description.expose {
        let from = output(&graph, reference)?;
        let kind = match split_port(reference) {
            Some((node, _)) if reference != "source" => nodes.get(node).map(|(_, kind)| *kind),
            _ => None,
        }
        .ok_or_else(|| usage(format!("expose {name}: {reference} is not a node's output")))?;
        exposures.push((name.clone(), kind, graph.expose_dyn(name.clone(), from)?));
    }

    let mut pipeline = graph.build()?;
    let mut writers = Vec::new();
    for (name, kind, exposed) in exposures {
        let writer = (kind.writer)(&mut pipeline, exposed, name.clone()).ok_or_else(|| {
            usage(format!(
                "expose {name}: the output carries an unexpected type"
            ))
        })?;
        writers.push(writer);
    }
    Ok(Built {
        pipeline,
        source: source.id,
        channels,
        writers,
    })
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use uiharu_core::{SignalItem, Tick};
    use uiharu_protocols::spi::SpiConfig;
    use uiharu_protocols::spi::synth::{SpiWaveform, nor_read_words};

    use super::*;

    const FLASH: &str = r#"{
        "source": { "channels": 4, "rate": 1000000 },
        "nodes": {
            "spi": { "type": "spi", "config": { "mode": 0 } },
            "flash": { "type": "spi-nor" }
        },
        "connections": [
            { "from": "source", "to": "spi.logic" },
            { "from": "spi.events", "to": "flash.bus" }
        ],
        "expose": { "nor": "flash.events", "spi": "spi.events" }
    }"#;

    fn parse(text: &str) -> Description {
        serde_json::from_str(text).expect("a description")
    }

    #[test]
    fn a_described_pipeline_decodes_and_writes() {
        let mut built = build(&parse(FLASH)).expect("builds");
        let mut waveform = SpiWaveform::new(SpiConfig::default(), 4, 3, Tick::ZERO);
        waveform.idle(10);
        waveform.transaction(nor_read_words(0x000123, &[0xaa, 0xbb]));
        waveform.idle(10);
        let block = waveform.finish();
        let end = block.end();
        built
            .pipeline
            .push(built.source, SignalItem::Data(Arc::new(block)))
            .expect("pushes");
        built
            .pipeline
            .push(built.source, SignalItem::End(end))
            .expect("pushes");
        built.pipeline.run();
        let mut out = Vec::new();
        for writer in &mut built.writers {
            writer.drain(&mut out, false).expect("writes");
        }
        let text = String::from_utf8(out).expect("utf-8");
        let lines: Vec<&str> = text.lines().collect();
        // The description names "nor" first: its events, its end, then the SPI stream.
        assert!(
            lines[0].starts_with(r#"{"stream":"nor","seq":0,"#),
            "{}",
            lines[0]
        );
        assert!(lines[0].contains(r#""event":{"kind":"begin","op":1,"tx":1,"command":"read"}"#));
        assert!(
            lines
                .iter()
                .any(|line| line.contains(r#""kind":"address","op":1,"address":291,"bytes":3"#))
        );
        assert!(lines.iter().any(|line| {
            line.contains(r#""kind":"data","op":1,"direction":"out","offset":0,"bytes":[170,187]"#)
        }));
        assert!(lines.contains(&format!(r#"{{"stream":"nor","end":{end}}}"#).as_str()));
        assert_eq!(
            lines
                .iter()
                .filter(|line| line.contains(r#""stream":"spi","seq""#))
                .count(),
            8
        );
        assert!(built.writers.iter().all(|writer| writer.is_ended()));
        assert_eq!(built.writers[1].event_count(), 8);
    }

    #[test]
    fn descriptions_are_checked() {
        let error = |text: &str| build(&parse(text)).expect_err("refused").to_string();
        assert_eq!(
            error(r#"{"source": {"channels": 4}, "nodes": {"x": {"type": "lin"}}}"#),
            "node x: unknown type lin"
        );
        assert_eq!(
            error(
                r#"{"source": {"channels": 4}, "nodes": {"spi": {"type": "spi", "config": {"clock": 1}}}}"#
            ),
            "node spi: unknown field `clock`, expected one of `clk`, `cs`, `mosi`, `miso`, `mode`, `word_bits`, `bit_order`, `cs_active_low`"
        );
        assert_eq!(
            error(
                r#"{"source": {"channels": 4}, "nodes": {"spi": {"type": "spi"}}, "connections": [{"from": "source", "to": "spi.clock"}]}"#
            ),
            "spi#0 has no input port named clock"
        );
        assert_eq!(
            error(
                r#"{"source": {"channels": 4}, "nodes": {"spi": {"type": "spi"}}, "connections": [{"from": "source", "to": "spi"}]}"#
            ),
            "spi: expected node.port"
        );
        assert_eq!(
            error(
                r#"{"source": {"channels": 4}, "nodes": {"spi": {"type": "spi"}}, "connections": [{"from": "source", "to": "spi.logic"}], "expose": {"x": "source"}}"#
            ),
            "expose x: source is not a node's output"
        );
        assert_eq!(
            error(
                r#"{"source": {"channels": 2}, "nodes": {"spi": {"type": "spi"}}, "connections": [{"from": "source", "to": "spi.logic"}]}"#
            ),
            "spi#0.logic requires channels >= 4 of source#0.samples, which declares 2"
        );
        assert_eq!(
            error(r#"{"source": {"channels": 4}, "nodes": {"spi": {"type": "spi"}}}"#),
            "spi#0.logic is not connected"
        );
        assert!(
            serde_json::from_str::<Description>(
                r#"{"source": {"channels": 4}, "nodes": {}, "exposed": {}}"#
            )
            .is_err()
        );
    }
}
