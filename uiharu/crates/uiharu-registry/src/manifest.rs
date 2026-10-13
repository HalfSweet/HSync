// SPDX-License-Identifier: Apache-2.0

//! What a user interface needs to know about a node type: its ports, the fields of its
//! configuration and the kinds of events it emits. Manifests are the JSON files in
//! `manifests/`, one per node type, embedded into the registry; the user-visible names of
//! everything they mention are the interface's business (i18n keys such as
//! `decoder.<type>.title` and `decoder.<type>.field.<name>`), never text here.

use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

use crate::kinds::KINDS;

/// A node type as the interface offers it.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    /// The node type, as [`KINDS`] names it.
    pub name: String,
    /// Where the catalog lists it.
    pub category: Category,
    /// The inputs, in port order.
    pub inputs: Vec<Port>,
    /// The outputs, in port order.
    pub outputs: Vec<Port>,
    /// The configuration fields, in the order the interface shows them.
    #[serde(default)]
    pub config: Vec<Field>,
    /// The kinds of events the first output emits, with their fields.
    #[serde(default)]
    pub events: Vec<EventKind>,
}

/// A group of the catalog.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Category {
    /// A bus decoded from logic samples: SPI, I²C, UART, CAN…
    Bus,
    /// A device or protocol decoded from a bus's events: SPI NOR, 24xx EEPROM, Modbus RTU…
    Device,
    /// A line or signal decoded from logic samples: PWM, timing, parallel, infrared…
    Line,
}

/// The schema name of logic samples, what a bus decoder's input carries.
pub const LOGIC: &str = "logic.samples";

/// A port: its name and the schema of what it carries ([`LOGIC`] for samples).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Port {
    /// The port's name.
    pub port: String,
    /// The schema's name: `spi.events`, or [`LOGIC`] for samples.
    pub schema: String,
}

/// A configuration field.
///
/// (No `deny_unknown_fields` here: serde does not combine it with the flattened kind.)
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Field {
    /// The field's name in the configuration.
    pub name: String,
    /// What the field holds.
    #[serde(flatten)]
    pub kind: FieldKind,
    /// Whether the field may be left unset (`null`).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub optional: bool,
}

/// What a configuration field holds.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum FieldKind {
    /// A channel of the source.
    Channel,
    /// Channels of the source, in order.
    Channels {
        /// The most channels the list takes, when bounded.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max: Option<u8>,
    },
    /// An integer.
    Integer {
        /// The smallest value accepted, when bounded.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        min: Option<i64>,
        /// The largest value accepted, when bounded.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max: Option<i64>,
    },
    /// One of a few names.
    Enum {
        /// The names, as the configuration spells them.
        values: Vec<String>,
    },
    /// A yes or no.
    Bool,
    /// Free text.
    Text,
}

/// A kind of event and its fields.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct EventKind {
    /// The `kind` tag.
    pub kind: String,
    /// The fields, in the order the contract lists them.
    pub fields: Vec<String>,
}

