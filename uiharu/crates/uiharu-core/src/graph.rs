// SPDX-License-Identifier: Apache-2.0

//! Building a decode graph: sources of logic samples, decoder nodes, the connections between
//! their typed ports and the outputs exposed to the caller. Connecting checks that the ports
//! agree on the schema and that the input's requirements are met; building checks that every
//! input is connected, that the graph has no cycle and that each node reads one time domain.

use std::collections::{BTreeSet, VecDeque};
use std::fmt;
use std::marker::PhantomData;
use std::sync::Arc;

use crate::exec::Pipeline;
use crate::node::{Node, NodeSpec};
use crate::schema::{EventSchema, InputSpec, OutputSpec, Props, Requirement, SchemaId};
use crate::signal::{LogicBlock, MAX_CHANNELS};
use crate::time::TimeBase;

/// A source of logic samples in a graph.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SourceId(pub(crate) usize);

impl SourceId {
    /// The source added `index`-th to its graph.
    pub const fn from_index(index: usize) -> Self {
        Self(index)
    }

    /// The source's position among the graph's sources.
    pub const fn index(self) -> usize {
        self.0
    }
}

/// A node in a graph.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct NodeId(pub(crate) usize);

impl NodeId {
    /// The node added `index`-th to its graph.
    pub const fn from_index(index: usize) -> Self {
        Self(index)
    }

    /// The node's position among the graph's nodes.
    pub const fn index(self) -> usize {
        self.0
    }
}

/// Who owns an output port.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Owner {
    Source(usize),
    Node(usize),
}

/// An output port of a source or node.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct PortKey {
    pub(crate) owner: Owner,
    pub(crate) index: usize,
}

/// An output port carrying `T` events, as [`Graph::connect`] and [`Graph::expose`] take it.
pub struct OutputRef<T> {
    key: PortKey,
    _payload: PhantomData<fn() -> T>,
}

impl<T> OutputRef<T> {
    /// Output port `index` of `node`. The node's own helpers build these, as only the node
    /// knows the payload types of its ports.
    pub fn of_node(node: NodeId, index: usize) -> Self {
        Self {
            key: PortKey {
                owner: Owner::Node(node.0),
                index,
            },
            _payload: PhantomData,
        }
    }
}

impl<T> Clone for OutputRef<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for OutputRef<T> {}

impl<T> fmt::Debug for OutputRef<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OutputRef").field("key", &self.key).finish()
    }
}

/// An input port reading `T` events.
pub struct InputRef<T> {
    node: usize,
    index: usize,
    _payload: PhantomData<fn() -> T>,
}

impl<T> InputRef<T> {
    /// Input port `index` of `node`; see [`OutputRef::of_node`].
    pub fn of_node(node: NodeId, index: usize) -> Self {
        Self {
            node: node.0,
            index,
            _payload: PhantomData,
        }
    }
}

impl<T> Clone for InputRef<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for InputRef<T> {}

impl<T> fmt::Debug for InputRef<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("InputRef")
            .field("node", &self.node)
            .field("index", &self.index)
            .finish()
    }
}

/// An exposed output, read through [`Pipeline::exposed`].
pub struct ExposedRef<T> {
    pub(crate) index: usize,
    _payload: PhantomData<fn() -> T>,
}

impl<T> Clone for ExposedRef<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for ExposedRef<T> {}

impl<T> fmt::Debug for ExposedRef<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ExposedRef")
            .field("index", &self.index)
            .finish()
    }
}

/// An output port named at run time, for graphs built from a description rather than from
/// typed code; see [`Graph::output`] and [`Graph::connect_dyn`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct DynOutput {
    key: PortKey,
}

/// An input port named at run time; see [`Graph::input`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct DynInput {
    node: usize,
    index: usize,
}

/// An output exposed through [`Graph::expose_dyn`], read with [`Pipeline::exposed_dyn`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct DynExposedRef {
    pub(crate) index: usize,
}

/// A source of logic samples, as [`Graph::source`] takes it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceSpec {
    /// The source's name, for diagnostics and statistics.
    pub name: String,
    /// Its channels, at most [`MAX_CHANNELS`].
    pub channels: u8,
    /// Its clock.
    pub time_base: TimeBase,
    /// The blocks the source holds for its readers before [`Pipeline::push`] reports it full.
    pub capacity: usize,
}

