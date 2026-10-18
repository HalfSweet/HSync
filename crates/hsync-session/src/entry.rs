//! A decoder of a capture as the interface and the files describe it: a uiharu pipeline — its
//! nodes, their connections and the outputs shown as lanes — with a label and the display
//! settings the interface keeps with it; and the catalog of node types the interface offers.

use std::collections::{BTreeMap, BTreeSet};

use hsync_core::ChannelIndex;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;
use uiharu_registry::{
    Catalog, Connection, Description, FieldKind, KindRef, NodeDescription, SourceDescription,
};

/// A decoder entry: a pipeline description without its source, which the capture supplies.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DecoderEntry {
    /// The nodes by name, the bus first.
    pub nodes: IndexMap<String, NodeDescription>,
    /// Connections between `source` or `node.port` and `node.port`.
    #[serde(default)]
    pub connections: Vec<Connection>,
    /// The outputs shown as lanes, by name, in lane order.
    #[serde(default)]
    pub expose: IndexMap<String, String>,
    /// What the user called the decoder.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Display settings the interface keeps; decoding does not read them.
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub show: Value,
}

impl DecoderEntry {
    /// Reads an entry.
    pub fn parse(value: &Value) -> Result<Self, serde_json::Error> {
        serde_json::from_value(value.clone())
    }

    /// Whether `value` is one of HSync's entries rather than DSView's.
    pub fn is_entry(value: &Value) -> bool {
        value.get("nodes").is_some()
    }

    /// The pipeline over a source of `channels` channels at `rate` samples per second.
    pub fn description(&self, channels: u8, rate: u64) -> Description {
        Description {
            source: SourceDescription {
                channels,
                rate,
                blocks: None,
            },
            nodes: self.nodes.clone(),
            connections: self.connections.clone(),
            expose: self.expose.clone(),
            queue: None,
            budget: None,
        }
    }

    /// The capture channels the pipeline reads, its node types among those of `catalog`.
    pub fn channels(&self, catalog: &Catalog) -> BTreeSet<ChannelIndex> {
        hsync_uiharu::channels_read(&self.description(64, 1), catalog)
    }

    /// Channels the source must have: one past the highest channel read, within a block's
    /// 64.
    pub fn source_channels(&self, catalog: &Catalog) -> u8 {
        self.channels(catalog)
            .iter()
            .next_back()
            .map_or(1, |&last| {
                u8::try_from(last.saturating_add(1)).unwrap_or(64).min(64)
            })
    }

    /// The node types, in the entry's order.
    pub fn kinds(&self) -> Vec<String> {
        self.nodes.values().map(|node| node.kind.clone()).collect()
    }

    /// The label shown: the user's, else the first node's type.
    pub fn label(&self) -> String {
        self.label
            .clone()
            .filter(|label| !label.trim().is_empty())
            .or_else(|| self.kinds().into_iter().next())
            .unwrap_or_default()
    }
}

/// A node type the interface can add, from its manifest.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CatalogEntry {
    /// The type's name, e.g. `i2c`.
    pub name: String,
    /// `bus`, `line` or `device`.
    pub category: String,
    pub inputs: Vec<CatalogPort>,
    pub outputs: Vec<CatalogPort>,
    pub config: Vec<CatalogField>,
    pub events: Vec<CatalogEvent>,
    /// The file of the decoder module the type comes from; `None` for one built in.
    pub module: Option<String>,
    /// What a loaded module says of itself, by language tag; none for a type built in, whose
    /// copy the interface has.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub locales: Option<BTreeMap<String, CatalogLocale>>,
}

/// What a loaded module shows for itself in one language: its name, a one-line summary and
/// the labels of its configuration fields, their enumerated values, its event kinds and their
/// fields, under the names the catalog entry uses; what is left out shows as the interface's
/// own copy or as the identifier.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CatalogLocale {
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub summary: Option<String>,
    pub fields: BTreeMap<String, String>,
    pub values: BTreeMap<String, BTreeMap<String, String>>,
    pub events: BTreeMap<String, String>,
    pub event_fields: BTreeMap<String, String>,
}

