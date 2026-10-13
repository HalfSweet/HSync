// SPDX-License-Identifier: Apache-2.0

//! A pipeline as a file or a session describes it.

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

/// A pipeline: a source, named nodes, the connections between their ports and the outputs
/// to expose. In JSON:
///
/// ```json
/// {
///   "source": { "channels": 4, "rate": 25000000 },
///   "nodes": {
///     "spi": { "type": "spi", "config": { "clk": 0, "cs": 1, "mosi": 2, "miso": 3 } },
///     "flash": { "type": "spi-nor" }
///   },
///   "connections": [
///     { "from": "source", "to": "spi.logic" },
///     { "from": "spi.events", "to": "flash.bus" }
///   ],
///   "expose": { "spi": "spi.events", "nor": "flash.events" }
/// }
/// ```
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Description {
    /// The logic samples the pipeline reads.
    pub source: SourceDescription,
    /// The nodes by name; a name is not `source` and has no dot.
    pub nodes: IndexMap<String, NodeDescription>,
    /// Connections from `source` or `node.port` to `node.port`.
    #[serde(default)]
    pub connections: Vec<Connection>,
    /// The outputs to expose, by the name the reader gets, as `node.port`.
    #[serde(default)]
    pub expose: IndexMap<String, String>,
    /// Items each queue holds at most (default 256).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub queue: Option<usize>,
    /// Operations a node may run per pass (default 1024).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub budget: Option<u32>,
}

/// The source of logic samples.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SourceDescription {
    /// Channels, 1 to 64.
    pub channels: u8,
    /// Samples per second (default 1 MHz).
    #[serde(default = "default_rate")]
    pub rate: u64,
    /// Blocks the source holds at most.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocks: Option<usize>,
}

fn default_rate() -> u64 {
    1_000_000
}

/// A node: its type by name and its configuration, as the node's configuration type reads it.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct NodeDescription {
    /// The node type, as [`crate::KINDS`] names it.
    #[serde(rename = "type")]
    pub kind: String,
    /// The configuration; absent or `{}` for the defaults.
    #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
    pub config: serde_json::Value,
}

/// A connection from an output (`source` or `node.port`) to an input (`node.port`).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Connection {
    /// The output.
    pub from: String,
    /// The input.
    pub to: String,
}
