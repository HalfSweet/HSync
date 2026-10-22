// SPDX-License-Identifier: Apache-2.0

//! The node types a pipeline may name: the engine's own ([`KINDS`]) and decoder modules loaded
//! at run time — the IR of decoders written in the DSL, run by `uiharu-interp` — which a
//! [`Catalog`] holds, each with the manifest a user interface needs, by the name a
//! description uses. A loaded module named like one of the engine's node types stands in for
//! it, so that a decoder under work replaces the one built in.

use std::sync::Arc;

use indexmap::IndexMap;
use uiharu_core::{DynExposedRef, Node, Pipeline, SchemaId};
use uiharu_interp::{DynEvent, Interpreter, LoadError, Program, snake};
use uiharu_ir::{Category as IrCategory, Module, Type, TypeKind};

use crate::dynamic::{DynReader, TypedReader};
use crate::kinds::{KINDS, Kind};
use crate::manifest::{
    Category, EventKind, Field, FieldKind, LOGIC, Manifest, Port, Role, manifests,
};

/// A decoder module loaded into a catalog.
pub struct LoadedKind {
    name: String,
    manifest: Manifest,
    program: Arc<Program>,
    defaults: serde_json::Value,
    kinds: Vec<String>,
}

impl std::fmt::Debug for LoadedKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LoadedKind")
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}

impl LoadedKind {
    /// The name a description uses: the module's.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The manifest, from the module's declarations.
    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    /// The module, loaded.
    pub fn program(&self) -> &Arc<Program> {
        &self.program
    }

    /// The default configuration, as the node reads it.
    pub fn defaults(&self) -> &serde_json::Value {
        &self.defaults
    }

    /// Whether a node type of the engine has the same name, which this one stands in for.
    pub fn shadows_builtin(&self) -> bool {
        KINDS.iter().any(|kind| kind.name == self.name)
    }

    /// A decoder of the module for `config`, and where it reports its faults.
    pub fn build(&self, config: &serde_json::Value) -> Result<Interpreter, String> {
        Interpreter::new(Arc::clone(&self.program), config).map_err(|error| error.to_string())
    }
}