impl SourceSpec {
    /// A source named `name` with `channels` channels on `time_base`, holding four blocks.
    pub fn new(name: impl Into<String>, channels: u8, time_base: TimeBase) -> Self {
        Self {
            name: name.into(),
            channels,
            time_base,
            capacity: 4,
        }
    }

    /// Sets how many blocks the source holds.
    pub fn with_capacity(mut self, capacity: usize) -> Self {
        self.capacity = capacity.max(1);
        self
    }

    pub(crate) fn output(&self) -> OutputSpec {
        OutputSpec::new::<Arc<LogicBlock>>(
            "samples",
            Props::new()
                .with("channels", i64::from(self.channels))
                .with(
                    "rateHz",
                    i64::try_from(self.time_base.rate_hz).unwrap_or(i64::MAX),
                ),
        )
    }
}

/// The source as added to a graph: its id and its output.
#[derive(Clone, Copy, Debug)]
pub struct SourceHandle {
    /// The source.
    pub id: SourceId,
    /// Its blocks of samples.
    pub samples: OutputRef<Arc<LogicBlock>>,
}

/// Limits every node and queue of a graph shares.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GraphConfig {
    /// Items a queue between nodes holds before the writer waits.
    pub queue_capacity: usize,
    /// Operations a node completes in one poll before it yields to the others.
    pub budget: u32,
}

impl Default for GraphConfig {
    fn default() -> Self {
        Self {
            queue_capacity: 256,
            budget: 1024,
        }
    }
}

/// Why a graph cannot be built as asked.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum GraphError {
    /// A port reference names a node the graph does not have.
    #[error("no node {0} in the graph")]
    UnknownNode(usize),
    /// A port reference names a port its owner does not declare.
    #[error("{owner} declares no {direction} port {index}")]
    UnknownPort {
        /// The source or node.
        owner: String,
        /// `input` or `output`.
        direction: &'static str,
        /// The port index.
        index: usize,
    },
    /// A source has more channels than a block can hold.
    #[error("source {name} has {channels} channels; at most {MAX_CHANNELS} are supported")]
    TooManyChannels {
        /// The source.
        name: String,
        /// Its channels.
        channels: u8,
    },
    /// The output and the input do not carry the same schema.
    #[error("{from} carries {output} but {to} reads {input}")]
    SchemaMismatch {
        /// The output port.
        from: String,
        /// The input port.
        to: String,
        /// The output's schema.
        output: SchemaId,
        /// The input's schema.
        input: SchemaId,
    },
    /// The output does not satisfy a requirement of the input.
    #[error("{to} requires {key} {requirement} of {from}, which declares {found}")]
    Requirement {
        /// The output port.
        from: String,
        /// The input port.
        to: String,
        /// The property.
        key: &'static str,
        /// What the input requires.
        requirement: String,
        /// What the output declares, or `nothing`.
        found: String,
    },
    /// The input is already connected; an input reads one output.
    #[error("{to} is already connected")]
    AlreadyConnected {
        /// The input port.
        to: String,
    },
    /// The input is not connected.
    #[error("{to} is not connected")]
    Unconnected {
        /// The input port.
        to: String,
    },
    /// The graph has a cycle through this node.
    #[error("{node} is part of a cycle")]
    Cycle {
        /// The node.
        node: String,
    },
    /// The node reads inputs from different sources.
    #[error("{node} reads from more than one time domain ({first} and {second})")]
    TimeDomain {
        /// The node.
        node: String,
        /// A source it reads.
        first: String,
        /// Another source it reads.
        second: String,
    },
    /// Two exposed outputs share a name.
    #[error("an output named {0} is already exposed")]
    DuplicateExposure(String),
    /// A port named at run time does not exist.
    #[error("{owner} has no {direction} port named {name}")]
    UnknownPortName {
        /// The source or node.
        owner: String,
        /// `input` or `output`.
        direction: &'static str,
        /// The name asked for.
        name: String,
    },
}

struct NodeEntry {
    node: Box<dyn Node>,
    spec: NodeSpec,
    /// What each input reads, once connected.
    inputs: Vec<Option<PortKey>>,
}

pub(crate) struct Exposure {
    pub(crate) name: String,
    pub(crate) from: PortKey,
}

