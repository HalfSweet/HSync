// SPDX-License-Identifier: Apache-2.0

//! Building a pipeline from a [`Description`].

use std::collections::BTreeMap;

use uiharu_core::{
    DynOutput, Graph, GraphConfig, GraphError, NodeId, Pipeline, SourceId, SourceSpec, TimeBase,
};

use crate::description::Description;
use crate::dynamic::DynReader;
use crate::kinds::{KINDS, Kind};

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
    /// The node type whose output it is.
    pub kind: &'static Kind,
    /// Its reader.
    pub reader: Box<dyn DynReader>,
}

impl std::fmt::Debug for Stream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Stream")
            .field("name", &self.name)
            .field("kind", &self.kind.name)
            .finish_non_exhaustive()
    }
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
}

impl std::fmt::Debug for Built {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Built")
            .field("channels", &self.channels)
            .field("streams", &self.streams)
            .finish_non_exhaustive()
    }
}

fn wrong(message: String) -> BuildError {
    BuildError::Description(message)
}

fn split_port(reference: &str) -> Option<(&str, &str)> {
    let (node, port) = reference.rsplit_once('.')?;
    (!node.is_empty() && !port.is_empty()).then_some((node, port))
}

/// Builds `description`: every node from its type and configuration, the connections, the
/// exposed outputs with a reader each.
pub fn build(description: &Description) -> Result<Built, BuildError> {
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

    let mut nodes: BTreeMap<&str, (NodeId, &'static Kind)> = BTreeMap::new();
    for (name, node) in &description.nodes {
        if name == "source" || name.contains('.') {
            return Err(wrong(format!(
                "node {name}: a node is not named source and has no dot"
            )));
        }
        let kind = KINDS
            .iter()
            .find(|kind| kind.name == node.kind)
            .ok_or_else(|| wrong(format!("node {name}: unknown type {}", node.kind)))?;
        let built =
            (kind.build)(&node.config).map_err(|error| wrong(format!("node {name}: {error}")))?;
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
        let (node, port) = split_port(&connection.to)
            .ok_or_else(|| wrong(format!("{}: expected node.port", connection.to)))?;
        let (id, _) = nodes
            .get(node)
            .ok_or_else(|| wrong(format!("{}: no node named {node}", connection.to)))?;
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
        .ok_or_else(|| wrong(format!("expose {name}: {reference} is not a node's output")))?;
        exposures.push((name.clone(), kind, graph.expose_dyn(name.clone(), from)?));
    }

    let mut pipeline = graph.build()?;
    let mut streams = Vec::new();
    for (name, kind, exposed) in exposures {
        let reader = (kind.open)(&mut pipeline, exposed, name.clone()).ok_or_else(|| {
            wrong(format!(
                "expose {name}: the output carries an unexpected type"
            ))
        })?;
        streams.push(Stream { name, kind, reader });
    }
    Ok(Built {
        pipeline,
        source: source.id,
        channels,
        streams,
    })
}

#[cfg(test)]
mod tests {
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

    #[test]
    fn a_described_pipeline_decodes_into_json_events() {
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
        assert_eq!(built.streams[0].name, "nor");
        assert_eq!(built.streams[0].kind.name, "spi-nor");
        assert_eq!(built.streams[1].name, "spi");
        let mut nor = Vec::new();
        while let Some(item) = built.streams[0].reader.next() {
            nor.push(item);
        }
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
        assert_eq!(nor.last(), Some(&DynItem::End(end)));
        assert!(built.streams[0].reader.is_ended());
        assert_eq!(built.streams[0].reader.next(), None);
        let mut spi_events = 0;
        while let Some(item) = built.streams[1].reader.next() {
            if matches!(item, DynItem::Event { .. }) {
                spi_events += 1;
            }
        }
        assert_eq!(spi_events, 8);
        assert_eq!(built.streams[1].reader.events(), 8);
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
    fn descriptions_round_trip_through_json() {
        let description = parse(FLASH);
        let text = serde_json::to_string(&description).expect("serializes");
        assert_eq!(parse(&text), description);
    }
}