/// Why a module could not be loaded.
#[derive(Debug, thiserror::Error)]
pub enum CatalogError {
    /// The JSON is not a module.
    #[error("the module does not parse: {0}")]
    Json(#[from] serde_json::Error),
    /// The module does not run; see [`LoadError`].
    #[error(transparent)]
    Load(#[from] LoadError),
    /// The module's defaults are refused by its own check.
    #[error("the default configuration is refused: {0}")]
    Defaults(String),
}

/// A node type, the engine's own or loaded.
#[derive(Clone, Copy, Debug)]
pub enum KindRef<'a> {
    /// One of [`KINDS`].
    Builtin(&'static Kind),
    /// A module loaded into the catalog.
    Loaded(&'a LoadedKind),
}

impl KindRef<'_> {
    /// The name a description uses.
    pub fn name(&self) -> &str {
        match self {
            KindRef::Builtin(kind) => kind.name,
            KindRef::Loaded(kind) => &kind.name,
        }
    }

    /// The manifest.
    pub fn manifest(&self) -> Manifest {
        match self {
            KindRef::Builtin(kind) => manifests()
                .iter()
                .find(|manifest| manifest.name == kind.name)
                .cloned()
                .unwrap_or_else(|| unreachable!("every node type has a manifest")),
            KindRef::Loaded(kind) => kind.manifest.clone(),
        }
    }

    /// The default configuration, as JSON.
    pub fn defaults(&self) -> serde_json::Value {
        match self {
            KindRef::Builtin(kind) => (kind.defaults)(),
            KindRef::Loaded(kind) => kind.defaults.clone(),
        }
    }

    /// The `kind` tags of the events the output carries, in the contract's order.
    pub fn event_kinds(&self) -> Vec<String> {
        match self {
            KindRef::Builtin(kind) => (kind.kinds)(),
            KindRef::Loaded(kind) => kind.kinds.clone(),
        }
    }

    /// Whether the node is a loaded module, interpreted.
    pub fn is_loaded(&self) -> bool {
        matches!(self, KindRef::Loaded(_))
    }

    /// Whether the node's input carries logic samples.
    pub(crate) fn reads_samples(&self) -> bool {
        match self {
            KindRef::Builtin(kind) => kind.reads_samples(),
            KindRef::Loaded(kind) => kind.program.input_schema().is_none(),
        }
    }

    /// Builds the node from its configuration, which it validates; a loaded node comes with
    /// where it reports its faults.
    pub(crate) fn build(
        &self,
        config: &serde_json::Value,
    ) -> Result<(Box<dyn Node>, Option<uiharu_interp::FaultHandle>), String> {
        match self {
            KindRef::Builtin(kind) => (kind.build)(config).map(|node| (node, None)),
            KindRef::Loaded(kind) => {
                let node = kind.build(config)?;
                let fault = node.fault_handle();
                Ok((Box::new(node), Some(fault)))
            }
        }
    }

    /// Opens a reader on the node's output.
    pub(crate) fn open(
        &self,
        pipeline: &mut Pipeline,
        exposed: DynExposedRef,
        name: String,
    ) -> Option<Box<dyn DynReader>> {
        match self {
            KindRef::Builtin(kind) => (kind.open)(pipeline, exposed, name),
            KindRef::Loaded(kind) => Some(Box::new(TypedReader::with_schema(
                name,
                pipeline.exposed_dyn::<DynEvent>(exposed)?,
                kind.program.output_schema(),
            ))),
        }
    }
}

impl Kind {
    fn reads_samples(&self) -> bool {
        manifests()
            .iter()
            .find(|manifest| manifest.name == self.name)
            .is_some_and(|manifest| manifest.inputs.iter().any(|input| input.schema == LOGIC))
    }

    /// The schema of the node's output.
    pub(crate) fn schema(&self) -> SchemaId {
        (self.bridge)().schema
    }
}

/// The node types a pipeline may name: the engine's, and the modules loaded here.
#[derive(Clone, Debug, Default)]
pub struct Catalog {
    loaded: IndexMap<String, Arc<LoadedKind>>,
}

impl Catalog {
    /// The engine's node types alone.
    pub fn new() -> Self {
        Self::default()
    }

    /// Loads `module`: checks it, compiles it and describes it; a module of the same name
    /// loaded before is replaced, and a node type of the engine with that name is stood in
    /// for from now on.
    pub fn load(&mut self, module: &Module) -> Result<&LoadedKind, CatalogError> {
        let program = Arc::new(Program::load(module)?);
        let defaults = Interpreter::new(Arc::clone(&program), &serde_json::Value::Null)
            .map_err(|error| CatalogError::Defaults(error.to_string()))?
            .config();
        let kinds = program.event_kinds();
        let manifest = manifest_of(module, &program, &defaults);
        let loaded = Arc::new(LoadedKind {
            name: module.name.clone(),
            manifest,
            program,
            defaults,
            kinds,
        });
        self.loaded.insert(module.name.clone(), Arc::clone(&loaded));
        Ok(self
            .loaded
            .get(&module.name)
            .map(Arc::as_ref)
            .unwrap_or_else(|| unreachable!("just loaded")))
    }

    /// Loads a module from its JSON, as `pnpm dsl:compile` writes it; see [`Catalog::load`].
    pub fn load_json(&mut self, json: &str) -> Result<&LoadedKind, CatalogError> {
        let module: Module = serde_json::from_str(json)?;
        self.load(&module)
    }

    /// Forgets the module `name`; whether there was one.
    pub fn unload(&mut self, name: &str) -> bool {
        self.loaded.shift_remove(name).is_some()
    }

    /// The modules loaded, in the order they were.
    pub fn loaded(&self) -> impl Iterator<Item = &LoadedKind> {
        self.loaded.values().map(Arc::as_ref)
    }

    /// The node type named `name`: a loaded module first, the engine's otherwise.
    pub fn get(&self, name: &str) -> Option<KindRef<'_>> {
        if let Some(kind) = self.loaded.get(name) {
            return Some(KindRef::Loaded(kind));
        }
        KINDS
            .iter()
            .find(|kind| kind.name == name)
            .map(KindRef::Builtin)
    }

    /// Every node type: the engine's in their order, each stood in for by the loaded module
    /// of its name when there is one, then the other loaded modules in the order they were
    /// loaded.
    pub fn kinds(&self) -> Vec<KindRef<'_>> {
        let mut kinds: Vec<KindRef<'_>> = KINDS
            .iter()
            .map(|kind| self.get(kind.name).unwrap_or(KindRef::Builtin(kind)))
            .collect();
        for loaded in self.loaded.values() {
            if !KINDS.iter().any(|kind| kind.name == loaded.name) {
                kinds.push(KindRef::Loaded(loaded));
            }
        }
        kinds
    }