/// A decode graph under construction.
pub struct Graph {
    config: GraphConfig,
    sources: Vec<SourceSpec>,
    nodes: Vec<NodeEntry>,
    exposed: Vec<Exposure>,
}

impl fmt::Debug for Graph {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Graph")
            .field("config", &self.config)
            .field("sources", &self.sources.len())
            .field("nodes", &self.nodes.len())
            .field("exposed", &self.exposed.len())
            .finish()
    }
}

impl Default for Graph {
    fn default() -> Self {
        Self::new()
    }
}

impl Graph {
    /// An empty graph with the default limits.
    pub fn new() -> Self {
        Self::with_config(GraphConfig::default())
    }

    /// An empty graph with `config`'s limits.
    pub fn with_config(config: GraphConfig) -> Self {
        Self {
            config,
            sources: Vec::new(),
            nodes: Vec::new(),
            exposed: Vec::new(),
        }
    }

    /// The graph's limits.
    pub fn config(&self) -> GraphConfig {
        self.config
    }

    /// Adds a source of logic samples.
    pub fn source(&mut self, spec: SourceSpec) -> Result<SourceHandle, GraphError> {
        if spec.channels > MAX_CHANNELS {
            return Err(GraphError::TooManyChannels {
                name: spec.name,
                channels: spec.channels,
            });
        }
        let id = SourceId(self.sources.len());
        self.sources.push(spec);
        Ok(SourceHandle {
            id,
            samples: OutputRef {
                key: PortKey {
                    owner: Owner::Source(id.0),
                    index: 0,
                },
                _payload: PhantomData,
            },
        })
    }

    /// Adds a node.
    pub fn add(&mut self, node: impl Node) -> NodeId {
        let spec = node.spec();
        let inputs = vec![None; spec.inputs.len()];
        self.nodes.push(NodeEntry {
            node: Box::new(node),
            spec,
            inputs,
        });
        NodeId(self.nodes.len() - 1)
    }

    fn owner_name(&self, owner: Owner) -> String {
        match owner {
            Owner::Source(index) => {
                format!(
                    "{}#{index}",
                    self.sources.get(index).map_or("?", |s| s.name.as_str())
                )
            }
            Owner::Node(index) => {
                format!(
                    "{}#{index}",
                    self.nodes.get(index).map_or("?", |n| n.spec.name.as_str())
                )
            }
        }
    }

    fn output_spec(&self, key: PortKey) -> Result<OutputSpec, GraphError> {
        let unknown = || GraphError::UnknownPort {
            owner: self.owner_name(key.owner),
            direction: "output",
            index: key.index,
        };
        match key.owner {
            Owner::Source(index) => {
                let source = self.sources.get(index).ok_or_else(unknown)?;
                if key.index != 0 {
                    return Err(unknown());
                }
                Ok(source.output())
            }
            Owner::Node(index) => {
                let node = self
                    .nodes
                    .get(index)
                    .ok_or(GraphError::UnknownNode(index))?;
                node.spec
                    .outputs
                    .get(key.index)
                    .cloned()
                    .ok_or_else(unknown)
            }
        }
    }

    fn input_spec(&self, node: usize, index: usize) -> Result<&InputSpec, GraphError> {
        let entry = self.nodes.get(node).ok_or(GraphError::UnknownNode(node))?;
        entry
            .spec
            .inputs
            .get(index)
            .ok_or_else(|| GraphError::UnknownPort {
                owner: self.owner_name(Owner::Node(node)),
                direction: "input",
                index,
            })
    }

    fn port_name(&self, owner: Owner, name: &str) -> String {
        format!("{}.{name}", self.owner_name(owner))
    }

    /// Connects an output to an input, checking that they carry the same schema and that the
    /// output satisfies what the input requires.
    pub fn connect<T: EventSchema>(
        &mut self,
        from: OutputRef<T>,
        to: InputRef<T>,
    ) -> Result<(), GraphError> {
        self.connect_keys(from.key, to.node, to.index)
    }

    /// Exposes an output under `name`, to be read from the pipeline.
    pub fn expose<T: EventSchema>(
        &mut self,
        name: impl Into<String>,
        from: OutputRef<T>,
    ) -> Result<ExposedRef<T>, GraphError> {
        let index = self.expose_key(name.into(), from.key)?;
        Ok(ExposedRef {
            index,
            _payload: PhantomData,
        })
    }