impl From<&uiharu_registry::Locale> for CatalogLocale {
    fn from(locale: &uiharu_registry::Locale) -> Self {
        Self {
            name: locale.name.clone(),
            summary: locale.summary.clone(),
            fields: locale.fields.clone(),
            values: locale.values.clone(),
            events: locale.events.clone(),
            event_fields: locale.event_fields.clone(),
        }
    }
}

/// A decoder module loaded into the app: a decoder written in the DSL, interpreted from the
/// IR its file holds (see `uiharu/docs/dsl.md` §8).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ModuleInfo {
    /// The node type the module provides, under the module's name.
    pub name: String,
    /// The file it was loaded from.
    pub path: String,
    /// `bus`, `line` or `device`.
    pub category: String,
    /// Whether a decoder built into the app has the same name, which the module stands in
    /// for while it is loaded.
    pub shadows: bool,
}

/// A port of a node type.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[ts(export)]
pub struct CatalogPort {
    pub port: String,
    /// The schema of the events it carries, e.g. `i2c.events`; `logic.samples` for samples.
    pub schema: String,
}

/// A configuration field of a node type.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CatalogField {
    pub name: String,
    pub kind: CatalogFieldKind,
    /// Whether the field may be left unset (`null`).
    pub optional: bool,
    /// The value taken when the field is left out.
    #[ts(type = "unknown")]
    pub default: Value,
}

/// What a configuration field holds.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase")]
#[ts(export)]
pub enum CatalogFieldKind {
    Channel,
    Channels { max: Option<u8> },
    Integer { min: Option<i64>, max: Option<i64> },
    Enum { values: Vec<String> },
    Bool,
    Text,
}

/// A kind of event a node type puts out.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[ts(export)]
pub struct CatalogEvent {
    pub kind: String,
    pub fields: Vec<String>,
}

fn catalog_entry(kind: &KindRef<'_>, module: Option<&str>) -> CatalogEntry {
    let manifest = kind.manifest();
    let defaults = Some(kind.defaults());
    let port = |port: &uiharu_registry::Port| CatalogPort {
        port: port.port.clone(),
        schema: port.schema.clone(),
    };
    CatalogEntry {
        name: manifest.name.clone(),
        category: category_name(manifest.category),
        inputs: manifest.inputs.iter().map(port).collect(),
        outputs: manifest.outputs.iter().map(port).collect(),
        config: manifest
            .config
            .iter()
            .map(|field| CatalogField {
                name: field.name.clone(),
                kind: match &field.kind {
                    FieldKind::Channel => CatalogFieldKind::Channel,
                    FieldKind::Channels { max } => CatalogFieldKind::Channels { max: *max },
                    FieldKind::Integer { min, max } => CatalogFieldKind::Integer {
                        min: *min,
                        max: *max,
                    },
                    FieldKind::Enum { values } => CatalogFieldKind::Enum {
                        values: values.clone(),
                    },
                    FieldKind::Bool => CatalogFieldKind::Bool,
                    FieldKind::Text => CatalogFieldKind::Text,
                },
                optional: field.optional,
                default: defaults
                    .as_ref()
                    .and_then(|defaults| defaults.get(&field.name).cloned())
                    .unwrap_or(Value::Null),
            })
            .collect(),
        events: manifest
            .events
            .iter()
            .map(|event| CatalogEvent {
                kind: event.kind.clone(),
                fields: event.fields.clone(),
            })
            .collect(),
        module: module.map(ToOwned::to_owned),
        locales: (!manifest.locales.is_empty()).then(|| {
            manifest
                .locales
                .iter()
                .map(|(tag, locale)| (tag.clone(), CatalogLocale::from(locale)))
                .collect()
        }),
    }
}

/// `bus`, `line` or `device`, as the manifests spell a category.
pub(crate) fn category_name(category: uiharu_registry::Category) -> String {
    serde_json::to_value(category)
        .ok()
        .and_then(|value| value.as_str().map(ToOwned::to_owned))
        .unwrap_or_default()
}

/// Every node type the interface can add, with its manifest and defaults: the decoders built
/// in alone.
pub fn catalog() -> Vec<CatalogEntry> {
    catalog_of(&Catalog::new(), |_| None)
}