    /// The manifests of every node type, in the order of [`Catalog::kinds`].
    pub fn manifests(&self) -> Vec<Manifest> {
        self.kinds().iter().map(KindRef::manifest).collect()
    }
}

/// The manifest of a loaded module, from its declarations.
fn manifest_of(module: &Module, program: &Program, defaults: &serde_json::Value) -> Manifest {
    let category = match program.category() {
        IrCategory::Bus => Category::Bus,
        IrCategory::Device => Category::Device,
        IrCategory::Line => Category::Line,
    };
    let inputs = match program.input_schema() {
        None => vec![Port {
            port: "logic".into(),
            schema: LOGIC.into(),
        }],
        Some(schema) => vec![Port {
            port: "bus".into(),
            schema: schema.name.into(),
        }],
    };
    let outputs = vec![Port {
        port: "events".into(),
        schema: program.output_schema().name.into(),
    }];
    let types = || {
        module
            .types
            .iter()
            .chain(module.imports.iter().flat_map(|import| import.types.iter()))
    };
    let enum_values = |name: &str| -> Option<Vec<String>> {
        types()
            .find(|def| def.name == name)
            .and_then(|def| match &def.kind {
                TypeKind::Enum { variants } => Some(variants.iter().map(|v| v.0.clone()).collect()),
                _ => None,
            })
    };
    let config = defaults.as_object().map_or_else(Vec::new, |object| {
        let fields = types()
            .find(|def| def.name == program.config_type())
            .and_then(|def| match &def.kind {
                TypeKind::Record { fields } => Some(fields.clone()),
                _ => None,
            })
            .unwrap_or_default();
        object
            .keys()
            .zip(&fields)
            .map(|(name, field)| {
                let (ty, optional) = match &field.ty {
                    Type::Option { option } => ((**option).clone(), true),
                    other => (other.clone(), false),
                };
                let kind = match &ty {
                    _ if field.json == Some(uiharu_ir::FieldJson::HexU64) => FieldKind::Text,
                    _ if field.channel => match &ty {
                        Type::Array { bound, .. } => FieldKind::Channels {
                            max: Some(bound_u8(*bound)),
                        },
                        Type::FixedArray { length, .. } => FieldKind::Channels {
                            max: Some(bound_u8(*length)),
                        },
                        _ => FieldKind::Channel,
                    },
                    Type::Name(name) => match name.as_str() {
                        "bool" => FieldKind::Bool,
                        "u8" => FieldKind::Integer {
                            min: Some(0),
                            max: Some(i64::from(u8::MAX)),
                        },
                        "u16" => FieldKind::Integer {
                            min: Some(0),
                            max: Some(i64::from(u16::MAX)),
                        },
                        "u32" => FieldKind::Integer {
                            min: Some(0),
                            max: Some(i64::from(u32::MAX)),
                        },
                        "u64" => FieldKind::Integer {
                            min: Some(0),
                            max: None,
                        },
                        "text" => FieldKind::Text,
                        other => match enum_values(other) {
                            Some(values) => FieldKind::Enum { values },
                            None => types()
                                .find(|def| def.name == other)
                                .and_then(|def| {
                                    if let TypeKind::NumericEnum { variants } = &def.kind {
                                        Some(FieldKind::Integer {
                                            min: variants
                                                .iter()
                                                .map(|variant| variant.value)
                                                .min()
                                                .and_then(|n| i64::try_from(n).ok()),
                                            max: variants
                                                .iter()
                                                .map(|variant| variant.value)
                                                .max()
                                                .and_then(|n| i64::try_from(n).ok()),
                                        })
                                    } else {
                                        None
                                    }
                                })
                                .unwrap_or(FieldKind::Text),
                        },
                    },
                    Type::Array { .. }
                    | Type::FixedArray { .. }
                    | Type::Batch { .. }
                    | Type::Option { .. } => FieldKind::Text,
                };
                Field {
                    name: name.clone(),
                    kind,
                    optional,
                }
            })
            .collect()
    });
    let events = module
        .imports
        .iter()
        .map(|import| &import.contract)
        .chain(module.contracts.iter())
        .find(|contract| {
            contract.schema.name == program.output_schema().name
                && contract.schema.version == program.output_schema().version
        })
        .and_then(|contract| {
            types()
                .find(|def| def.name == contract.event)
                .map(|def| (contract, def))
        })
        .and_then(|(contract, def)| match &def.kind {
            TypeKind::Union { variants } => Some(
                variants
                    .iter()
                    .map(|variant| {
                        // A module compiled before roles were declared shows its events as data.
                        let role = contract.roles.get(&variant.name);
                        EventKind {
                            kind: variant.name.clone(),
                            role: role.map_or(Role::Data, |role| role.role),
                            fields: variant
                                .fields
                                .iter()
                                .map(|field| field.name.clone())
                                .collect(),
                            error_when: role
                                .map(|role| role.error_when.clone())
                                .unwrap_or_default(),
                        }
                    })
                    .collect(),
            ),
            _ => None,
        })
        .unwrap_or_default();
    // The copy keys its configuration fields as the module names them; the manifest's are
    // named as the configuration reads them.
    let locales = module
        .decoder
        .as_ref()
        .map(|decoder| {
            decoder
                .locales
                .iter()
                .map(|(tag, locale)| {
                    let mut named = locale.clone();
                    named.fields = locale
                        .fields
                        .iter()
                        .map(|(name, text)| (snake(name), text.clone()))
                        .collect();
                    named.values = locale
                        .values
                        .iter()
                        .map(|(name, values)| (snake(name), values.clone()))
                        .collect();
                    (tag.clone(), named)
                })
                .collect()
        })
        .unwrap_or_default();
    Manifest {
        name: module.name.clone(),
        category,
        inputs,
        outputs,
        config,
        events,
        locales,
    }
}