    /// The samples output of `source`, for a graph built from a description.
    pub fn source_output(&self, source: SourceId) -> Result<DynOutput, GraphError> {
        if source.0 >= self.sources.len() {
            return Err(GraphError::UnknownPort {
                owner: self.owner_name(Owner::Source(source.0)),
                direction: "output",
                index: 0,
            });
        }
        Ok(DynOutput {
            key: PortKey {
                owner: Owner::Source(source.0),
                index: 0,
            },
        })
    }

    /// Output port `name` of `node`, for a graph built from a description.
    pub fn output(&self, node: NodeId, name: &str) -> Result<DynOutput, GraphError> {
        let entry = self
            .nodes
            .get(node.0)
            .ok_or(GraphError::UnknownNode(node.0))?;
        let index = entry
            .spec
            .outputs
            .iter()
            .position(|output| output.name == name)
            .ok_or_else(|| GraphError::UnknownPortName {
                owner: self.owner_name(Owner::Node(node.0)),
                direction: "output",
                name: name.to_owned(),
            })?;
        Ok(DynOutput {
            key: PortKey {
                owner: Owner::Node(node.0),
                index,
            },
        })
    }

    /// Input port `name` of `node`, for a graph built from a description.
    pub fn input(&self, node: NodeId, name: &str) -> Result<DynInput, GraphError> {
        let entry = self
            .nodes
            .get(node.0)
            .ok_or(GraphError::UnknownNode(node.0))?;
        let index = entry
            .spec
            .inputs
            .iter()
            .position(|input| input.name == name)
            .ok_or_else(|| GraphError::UnknownPortName {
                owner: self.owner_name(Owner::Node(node.0)),
                direction: "input",
                name: name.to_owned(),
            })?;
        Ok(DynInput {
            node: node.0,
            index,
        })
    }

    /// Connects ports named at run time, with the checks of [`Graph::connect`]; the schema
    /// check is what keeps the event types straight here.
    pub fn connect_dyn(&mut self, from: DynOutput, to: DynInput) -> Result<(), GraphError> {
        self.connect_keys(from.key, to.node, to.index)
    }

    /// Exposes an output named at run time under `name`.
    pub fn expose_dyn(
        &mut self,
        name: impl Into<String>,
        from: DynOutput,
    ) -> Result<DynExposedRef, GraphError> {
        Ok(DynExposedRef {
            index: self.expose_key(name.into(), from.key)?,
        })
    }

    /// The schema of the output behind a port named at run time.
    pub fn schema_of(&self, output: DynOutput) -> Result<SchemaId, GraphError> {
        self.output_spec(output.key).map(|spec| spec.schema)
    }

    fn connect_keys(&mut self, from: PortKey, node: usize, index: usize) -> Result<(), GraphError> {
        let output = self.output_spec(from)?;
        let input = self.input_spec(node, index)?.clone();
        let from_name = self.port_name(from.owner, output.name);
        let to_name = self.port_name(Owner::Node(node), input.name);
        if output.schema != input.schema {
            return Err(GraphError::SchemaMismatch {
                from: from_name,
                to: to_name,
                output: output.schema,
                input: input.schema,
            });
        }
        if let Some((key, requirement)) = input.requires.unmet(&output.props) {
            let found = output
                .props
                .get(key)
                .map_or_else(|| "nothing".to_owned(), |v| v.to_string());
            let requirement = match requirement {
                Requirement::Equals(value) => format!("= {value}"),
                Requirement::AtLeast(least) => format!(">= {least}"),
            };
            return Err(GraphError::Requirement {
                from: from_name,
                to: to_name,
                key,
                requirement,
                found,
            });
        }
        let slot = &mut self.nodes[node].inputs[index];
        if slot.is_some() {
            return Err(GraphError::AlreadyConnected { to: to_name });
        }
        *slot = Some(from);
        Ok(())
    }

    fn expose_key(&mut self, name: String, from: PortKey) -> Result<usize, GraphError> {
        self.output_spec(from)?;
        if self.exposed.iter().any(|exposure| exposure.name == name) {
            return Err(GraphError::DuplicateExposure(name));
        }
        self.exposed.push(Exposure { name, from });
        Ok(self.exposed.len() - 1)
    }

