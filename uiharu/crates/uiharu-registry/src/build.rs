// SPDX-License-Identifier: Apache-2.0

//! Building a pipeline from a [`Description`], with the node types of a [`Catalog`].

use std::collections::BTreeMap;

use uiharu_core::{
    DynOutput, Graph, GraphConfig, GraphError, NodeId, Pipeline, SourceId, SourceSpec, TimeBase,
};
use uiharu_interp::FaultHandle;

use crate::catalog::{Catalog, KindRef};
use crate::description::Description;
use crate::dynamic::DynReader;
use crate::kinds::KINDS;

/// Why a description does not build.
#[derive(Debug, thiserror::Error)]
pub enum BuildError {
    /// The description itself is wrong: an unknown type, a bad configuration, a reference
    /// that names nothing.
    #[error("{0}")]
    Description(String),
    /// The graph refused the wiring.
    #[error(transparent)]
    Graph(#[from] GraphError),
}

/// An exposed output of a built pipeline.
pub struct Stream {
    /// The name the description exposed it under.
    pub name: String,
    /// The node type whose output it is, by name.
    pub kind: String,
    /// Its reader.
    pub reader: Box<dyn DynReader>,
}

impl std::fmt::Debug for Stream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Stream")
            .field("name", &self.name)
            .field("kind", &self.kind)
            .finish_non_exhaustive()
    }
}

/// A fault of a node that stopped decoding: an interpreted decoder, or a bridge given an
/// event that does not fit the contract.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fault {
    /// The node, as the description names it (`node` for a decoder, `node.port` for the
    /// bridge behind a port).
    pub node: String,
    /// What happened.
    pub message: String,
}

/// A pipeline built from a description.
pub struct Built {
    /// The pipeline, started.
    pub pipeline: Pipeline,
    /// Its source.
    pub source: SourceId,
    /// The source's channels.
    pub channels: u8,
    /// The exposed outputs, in the description's order.
    pub streams: Vec<Stream>,
    /// Where the nodes that can fault report it.
    monitored: Vec<(String, FaultHandle)>,
}

impl std::fmt::Debug for Built {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Built")
            .field("channels", &self.channels)
            .field("streams", &self.streams)
            .finish_non_exhaustive()
    }
}

impl Built {
    /// The faults the nodes have reported so far.
    pub fn faults(&self) -> Vec<Fault> {
        self.monitored
            .iter()
            .filter_map(|(node, handle)| {
                handle.get().map(|message| Fault {
                    node: node.clone(),
                    message,
                })
            })
            .collect()
    }
}

fn wrong(message: String) -> BuildError {
    BuildError::Description(message)
}

fn split_port(reference: &str) -> Option<(&str, &str)> {
    let (node, port) = reference.rsplit_once('.')?;
    (!node.is_empty() && !port.is_empty()).then_some((node, port))
}

/// Builds `description` with the engine's node types: every node from its type and
/// configuration, the connections, the exposed outputs with a reader each.
pub fn build(description: &Description) -> Result<Built, BuildError> {
    build_with(description, &Catalog::new())
}