fn bound_u8(bound: u32) -> u8 {
    u8::try_from(bound).unwrap_or(u8::MAX)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn module(name: &str) -> Module {
        let path =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!("../../dsl/ir/{name}.json"));
        let json = std::fs::read_to_string(&path).unwrap_or_else(|error| {
            panic!(
                "{} reads: {error}; run `pnpm dsl:generate` before Cargo tests",
                path.display()
            )
        });
        serde_json::from_str(&json).expect("the module parses")
    }

    #[test]
    fn a_loaded_module_stands_in_for_the_node_type_of_its_name() {
        let mut catalog = Catalog::new();
        assert!(matches!(catalog.get("pwm"), Some(KindRef::Builtin(_))));
        assert_eq!(catalog.kinds().len(), KINDS.len());
        let loaded = catalog.load(&module("pwm")).expect("loads");
        assert_eq!(loaded.name(), "pwm");
        assert!(loaded.shadows_builtin());
        assert!(matches!(catalog.get("pwm"), Some(KindRef::Loaded(_))));
        assert_eq!(catalog.kinds().len(), KINDS.len());
        assert_eq!(
            catalog
                .kinds()
                .iter()
                .filter(|kind| kind.is_loaded())
                .count(),
            1
        );
        assert!(catalog.unload("pwm"));
        assert!(!catalog.unload("pwm"));
        assert!(matches!(catalog.get("pwm"), Some(KindRef::Builtin(_))));
        assert!(catalog.get("no-such-type").is_none());
    }

    #[test]
    fn native_wire_config_fields_keep_numeric_and_hex_json_values() {
        let mut declared = module("pwm");
        declared.types.push(
            serde_json::from_value(serde_json::json!({
                "name": "Clause", "type": "numeric-enum", "variants": [
                    { "name": "c22", "value": 22 }, { "name": "c45", "value": 45 }
                ]
            }))
            .expect("a numeric enum declaration"),
        );
        let config_name = declared
            .decoder
            .as_ref()
            .expect("a decoder")
            .config
            .ty
            .clone();
        let config = declared
            .types
            .iter_mut()
            .find(|def| def.name == config_name)
            .expect("configuration type");
        let TypeKind::Record { fields } = &mut config.kind else {
            panic!("configuration is a record")
        };
        for field in [
            serde_json::json!({ "name": "clause", "type": "Clause" }),
            serde_json::json!({ "name": "romId", "type": "u64", "json": "hex-u64" }),
        ] {
            fields.push(serde_json::from_value(field).expect("configuration field"));
        }
        let defaults = &mut declared
            .decoder
            .as_mut()
            .expect("a decoder")
            .config
            .defaults;
        defaults["clause"] = serde_json::json!(22);
        defaults["romId"] = serde_json::json!("0xffffffffffffffff");
        let mut catalog = Catalog::new();
        let loaded = catalog.load(&declared).expect("exact defaults load");
        assert_eq!(loaded.defaults()["clause"], serde_json::json!(22));
        assert_eq!(
            loaded.defaults()["rom_id"],
            serde_json::json!("0xffffffffffffffff")
        );
        let field = |name: &str| {
            &loaded
                .manifest()
                .config
                .iter()
                .find(|field| field.name == name)
                .expect("field is described")
                .kind
        };
        assert_eq!(
            field("clause"),
            &FieldKind::Integer {
                min: Some(22),
                max: Some(45)
            }
        );
        assert_eq!(field("rom_id"), &FieldKind::Text);
        assert!(loaded.build(&serde_json::json!({ "clause": 45 })).is_ok());
        assert!(loaded.build(&serde_json::json!({ "clause": 23 })).is_err());
        assert!(
            loaded
                .build(&serde_json::json!({ "clause": "45" }))
                .is_err()
        );
    }

    #[test]
    fn a_loaded_module_describes_itself_as_the_built_in_manifest_does() {
        let mut catalog = Catalog::new();
        for kind in KINDS {
            let name = kind.name;
            let dsl = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../dsl");
            // Nodes without a TS module need no intermediate. Every declared TS module
            // requires its ignored IR; missing generation must not skip catalog checks.
            if !dsl.join(format!("decoders/{name}.ts")).is_file()
                && !dsl.join(format!("contracts/{name}.ts")).is_file()
            {
                continue;
            }
            let declared = module(name);
            if declared.decoder.is_none() {
                continue;
            }
            let loaded = catalog.load(&declared).expect("loads");
            let builtin = manifests()
                .iter()
                .find(|manifest| manifest.name == name)
                .expect("a built-in manifest");
            let manifest = loaded.manifest();
            assert_eq!(manifest.name, builtin.name);
            assert_eq!(manifest.category, builtin.category);
            assert_eq!(manifest.inputs, builtin.inputs);
            assert_eq!(manifest.outputs, builtin.outputs);
            assert_eq!(manifest.events, builtin.events);
            if !builtin.locales.is_empty() {
                assert_eq!(manifest.locales, builtin.locales, "{name}: localized copy");
            }
            let names = |fields: &[Field]| -> Vec<(String, bool)> {
                fields
                    .iter()
                    .map(|field| (field.name.clone(), field.optional))
                    .collect()
            };
            assert_eq!(names(&manifest.config), names(&builtin.config), "{name}");
            for (field, expected) in manifest.config.iter().zip(&builtin.config) {
                let same_kind = match (&field.kind, &expected.kind) {
                    (FieldKind::Integer { .. }, FieldKind::Integer { .. }) => true,
                    (a, b) => a == b,
                };
                assert!(
                    same_kind,
                    "{name}: {}: {:?} vs {:?}",
                    field.name, field.kind, expected.kind
                );
            }
            let builtin_defaults = (kind.defaults)();
            assert_eq!(loaded.defaults(), &builtin_defaults, "{name}");
        }
    }

    #[test]
    fn a_loaded_module_carries_its_copy_under_the_manifest_names() {
        let mut module = module("uart");
        let decoder = module.decoder.as_mut().expect("a decoder");
        let locale: uiharu_ir::Locale = serde_json::from_value(serde_json::json!({
            "name": "UART",
            "fields": { "dataBits": "Data bits", "line": "Line" },
            "values": { "stopBits": { "one-and-a-half": "1.5 bits" } },
            "events": { "break": "Break" },
            "eventFields": { "framingOk": "Framing OK" }
        }))
        .expect("a locale");
        decoder.locales.insert("en-US".into(), locale);
        let mut catalog = Catalog::new();
        let manifest = catalog.load(&module).expect("loads").manifest().clone();
        let en = &manifest.locales["en-US"];
        assert_eq!(en.name.as_deref(), Some("UART"));
        assert_eq!(
            en.fields.iter().collect::<Vec<_>>(),
            [
                (&"data_bits".to_string(), &"Data bits".to_string()),
                (&"line".to_string(), &"Line".to_string())
            ]
        );
        assert_eq!(en.values["stop_bits"]["one-and-a-half"], "1.5 bits");
        assert_eq!(en.events["break"], "Break");
        assert_eq!(en.event_fields["framingOk"], "Framing OK");
        assert!(
            manifest
                .config
                .iter()
                .any(|field| field.name == "data_bits")
        );
    }

    #[test]
    fn a_loaded_producer_describes_the_exact_imported_output_contract() {
        let mut declared = module("pwm");
        declared.name = "independent-pwm".into();
        declared.decoder.as_mut().expect("a decoder").name = declared.name.clone();
        let event = declared.types.remove(0);
        let contract = declared.contracts.remove(0);
        declared.imports.push(
            serde_json::from_value(serde_json::json!({
                "module": "pwm", "types": [event], "contract": contract
            }))
            .expect("the imported contract parses"),
        );
        // A local contract with the same name but another version must not supply the
        // imported output's event metadata.
        let mut another = declared.imports[0].contract.clone();
        another.schema.version = 2;
        another.event = "OtherEvent".into();
        another.roles.clear();
        declared.contracts.push(another);
        declared.types.push(
            serde_json::from_value(serde_json::json!({
                "name": "OtherEvent", "type": "union", "variants": [{"name": "other"}]
            }))
            .expect("the other event parses"),
        );
        let mut catalog = Catalog::new();
        let loaded = catalog.load(&declared).expect("the imported output loads");
        let manifest = loaded.manifest();
        let builtin = manifests()
            .iter()
            .find(|m| m.name == "pwm")
            .expect("PWM manifest");
        assert_eq!(manifest.outputs, builtin.outputs);
        assert_eq!(manifest.events, builtin.events);
        assert_eq!(manifest.events[0].kind, "cycle");
        assert_eq!(manifest.events[0].fields, ["periodTicks", "activeTicks"]);
    }

    #[test]
    fn a_contract_alone_does_not_load() {
        let mut catalog = Catalog::new();
        let error = catalog.load(&module("i2c")).expect_err("refused");
        assert_eq!(error.to_string(), "decoder: the module declares no decoder");
        let error = catalog.load_json("{\"ir\": 1}").expect_err("refused");
        assert!(
            error
                .to_string()
                .starts_with("the module does not parse: missing field"),
            "{error}"
        );
        let json = serde_json::to_string(&module("uart")).expect("serializes");
        assert_eq!(catalog.load_json(&json).expect("loads").name(), "uart");
    }
}