    /// The nodes in an order where every node comes after the ones it reads.
    fn order(&self) -> Result<Vec<usize>, GraphError> {
        let mut pending: Vec<usize> = self
            .nodes
            .iter()
            .map(|entry| {
                entry
                    .inputs
                    .iter()
                    .filter(|input| {
                        matches!(
                            input,
                            Some(PortKey {
                                owner: Owner::Node(_),
                                ..
                            })
                        )
                    })
                    .count()
            })
            .collect();
        let mut ready: VecDeque<usize> = (0..self.nodes.len())
            .filter(|&node| pending[node] == 0)
            .collect();
        let mut order = Vec::with_capacity(self.nodes.len());
        while let Some(node) = ready.pop_front() {
            order.push(node);
            for (next, entry) in self.nodes.iter().enumerate() {
                let reads = entry
                    .inputs
                    .iter()
                    .filter(|input| matches!(input, Some(PortKey { owner: Owner::Node(n), .. }) if *n == node))
                    .count();
                if reads > 0 {
                    pending[next] -= reads;
                    if pending[next] == 0 {
                        ready.push_back(next);
                    }
                }
            }
        }
        if order.len() < self.nodes.len() {
            let stuck = (0..self.nodes.len())
                .find(|node| !order.contains(node))
                .unwrap_or(0);
            return Err(GraphError::Cycle {
                node: self.owner_name(Owner::Node(stuck)),
            });
        }
        Ok(order)
    }

    /// The time domain (source) of each node, in `order`.
    fn domains(&self, order: &[usize]) -> Result<Vec<Option<usize>>, GraphError> {
        let mut domains: Vec<Option<usize>> = vec![None; self.nodes.len()];
        for &node in order {
            let mut domain: Option<usize> = None;
            for input in self.nodes[node].inputs.iter().flatten() {
                let read = match input.owner {
                    Owner::Source(source) => Some(source),
                    Owner::Node(upstream) => domains[upstream],
                };
                match (domain, read) {
                    (Some(first), Some(second)) if first != second => {
                        return Err(GraphError::TimeDomain {
                            node: self.owner_name(Owner::Node(node)),
                            first: self.owner_name(Owner::Source(first)),
                            second: self.owner_name(Owner::Source(second)),
                        });
                    }
                    (None, Some(read)) => domain = Some(read),
                    _ => {}
                }
            }
            domains[node] = domain;
        }
        Ok(domains)
    }

    /// Checks the graph and starts its nodes.
    pub fn build(self) -> Result<Pipeline, GraphError> {
        for (index, entry) in self.nodes.iter().enumerate() {
            for (port, input) in entry.inputs.iter().enumerate() {
                if input.is_none() {
                    return Err(GraphError::Unconnected {
                        to: self.port_name(Owner::Node(index), entry.spec.inputs[port].name),
                    });
                }
            }
        }
        let order = self.order()?;
        let domains = self.domains(&order)?;
        // What each input reads: the properties of the output behind it.
        let input_props: Vec<Vec<Props>> = self
            .nodes
            .iter()
            .map(|entry| {
                entry
                    .inputs
                    .iter()
                    .flatten()
                    .map(|key| self.output_spec(*key).map(|spec| spec.props))
                    .collect::<Result<Vec<_>, _>>()
            })
            .collect::<Result<_, _>>()?;
        let mut exposed_names = BTreeSet::new();
        for exposure in &self.exposed {
            if !exposed_names.insert(exposure.name.as_str()) {
                return Err(GraphError::DuplicateExposure(exposure.name.clone()));
            }
        }
        let Graph {
            config,
            sources,
            nodes,
            exposed,
        } = self;
        let (nodes, specs, inputs): (Vec<_>, Vec<_>, Vec<_>) = nodes
            .into_iter()
            .map(|entry| (entry.node, entry.spec, entry.inputs))
            .fold(
                (Vec::new(), Vec::new(), Vec::new()),
                |mut acc, (node, spec, input)| {
                    acc.0.push(node);
                    acc.1.push(spec);
                    acc.2.push(input);
                    acc
                },
            );
        Pipeline::start(
            config,
            sources,
            nodes,
            specs,
            inputs,
            order,
            domains,
            input_props,
            exposed,
        )
    }
}