/// The manifests of every node type, in the order of [`KINDS`].
///
/// # Panics
///
/// When a manifest embedded in the registry does not parse; the registry's tests keep that
/// from happening.
pub fn manifests() -> &'static [Manifest] {
    static ALL: OnceLock<Vec<Manifest>> = OnceLock::new();
    ALL.get_or_init(|| {
        KINDS
            .iter()
            .map(|kind| {
                serde_json::from_str(kind.manifest).unwrap_or_else(|error| {
                    panic!("the manifest of {} does not parse: {error}", kind.name)
                })
            })
            .collect()
    })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use uiharu_core::{MAX_CHANNELS, Node};

    use super::*;
    use crate::kinds::expected_variants;

    /// A field's kind guessed from its default value, for a skeleton the maintainer refines.
    fn guess(name: &str, value: &serde_json::Value) -> (FieldKind, bool) {
        let channel_like = [
            "clk", "cs", "mosi", "miso", "sda", "scl", "line", "data", "rx", "tx", "cc", "mdc",
            "mdio", "sk", "di", "do", "channel", "a", "b", "index", "fsync", "bclk",
        ];
        match value {
            serde_json::Value::Null => (FieldKind::Channel, true),
            serde_json::Value::Bool(_) => (FieldKind::Bool, false),
            serde_json::Value::Number(_) if channel_like.contains(&name) => {
                (FieldKind::Channel, false)
            }
            serde_json::Value::Number(_) => (
                FieldKind::Integer {
                    min: None,
                    max: None,
                },
                false,
            ),
            serde_json::Value::String(text) => (
                FieldKind::Enum {
                    values: vec![text.clone()],
                },
                false,
            ),
            serde_json::Value::Array(_) => (FieldKind::Channels { max: None }, false),
            serde_json::Value::Object(_) => (FieldKind::Text, false),
        }
    }

    /// Writes a skeleton manifest for every node type that has none yet, from the node
    /// itself; run with `UIHARU_WRITE_MANIFESTS=1` after adding a decoder, then refine the
    /// fields and list the event kinds by hand. Manifests that exist are left alone.
    #[test]
    fn write_manifest_skeletons() {
        if std::env::var_os("UIHARU_WRITE_MANIFESTS").is_none() {
            return;
        }
        for kind in KINDS {
            let path = format!(
                "{}/manifests/{}.json",
                env!("CARGO_MANIFEST_DIR"),
                kind.name
            );
            if std::path::Path::new(&path).exists() {
                continue;
            }
            let defaults = (kind.defaults)();
            let node = (kind.build)(&defaults).expect("the defaults build");
            let spec = node.spec();
            let port = |name: &str, schema: uiharu_core::SchemaId| Port {
                port: name.to_owned(),
                schema: if schema.name == "logic" {
                    "logic".to_owned()
                } else {
                    schema.name.to_owned()
                },
            };
            let inputs: Vec<Port> = spec
                .inputs
                .iter()
                .map(|input| port(input.name, input.schema))
                .collect();
            let outputs: Vec<Port> = spec
                .outputs
                .iter()
                .map(|output| port(output.name, output.schema))
                .collect();
            let config: Vec<Field> = defaults
                .as_object()
                .map(|fields| {
                    fields
                        .iter()
                        .map(|(name, value)| {
                            let (kind, optional) = guess(name, value);
                            Field {
                                name: name.clone(),
                                kind,
                                optional,
                            }
                        })
                        .collect()
                })
                .unwrap_or_default();
            let category = if inputs.iter().any(|input| input.schema == LOGIC) {
                Category::Bus
            } else {
                Category::Device
            };
            let manifest = Manifest {
                name: kind.name.to_owned(),
                category,
                inputs,
                outputs,
                config,
                events: Vec::new(),
            };
            let text = serde_json::to_string_pretty(&manifest).expect("serializes");
            std::fs::write(&path, text + "\n").expect("writes the skeleton");
        }
    }

    #[test]
    fn every_manifest_parses_and_names_its_kind() {
        for (kind, manifest) in KINDS.iter().zip(manifests()) {
            assert_eq!(manifest.name, kind.name);
        }
        assert_eq!(manifests().len(), KINDS.len());
    }

    #[test]
    fn manifests_list_the_configuration_fields_and_ports_of_their_nodes() {
        for (kind, manifest) in KINDS.iter().zip(manifests()) {
            let defaults = (kind.defaults)();
            let declared: BTreeSet<&str> = manifest
                .config
                .iter()
                .map(|field| field.name.as_str())
                .collect();
            let actual: BTreeSet<&str> = defaults
                .as_object()
                .map(|fields| fields.keys().map(String::as_str).collect())
                .unwrap_or_default();
            assert_eq!(declared, actual, "{}: configuration fields", kind.name);
            let node = (kind.build)(&defaults).expect("the defaults build");
            let spec = node.spec();
            let inputs: Vec<&str> = spec.inputs.iter().map(|input| input.name).collect();
            let declared_inputs: Vec<&str> = manifest
                .inputs
                .iter()
                .map(|port| port.port.as_str())
                .collect();
            assert_eq!(declared_inputs, inputs, "{}: inputs", kind.name);
            let outputs: Vec<&str> = spec.outputs.iter().map(|output| output.name).collect();
            let declared_outputs: Vec<&str> = manifest
                .outputs
                .iter()
                .map(|port| port.port.as_str())
                .collect();
            assert_eq!(declared_outputs, outputs, "{}: outputs", kind.name);
            for (declared, actual) in manifest.inputs.iter().zip(&spec.inputs) {
                let expected = if actual.schema.name == "logic" {
                    "logic"
                } else {
                    actual.schema.name
                };
                assert_eq!(
                    declared.schema, expected,
                    "{}: input {}",
                    kind.name, actual.name
                );
            }
            for (declared, actual) in manifest.outputs.iter().zip(&spec.outputs) {
                assert_eq!(
                    declared.schema, actual.schema.name,
                    "{}: output {}",
                    kind.name, actual.name
                );
            }
        }
    }

    #[test]
    fn enum_fields_accept_their_default() {
        for (kind, manifest) in KINDS.iter().zip(manifests()) {
            let defaults = (kind.defaults)();
            for field in &manifest.config {
                if let FieldKind::Enum { values } = &field.kind
                    && let Some(serde_json::Value::String(default)) = defaults.get(&field.name)
                {
                    assert!(
                        values.contains(default),
                        "{}: {} lists {default}",
                        kind.name,
                        field.name
                    );
                }
            }
        }
    }

    #[test]
    fn manifests_list_the_event_kinds_of_their_contracts() {
        for (kind, manifest) in KINDS.iter().zip(manifests()) {
            let kinds = (kind.kinds)();
            assert!(
                !kinds.is_empty(),
                "{}: serde names no event kind",
                kind.name
            );
            let declared: Vec<&str> = manifest
                .events
                .iter()
                .map(|event| event.kind.as_str())
                .collect();
            assert_eq!(declared, kinds, "{}: event kinds", kind.name);
        }
    }

    #[test]
    fn enum_fields_list_every_name_of_their_enum() {
        for (kind, manifest) in KINDS.iter().zip(manifests()) {
            for field in &manifest.config {
                let FieldKind::Enum { values } = &field.kind else {
                    continue;
                };
                // Given a name its enum does not have, the configuration names those it has.
                let mut config = (kind.defaults)();
                config[field.name.as_str()] = serde_json::Value::from("\u{0}");
                let Err(error) = (kind.build)(&config) else {
                    panic!("{}: {} takes a name no variant has", kind.name, field.name);
                };
                assert_eq!(
                    *values,
                    expected_variants(&error),
                    "{}: {}",
                    kind.name,
                    field.name
                );
            }
        }
    }

    #[test]
    fn integer_fields_default_within_their_bounds() {
        for (kind, manifest) in KINDS.iter().zip(manifests()) {
            let defaults = (kind.defaults)();
            for field in &manifest.config {
                let FieldKind::Integer { min, max } = &field.kind else {
                    continue;
                };
                let default = &defaults[field.name.as_str()];
                if field.optional && default.is_null() {
                    continue;
                }
                let value = default
                    .as_i64()
                    .expect("an integer field defaults to an integer");
                assert!(
                    min.is_none_or(|min| min <= value) && max.is_none_or(|max| value <= max),
                    "{}: {} defaults to {value}, outside {min:?} to {max:?}",
                    kind.name,
                    field.name
                );
            }
        }
    }

    #[test]
    fn integer_fields_refuse_numbers_beyond_their_bounds() {
        for (kind, manifest) in KINDS.iter().zip(manifests()) {
            for field in &manifest.config {
                let FieldKind::Integer { min, max } = &field.kind else {
                    continue;
                };
                let beyond = [
                    min.and_then(|min| min.checked_sub(1)),
                    max.and_then(|max| max.checked_add(1)),
                ];
                for value in beyond.into_iter().flatten() {
                    let mut config = (kind.defaults)();
                    config[field.name.as_str()] = value.into();
                    assert!(
                        (kind.build)(&config).is_err(),
                        "{}: {} takes {value}, beyond its bounds",
                        kind.name,
                        field.name
                    );
                }
            }
        }
    }

    #[test]
    fn channel_fields_default_to_channels_a_source_has() {
        let channel = |value: &serde_json::Value| {
            value
                .as_u64()
                .is_some_and(|channel| channel < u64::from(MAX_CHANNELS))
        };
        for (kind, manifest) in KINDS.iter().zip(manifests()) {
            let defaults = (kind.defaults)();
            for field in &manifest.config {
                let default = &defaults[field.name.as_str()];
                match &field.kind {
                    FieldKind::Channel if !(field.optional && default.is_null()) => {
                        assert!(
                            channel(default),
                            "{}: {} defaults to {default}",
                            kind.name,
                            field.name
                        );
                    }
                    FieldKind::Channels { max } => {
                        let channels = default
                            .as_array()
                            .expect("a channel list defaults to a list");
                        assert!(
                            channels.iter().all(channel)
                                && max.is_none_or(|max| channels.len() <= usize::from(max)),
                            "{}: {} defaults to {default}",
                            kind.name,
                            field.name
                        );
                    }
                    _ => {}
                }
            }
        }
    }

    #[test]
    fn only_optional_fields_may_be_left_unset() {
        for (kind, manifest) in KINDS.iter().zip(manifests()) {
            for field in &manifest.config {
                let mut config = (kind.defaults)();
                config[field.name.as_str()] = serde_json::Value::Null;
                assert_eq!(
                    (kind.build)(&config).is_ok(),
                    field.optional,
                    "{}: {} set to null",
                    kind.name,
                    field.name
                );
            }
        }
    }
}