/// Every node type of `catalog` the interface can add, with its manifest and defaults;
/// `module_of` names the file a loaded module came from.
pub fn catalog_of(
    catalog: &Catalog,
    module_of: impl Fn(&str) -> Option<String>,
) -> Vec<CatalogEntry> {
    catalog
        .kinds()
        .iter()
        .map(|kind| {
            let module = kind.is_loaded().then(|| module_of(kind.name())).flatten();
            catalog_entry(kind, module.as_deref())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn an_entry_describes_a_pipeline_over_the_capture() {
        let entry = DecoderEntry::parse(&json!({
            "nodes": {
                "i2c": { "type": "i2c", "config": { "scl": 9, "sda": 2 } },
                "eeprom": { "type": "eeprom24xx" }
            },
            "connections": [
                { "from": "source", "to": "i2c.logic" },
                { "from": "i2c.events", "to": "eeprom.bus" }
            ],
            "expose": { "i2c": "i2c.events", "eeprom": "eeprom.operations" },
            "label": "EEPROM"
        }))
        .expect("parses");
        let catalog = Catalog::new();
        assert_eq!(
            entry.channels(&catalog).into_iter().collect::<Vec<_>>(),
            [2, 9]
        );
        assert_eq!(entry.source_channels(&catalog), 10);
        assert_eq!(entry.kinds(), ["i2c", "eeprom24xx"]);
        assert_eq!(entry.label(), "EEPROM");
        let description = entry.description(10, 25_000_000);
        assert_eq!(description.source.channels, 10);
        assert_eq!(description.source.rate, 25_000_000);
        assert_eq!(
            description.expose.keys().collect::<Vec<_>>(),
            ["i2c", "eeprom"]
        );
        uiharu_registry::build(&description).expect("builds");
        assert!(DecoderEntry::is_entry(&json!({ "nodes": {} })));
        assert!(!DecoderEntry::is_entry(&json!({ "id": "1:i2c" })));
        assert!(DecoderEntry::parse(&json!({ "nodes": {}, "channel": [] })).is_err());
    }

    #[test]
    fn the_label_falls_back_to_the_first_node_type() {
        let entry = DecoderEntry::parse(&json!({
            "nodes": { "u": { "type": "uart", "config": { "line": 5 } } },
            "label": "  "
        }))
        .expect("parses");
        assert_eq!(entry.label(), "uart");
        let catalog = Catalog::new();
        assert_eq!(entry.source_channels(&catalog), 6);
        let empty = DecoderEntry::parse(&json!({ "nodes": {} })).expect("parses");
        assert_eq!(empty.source_channels(&catalog), 1);
        assert_eq!(empty.label(), "");
    }

    #[test]
    fn the_catalog_lists_every_node_type_with_its_defaults() {
        let catalog = catalog();
        assert_eq!(catalog.len(), 37);
        let uart = catalog
            .iter()
            .find(|entry| entry.name == "uart")
            .expect("uart");
        assert_eq!(uart.category, "bus");
        assert_eq!(uart.inputs[0].schema, "logic.samples");
        assert_eq!(uart.outputs[0].schema, "uart.events");
        let baud = uart
            .config
            .iter()
            .find(|field| field.name == "baud")
            .expect("baud");
        assert_eq!(baud.default, json!(115_200));
        assert!(matches!(
            baud.kind,
            CatalogFieldKind::Integer {
                min: Some(1),
                max: None
            }
        ));
        let parity = uart
            .config
            .iter()
            .find(|field| field.name == "parity")
            .expect("parity");
        assert!(matches!(&parity.kind, CatalogFieldKind::Enum { values } if values[0] == "none"));
        assert!(uart.events.iter().any(|event| event.kind == "frame"));
        let eeprom = catalog
            .iter()
            .find(|entry| entry.name == "eeprom24xx")
            .expect("eeprom24xx");
        assert_eq!(eeprom.category, "device");
        assert_eq!(eeprom.inputs[0].schema, "i2c.events");
        assert!(
            eeprom
                .config
                .iter()
                .any(|field| field.name == "device_address" && field.optional)
        );
        assert!(catalog.iter().all(|entry| entry.module.is_none()));
    }
}
