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
    /// A decoder refused its configuration. Keep its diagnostic separate from the
    /// node name so embedding applications can localize a decoder's message key.
    #[error("node {node}: {message}")]
    Configuration { node: String, message: String },
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
        let (built, fault) =
            kind.build(&node.config)
                .map_err(|message| BuildError::Configuration {
                    node: name.clone(),
                    message,
                })?;
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
    fn new_transport_paths_build_and_expose_typed_readers() {
        for name in [
            "qspi",
            "sdcard-sd",
            "swim",
            "swim-stm8",
            "aud",
            "c2",
            "avr-pdi",
            "fsi",
            "sd-native",
            "st7735",
            "st7789",
            "sda2506",
            "tlc5620",
        ] {
            let description: Description = serde_json::from_value(serde_json::json!({
                "source": { "channels": 8, "rate": 16000000 },
                "nodes": { "transport": { "type": name } },
                "connections": [{ "from": "source", "to": "transport.logic" }],
                "expose": { "result": "transport.events" }
            }))
            .expect("description");
            let built = build(&description).expect("transport builds");
            assert_eq!(built.streams.len(), 1);
            assert_eq!(built.streams[0].kind, name);
            if [
                "swim-stm8",
                "aud",
                "c2",
                "avr-pdi",
                "fsi",
                "sd-native",
                "st7735",
                "st7789",
                "sda2506",
                "tlc5620",
            ]
            .contains(&name)
            {
                let mut catalog = Catalog::new();
                catalog.load(&module(name)).expect("loads transport IR");
                assert_eq!(
                    build_with(&description, &catalog)
                        .expect("loaded transport builds")
                        .streams[0]
                        .kind,
                    name
                );
            }
        }
        let description = parse(
            r#"{
            "source": { "channels": 4, "rate": 16000000 },
            "nodes": { "tap": { "type": "jtag" }, "debug": { "type": "jtag-dp" } },
            "connections": [
                { "from": "source", "to": "tap.logic" },
                { "from": "tap.events", "to": "debug.bus" }
            ],
            "expose": { "debug": "debug.events" }
        }"#,
        );
        assert_eq!(
            build(&description).expect("JTAG debug builds").streams[0].kind,
            "jtag-dp"
        );
        let mut missing = description;
        missing.nodes.get_mut("tap").expect("tap").config = serde_json::json!({"tdo":null});
        assert!(
            matches!(build(&missing), Err(BuildError::Graph(GraphError::Requirement { key, .. })) if key == "tdo")
        );
    }

    #[test]
    fn jtag_device_profiles_build_as_compiled_or_loaded_layers() {
        for name in ["jtag-ejtag", "jtag-stm32"] {
            let description: Description = serde_json::from_value(serde_json::json!({
                "source": { "channels": 4, "rate": 16000000 },
                "nodes": { "tap": { "type": "jtag" }, "device": { "type": name } },
                "connections": [
                    { "from": "source", "to": "tap.logic" },
                    { "from": "tap.events", "to": "device.bus" }
                ],
                "expose": { "result": "device.events" }
            }))
            .expect("description");
            for loaded in [false, true] {
                let mut catalog = Catalog::new();
                if loaded {
                    catalog.load(&module(name)).expect("loads JTAG device IR");
                }
                assert_eq!(
                    build_with(&description, &catalog).expect("builds").streams[0].kind,
                    name
                );
                let mut missing = description.clone();
                missing.nodes.get_mut("tap").expect("tap").config = serde_json::json!({"tdi":null});
                assert!(matches!(build_with(&missing, &catalog),
                    Err(BuildError::Graph(GraphError::Requirement { key, .. })) if key == "tdi"));
            }
        }
    }

    #[test]
    fn jtag_chain_and_memap_connect_through_four_mixed_layers() {
        let description = parse(
            r#"{
            "source": { "channels": 4, "rate": 16000000 },
            "nodes": {
                "tap": { "type": "jtag" },
                "chain": { "type": "jtag-chain" },
                "debug": { "type": "jtag-dp" },
                "memory": { "type": "arm-memap" }
            },
            "connections": [
                { "from": "source", "to": "tap.logic" },
                { "from": "tap.events", "to": "chain.bus" },
                { "from": "chain.events", "to": "debug.bus" },
                { "from": "debug.events", "to": "memory.bus" }
            ],
            "expose": { "memory": "memory.events" }
        }"#,
        );
        for modules in [
            vec![],
            vec!["jtag-chain"],
            vec!["arm-memap"],
            vec!["jtag-chain", "arm-memap"],
        ] {
            let mut catalog = Catalog::new();
            for name in modules {
                catalog.load(&module(name)).expect("loads IR");
            }
            let built = build_with(&description, &catalog).expect("four-layer graph builds");
            assert_eq!(built.streams[0].kind, "arm-memap");
        }
    }

    #[test]
    fn device_profiles_connect_as_compiled_and_loaded_nodes() {
        for name in [
            "adxl345",
            "ds1307",
            "lm75",
            "mcp230xx",
            "mlx90614",
            "pca9571",
            "rtc8564",
            "tca6408a",
            "edid",
            "ltc26x7",
            "mxc6225xu",
            "nunchuk",
            "atsha204a",
            "xfp",
            "eeprom24xx-memory",
            "smbus",
        ] {
            let description: Description = serde_json::from_value(serde_json::json!({
                "source": { "channels": 2, "rate": 1000000 },
                "nodes": {
                    "bus": { "type": "i2c" },
                    "device": { "type": name }
                },
                "connections": [
                    { "from": "source", "to": "bus.logic" },
                    { "from": "bus.events", "to": "device.bus" }
                ],
                "expose": { "device": "device.events" }
            }))
            .expect("device graph");
            let compiled = build(&description).unwrap_or_else(|error| panic!("{name}: {error}"));
            assert_eq!(compiled.streams[0].kind, name);
            let mut catalog = Catalog::new();
            catalog.load(&module(name)).expect("loads device IR");
            let loaded = build_with(&description, &catalog)
                .unwrap_or_else(|error| panic!("loaded {name}: {error}"));
            assert_eq!(loaded.streams[0].kind, name);
        }
    }

    #[test]
    fn ps2_device_profiles_connect_in_both_execution_modes() {
        for mode in ["keyboard-set2", "mouse"] {
            let description: Description = serde_json::from_value(serde_json::json!({
                "source":{"channels":2,"rate":1000000},
                "nodes":{"bus":{"type":"ps2"},"device":{"type":"ps2-device","config":{"mode":mode}}},
                "connections":[{"from":"source","to":"bus.logic"},{"from":"bus.events","to":"device.bus"}],
                "expose":{"device":"device.events"}
            })).expect("PS/2 graph");
            build(&description).expect("compiled PS/2 device graph");
            let mut catalog = Catalog::new();
            catalog
                .load(&module("ps2-device"))
                .expect("loads PS/2 device IR");
            build_with(&description, &catalog).expect("loaded PS/2 device graph");
        }
    }

    #[test]
    fn modbus_pdu_connects_through_three_layers_in_both_execution_modes() {
        let description = parse(
            r#"{
            "source": { "channels": 1, "rate": 1000000 },
            "nodes": {
                "line": { "type": "uart" },
                "rtu": { "type": "modbus-rtu" },
                "pdu": { "type": "modbus-pdu" }
            },
            "connections": [
                { "from": "source", "to": "line.logic" },
                { "from": "line.events", "to": "rtu.uart" },
                { "from": "rtu.events", "to": "pdu.bus" }
            ],
            "expose": { "pdu": "pdu.events" }
        }"#,
        );
        assert_eq!(
            build(&description).expect("compiled PDU graph").streams[0].kind,
            "modbus-pdu"
        );
        let mut catalog = Catalog::new();
        catalog.load(&module("modbus-pdu")).expect("loads PDU IR");
        assert_eq!(
            build_with(&description, &catalog)
                .expect("loaded PDU graph")
                .streams[0]
                .kind,
            "modbus-pdu"
        );
    }

    #[test]
    fn spi_profiles_require_their_word_width_in_both_execution_modes() {
        for (name, bits) in [
            ("ad5626", 1),
            ("ad79x0", 1),
            ("adf435x", 1),
            ("ltc242x", 1),
            ("x2444m", 1),
            ("a7105", 8),
            ("adns5020", 8),
            ("adxl345-spi", 8),
            ("ade77xx", 8),
            ("cc1101", 8),
            ("enc28j60", 8),
            ("max7219", 8),
            ("max72xx", 8),
            ("mrf24j40", 8),
            ("nrf905", 8),
            ("rfm12", 8),
            ("sd-registers", 8),
            ("spi-tpm", 8),
            ("st25r39xx-spi", 8),
            ("rgb-led-spi", 8),
            ("xn297", 8),
        ] {
            let mut description: Description = serde_json::from_value(serde_json::json!({
                "source":{"channels":4,"rate":10000000},
                "nodes":{"bus":{"type":"spi","config":{"word_bits":bits}}, "device":{"type":name}},
                "connections":[{"from":"source","to":"bus.logic"},{"from":"bus.events","to":"device.bus"}],
                "expose":{"decoded":"device.events"}
            })).expect("SPI graph");
            let mut loaded = Catalog::new();
            loaded.load(&module(name)).expect("loads IR");
            for catalog in [Catalog::new(), loaded] {
                build_with(&description, &catalog)
                    .unwrap_or_else(|error| panic!("{name}: {error}"));
                description.nodes.get_mut("bus").expect("bus").config["word_bits"] =
                    serde_json::json!(7);
                assert!(
                    matches!(build_with(&description, &catalog), Err(BuildError::Graph(GraphError::Requirement { key, .. })) if key == "wordBits")
                );
                description.nodes.get_mut("bus").expect("bus").config["word_bits"] =
                    serde_json::json!(bits);
            }
        }
    }

    #[test]
    fn trace_transport_feeds_itm_and_etm_with_compiled_and_loaded_contracts() {
        for name in ["arm-itm", "arm-etmv3"] {
            let description: Description = serde_json::from_value(serde_json::json!({
                "source":{"channels":1,"rate":100000000},
                "nodes":{"line":{"type":"uart"},"trace":{"type":"arm-trace-transport","config":{"format":"tpiu"}},"packets":{"type":name}},
                "connections":[{"from":"source","to":"line.logic"},{"from":"line.events","to":"trace.bus"},{"from":"trace.events","to":"packets.bus"}],
                "expose":{"packets":"packets.events"}
            })).expect("trace graph");
            for modules in [
                vec![],
                vec!["arm-trace-transport"],
                vec![name],
                vec!["arm-trace-transport", name],
            ] {
                let mut catalog = Catalog::new();
                for name in modules {
                    catalog.load(&module(name)).expect("loads IR");
                }
                build_with(&description, &catalog).expect("trace contracts connect");
            }
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