/// Builds `description` with the node types of `catalog`: every node from its type and
/// configuration, the connections, the exposed outputs with a reader each. A connection
/// between a decoder of the engine and a loaded module goes through a bridge, so that each
/// reads the stream in its own form.
pub fn build_with(description: &Description, catalog: &Catalog) -> Result<Built, BuildError> {
    let channels = description.source.channels;
    if !(1..=64).contains(&channels) {
        return Err(wrong(format!(
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
    let mut monitored: Vec<(String, FaultHandle)> = Vec::new();

    let mut nodes: BTreeMap<&str, (NodeId, KindRef<'_>)> = BTreeMap::new();
    for (name, node) in &description.nodes {
        if name == "source" || name.contains('.') {
            return Err(wrong(format!(
                "node {name}: a node is not named source and has no dot"
            )));
        }
        let kind = catalog
            .get(&node.kind)
            .ok_or_else(|| wrong(format!("node {name}: unknown type {}", node.kind)))?;
        let (built, fault) = kind
            .build(&node.config)
            .map_err(|error| wrong(format!("node {name}: {error}")))?;
        if let Some(fault) = fault {
            monitored.push((name.clone(), fault));
        }
        nodes.insert(name, (graph.add(built), kind));
    }

    let output = |graph: &Graph, reference: &str| -> Result<DynOutput, BuildError> {
        if reference == "source" {
            return Ok(graph.source_output(source.id)?);
        }
        let (node, port) = split_port(reference)
            .ok_or_else(|| wrong(format!("{reference}: expected source or node.port")))?;
        let (id, _) = nodes
            .get(node)
            .ok_or_else(|| wrong(format!("{reference}: no node named {node}")))?;
        Ok(graph.output(*id, port)?)
    };
    for connection in &description.connections {
        let from = output(&graph, &connection.from)?;
        // Whether the output carries the JSON events of a loaded module.
        let from_loaded = split_port(&connection.from)
            .filter(|_| connection.from != "source")
            .and_then(|(node, _)| nodes.get(node))
            .is_some_and(|(_, kind)| kind.is_loaded());
        let (node, port) = split_port(&connection.to)
            .ok_or_else(|| wrong(format!("{}: expected node.port", connection.to)))?;
        let (id, kind) = nodes
            .get(node)
            .ok_or_else(|| wrong(format!("{}: no node named {node}", connection.to)))?;
        let to = graph.input(*id, port)?;
        // Whether the input reads JSON events: a loaded module's, unless it reads samples.
        let to_loaded = kind.is_loaded() && !kind.reads_samples();
        let from = match (from_loaded, to_loaded) {
            (false, true) if connection.from != "source" => {
                // A compiled decoder feeding a loaded module: bridged by the producer's type.
                let producer = split_port(&connection.from)
                    .and_then(|(node, _)| nodes.get(node))
                    .and_then(|(_, kind)| match kind {
                        KindRef::Builtin(kind) => Some(*kind),
                        KindRef::Loaded(_) => None,
                    })
                    .ok_or_else(|| wrong(format!("{}: not a node's output", connection.from)))?;
                let bridge = graph.add(((producer.bridge)().to_dyn)(graph.props_of(from)?));
                graph.connect_dyn(from, graph.input(bridge, "in")?)?;
                graph.output(bridge, "out")?
            }
            (true, false) => {
                // A loaded module feeding a compiled decoder: bridged by the type of the
                // schema it emits, when the engine has one; otherwise the graph says why the
                // ports do not fit.
                let schema = graph.schema_of(from)?;
                match KINDS.iter().find(|kind| kind.schema() == schema) {
                    Some(owner) => {
                        let bridge = graph.add(((owner.bridge)().from_dyn)(graph.props_of(from)?));
                        graph.connect_dyn(from, graph.input(bridge, "in")?)?;
                        graph.output(bridge, "out")?
                    }
                    None => from,
                }
            }
            _ => from,
        };
        graph.connect_dyn(from, to)?;
    }

    let mut exposures = Vec::new();
    for (name, reference) in &description.expose {
        let from = output(&graph, reference)?;
        let kind = match split_port(reference) {
            Some((node, _)) if reference != "source" => nodes.get(node).map(|(_, kind)| *kind),
            _ => None,
        }
        .ok_or_else(|| wrong(format!("expose {name}: {reference} is not a node's output")))?;
        exposures.push((name.clone(), kind, graph.expose_dyn(name.clone(), from)?));
    }

    let mut pipeline = graph.build()?;
    let mut streams = Vec::new();
    for (name, kind, exposed) in exposures {
        let reader = kind
            .open(&mut pipeline, exposed, name.clone())
            .ok_or_else(|| {
                wrong(format!(
                    "expose {name}: the output carries an unexpected type"
                ))
            })?;
        streams.push(Stream {
            name,
            kind: kind.name().to_owned(),
            reader,
        });
    }
    Ok(Built {
        pipeline,
        source: source.id,
        channels,
        streams,
        monitored,
    })
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::Arc;

    use uiharu_core::{SignalItem, Tick};
    use uiharu_protocols::spi::SpiConfig;
    use uiharu_protocols::spi::synth::{SpiWaveform, nor_read_words};

    use super::*;
    use crate::DynItem;

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

    fn module(name: &str) -> uiharu_ir::Module {
        let path =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!("../../dsl/ir/{name}.json"));
        let json = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("{} reads: {error}", path.display()));
        serde_json::from_str(&json).expect("the module parses")
    }

    #[test]
    fn exposed_outputs_keep_the_order_of_the_description() {
        let built = build(&parse(FLASH)).expect("builds");
        let names: Vec<&str> = built
            .streams
            .iter()
            .map(|stream| stream.name.as_str())
            .collect();
        assert_eq!(names, ["nor", "spi"]);
        let description = parse(FLASH);
        let nodes: Vec<&String> = description.nodes.keys().collect();
        assert_eq!(nodes, ["spi", "flash"]);
    }

    /// Runs a NOR read through `built` and returns the NOR items and the SPI event count.
    fn decode_flash(mut built: Built) -> (Vec<DynItem>, usize) {
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
        assert_eq!(built.streams[0].name, "nor");
        assert_eq!(built.streams[0].kind, "spi-nor");
        assert_eq!(built.streams[1].name, "spi");
        let mut nor = Vec::new();
        while let Some(item) = built.streams[0].reader.next() {
            nor.push(item);
        }
        assert_eq!(nor.last(), Some(&DynItem::End(end)));
        assert!(built.streams[0].reader.is_ended());
        assert_eq!(built.streams[0].reader.next(), None);
        let mut spi_events = 0;
        while let Some(item) = built.streams[1].reader.next() {
            if matches!(item, DynItem::Event { .. }) {
                spi_events += 1;
            }
        }
        assert_eq!(spi_events, built.streams[1].reader.events() as usize);
        assert!(built.faults().is_empty());
        (nor, spi_events)
    }

    fn check_flash(nor: &[DynItem], spi_events: usize) {
        let payloads: Vec<&serde_json::Value> = nor
            .iter()
            .filter_map(|item| match item {
                DynItem::Event { payload, .. } => Some(payload),
                _ => None,
            })
            .collect();
        assert_eq!(payloads[0]["kind"], "begin");
        assert_eq!(payloads[0]["command"], "read");
        assert!(
            payloads
                .iter()
                .any(|p| p["kind"] == "address" && p["address"] == 291)
        );
        assert!(
            payloads
                .iter()
                .any(|p| p["kind"] == "data" && p["bytes"] == serde_json::json!([170, 187]))
        );
        assert_eq!(spi_events, 8);
    }

    #[test]
    fn a_described_pipeline_decodes_into_json_events() {
        let (nor, spi_events) = decode_flash(build(&parse(FLASH)).expect("builds"));
        check_flash(&nor, spi_events);
    }

    #[test]
    fn loaded_modules_decode_the_same_bridged_to_the_engine_either_way() {
        let reference = decode_flash(build(&parse(FLASH)).expect("builds"));
        for loaded in [&["spi"][..], &["spi-nor"], &["spi", "spi-nor"]] {
            let mut catalog = Catalog::new();
            for name in loaded {
                catalog.load(&module(name)).expect("loads");
            }
            let built = build_with(&parse(FLASH), &catalog).expect("builds");
            let (nor, spi_events) = decode_flash(built);
            assert_eq!(
                (&nor, spi_events),
                (&reference.0, reference.1),
                "{loaded:?}"
            );
            check_flash(&nor, spi_events);
        }
    }

    #[test]
    fn descriptions_are_checked() {
        let error = |text: &str| build(&parse(text)).expect_err("refused").to_string();
        assert_eq!(
            error(r#"{"source": {"channels": 4}, "nodes": {"x": {"type": "no-such-protocol"}}}"#),
            "node x: unknown type no-such-protocol"
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

    #[test]
    fn a_loaded_module_is_checked_like_the_engine_s_decoders() {
        let mut catalog = Catalog::new();
        catalog.load(&module("spi")).expect("loads");
        let error = |text: &str| {
            build_with(&parse(text), &catalog)
                .expect_err("refused")
                .to_string()
        };
        assert_eq!(
            error(
                r#"{"source": {"channels": 4}, "nodes": {"spi": {"type": "spi", "config": {"clock": 1}}}}"#
            ),
            "node spi: unknown field `clock`, expected one of `clk`, `cs`, `mosi`, `miso`, `mode`, `word_bits`, `bit_order`, `cs_active_low`"
        );
        assert_eq!(
            error(
                r#"{"source": {"channels": 2}, "nodes": {"spi": {"type": "spi"}}, "connections": [{"from": "source", "to": "spi.logic"}]}"#
            ),
            "spi#0.logic requires channels >= 4 of source#0.samples, which declares 2"
        );
        assert_eq!(
            error(
                r#"{"source": {"channels": 4}, "nodes": {"spi": {"type": "spi", "config": {"word_bits": 4}}, "flash": {"type": "spi-nor"}}, "connections": [{"from": "source", "to": "spi.logic"}, {"from": "spi.events", "to": "flash.bus"}]}"#
            ),
            "spi-nor#1.bus requires wordBits = 8 of from-dyn#2.out, which declares 4"
        );
    }

    #[test]
    fn descriptions_round_trip_through_json() {
        let description = parse(FLASH);
        let text = serde_json::to_string(&description).expect("serializes");
        assert_eq!(parse(&text), description);
    }
}
