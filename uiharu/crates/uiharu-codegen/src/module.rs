// SPDX-License-Identifier: Apache-2.0

//! One IR module to one Rust module: contracts and their types, the configuration, the free
//! functions, the node, and the state with the decoder's methods and the trait impl the
//! runtime's main loop drives.

use std::collections::HashMap;
use std::fmt::Write;

use uiharu_ir::{
    Contract, Decoder, Expr, Function, Input, Module, Op, RequireKind, Stmt, TypeKind,
};

use crate::emit::{Emitter, Env, ParamSig, Sig, named};
use crate::names::{pascal, plain_literal, snake};
use crate::types::{Defs, Ty, emit_types, item_def};
use crate::{GenError, Kind};

/// The crate root listing the generated modules.
pub fn generate_lib(modules: &[String]) -> String {
    let mut out = String::from(
        "// SPDX-License-Identifier: Apache-2.0\n\n//! The protocol decoders generated from the decoder DSL (`uiharu/dsl/decoders/`) and the\n//! contracts of decoders written by hand (`uiharu/dsl/contracts/`), by `uiharu-codegen` from\n//! `uiharu/dsl/ir/`; do not edit. A module is one decoder — its contract's types, its\n//! configuration and its node — or one contract: its types alone.\n\n#![forbid(unsafe_code)]\n\n",
    );
    for module in modules {
        let _ = writeln!(out, "pub mod {module};");
    }
    out
}

/// The Rust source of one module.
pub fn generate(module: &Module) -> Result<String, GenError> {
    // The item types of the framed contracts, declared here or imported, as types of their own.
    let mut items = Vec::new();
    let contracts = module
        .contracts
        .iter()
        .chain(module.imports.iter().map(|import| &import.contract));
    let types: Vec<&uiharu_ir::TypeDef> = module
        .types
        .iter()
        .chain(module.imports.iter().flat_map(|import| import.types.iter()))
        .collect();
    for contract in contracts {
        let Some(framing) = &contract.framing else {
            continue;
        };
        let variants = types
            .iter()
            .find(|def| def.name == contract.event)
            .and_then(|def| match &def.kind {
                TypeKind::Union { variants } => Some(variants),
                _ => None,
            })
            .ok_or_else(|| GenError::new(format!("{} is not a union", contract.event)))?;
        items.push(item_def(&contract.event, variants, framing).map_err(GenError::new)?);
    }
    Generator::new(module, &items)?.generate()
}

struct Generator<'a> {
    module: &'a Module,
    /// The decoder; none for a module that only declares contracts.
    decoder: Option<&'a Decoder>,
    kind: Kind,
    defs: Defs<'a>,
    /// The item types of the framed contracts in scope.
    items: &'a [uiharu_ir::TypeDef],
    /// The lower contract, for stream and bus decoders.
    lower: Option<&'a Contract>,
    /// The event type of the output; empty for a module without a decoder.
    event: String,
}

impl<'a> Generator<'a> {
    fn new(module: &'a Module, items: &'a [uiharu_ir::TypeDef]) -> Result<Self, GenError> {
        let all_types = module
            .types
            .iter()
            .chain(module.imports.iter().flat_map(|import| import.types.iter()))
            .chain(items.iter());
        let defs = Defs::new(all_types);
        let Some(decoder) = module.decoder.as_ref() else {
            return Ok(Self {
                module,
                decoder: None,
                kind: Kind::Logic,
                defs,
                items,
                lower: None,
                event: String::new(),
            });
        };
        let (kind, schema) = match &decoder.input {
            Input::Logic => (Kind::Logic, None),
            Input::Stream { schema } => (Kind::Stream, Some(schema)),
            Input::Bus { schema } => (Kind::Bus, Some(schema)),
        };
        let lower = match schema {
            Some(schema) => Some(
                module
                    .imports
                    .iter()
                    .map(|import| &import.contract)
                    .chain(module.contracts.iter())
                    .find(|contract| &contract.schema == schema)
                    .ok_or_else(|| GenError::new(format!("{} is not imported", schema.name)))?,
            ),
            None => None,
        };
        let event = module
            .contracts
            .iter()
            .find(|contract| contract.schema == decoder.output.schema)
            .map(|contract| contract.event.clone())
            .ok_or_else(|| GenError::new("the output's contract is not declared here"))?;
        Ok(Self {
            module,
            decoder: Some(decoder),
            kind,
            defs,
            items,
            lower,
            event,
        })
    }

    fn decoder(&self) -> Result<&'a Decoder, GenError> {
        self.decoder
            .ok_or_else(|| GenError::new("the module has no decoder"))
    }

    fn lower_event(&self) -> Option<String> {
        self.lower.map(|contract| contract.event.clone())
    }

    fn io_type(&self) -> String {
        match self.kind {
            Kind::Logic => format!("LogicIo<{}>", self.event),
            Kind::Stream => format!(
                "StreamIo<{}, {}>",
                self.lower_event().unwrap_or_default(),
                self.event
            ),
            Kind::Bus => format!("BusIo<{}>", self.event),
        }
    }

    fn generate(&self) -> Result<String, GenError> {
        let mut out = String::new();
        self.header(&mut out);
        self.imports(&mut out);
        for constant in &self.module.consts {
            let ty = Ty::from_ir(&constant.ty);
            let value = match &constant.value {
                uiharu_ir::Literal::Int(n) => n.to_string(),
                uiharu_ir::Literal::Bool(b) => b.to_string(),
                uiharu_ir::Literal::Text(text) => plain_literal(text),
            };
            let _ = writeln!(
                out,
                "pub const {}: {} = {value};\n",
                constant.name,
                ty.rust(None)
            );
        }
        let lower = self.lower_event();
        let config = self
            .decoder
            .map_or("", |decoder| decoder.config.ty.as_str());
        out.push_str(&emit_types(
            &self.defs,
            &self.module.types,
            config,
            lower.as_deref(),
        ));
        for contract in &self.module.contracts {
            self.contract(&mut out, contract)?;
        }
        if self.decoder.is_none() {
            // A module of contracts for decoders written by hand: its types, and the functions
            // that go with them.
            let env = self.env()?;
            for function in &self.module.functions {
                out.push_str(&self.function(&env, function, false)?);
            }
            return Ok(out);
        }
        self.config(&mut out)?;
        let env = self.env()?;
        for function in &self.module.functions {
            out.push_str(&self.function(&env, function, false)?);
        }
        self.node(&mut out)?;
        self.state(&mut out, &env)?;
        Ok(out)
    }

    fn header(&self, out: &mut String) {
        let (what, dir) = if self.decoder.is_some() {
            ("decoder", "decoders")
        } else {
            ("contract", "contracts")
        };
        let _ = writeln!(
            out,
            "// SPDX-License-Identifier: Apache-2.0\n\n//! The `{}` {what}, generated by `uiharu-codegen` from `uiharu/dsl/ir/{}.json`; do not\n//! edit. The source is `uiharu/dsl/{dir}/{}.ts`.\n\n#![allow(\n    dead_code,\n    unused_imports,\n    unused_mut,\n    unused_variables,\n    unused_parens,\n    clippy::style,\n    clippy::complexity,\n    clippy::needless_return\n)]\n",
            self.module.name, self.module.name, self.module.name
        );
    }

    fn imports(&self, out: &mut String) {
        out.push_str("use std::sync::Arc;\n\nuse serde::{Deserialize, Serialize};\nuse uiharu_core::dsl::{\n    Batch, BusDecoder, BusIo, ConfigError, Cut, CutKind, Edge, Frame, Framed, Int, LogicDecoder,\n    LogicIo, Partial, Read, Received, StreamDecoder, StreamIo, Taken, Timed, Transaction, mul_div,\n    mul_div_ceil, required, run_bus, run_logic, run_stream,\n};\nuse uiharu_core::{\n    ChannelMask, EventSchema, InputRef, InputSpec, Io, Levels, LogicBlock, MAX_CHANNELS, Node,\n    NodeFuture, NodeId, NodeSpec, OutputRef, OutputSpec, Props, Provenance, Requires, SchemaId,\n    Tick, TickRange,\n};\n");
        for import in &self.module.imports {
            let mut names: Vec<String> = import.types.iter().map(|def| def.name.clone()).collect();
            if import.contract.framing.is_some() {
                names.push(format!("{}Item", import.contract.event));
            }
            if !names.is_empty() {
                let _ = writeln!(
                    out,
                    "use crate::{}::{{{}}};",
                    snake(&import.module),
                    names.join(", ")
                );
            }
        }
        out.push('\n');
    }

    /// `impl EventSchema`, and for a framed contract the item type and `impl Framed`.
    fn contract(&self, out: &mut String, contract: &Contract) -> Result<(), GenError> {
        let _ = writeln!(
            out,
            "impl EventSchema for {} {{\n    const SCHEMA: SchemaId = SchemaId::new({}, {});\n}}\n",
            contract.event,
            plain_literal(&contract.schema.name),
            contract.schema.version
        );
        let Some(framing) = &contract.framing else {
            return Ok(());
        };
        let variants = match self.defs.get(&contract.event).map(|def| &def.kind) {
            Some(TypeKind::Union { variants }) => variants,
            _ => return Err(GenError::new(format!("{} is not a union", contract.event))),
        };
        let item_name = format!("{}Item", contract.event);
        let item = self
            .items
            .iter()
            .find(|def| def.name == item_name)
            .ok_or_else(|| GenError::new(format!("no item type for {}", contract.event)))?;
        let _ = writeln!(
            out,
            "/// One item of a frame of `{}`, as the cursor hands it out.",
            contract.schema.name
        );
        out.push_str(&emit_types(
            &self.defs,
            std::slice::from_ref(item),
            "",
            None,
        ));
        let begin_variant = variants
            .iter()
            .find(|variant| variant.name == framing.begin)
            .ok_or_else(|| GenError::new(format!("no variant {}", framing.begin)))?;
        let id_type = begin_variant
            .fields
            .iter()
            .find(|field| field.name == framing.id)
            .map(|field| Ty::from_ir(&field.ty))
            .ok_or_else(|| {
                GenError::new(format!("{} has no field {}", framing.begin, framing.id))
            })?;
        let end_variant = variants
            .iter()
            .find(|variant| variant.name == framing.end)
            .ok_or_else(|| GenError::new(format!("no variant {}", framing.end)))?;
        let reason_field = end_variant
            .fields
            .iter()
            .find(|field| field.name == framing.reason)
            .ok_or_else(|| {
                GenError::new(format!("{} has no field {}", framing.end, framing.reason))
            })?;
        let reason_enum = named(&Ty::from_ir(&reason_field.ty))
            .ok_or_else(|| GenError::new("the reason is an enum"))?;
        let _ = writeln!(
            out,
            "impl Framed for {} {{\n    type Id = {};\n    type Item = {item_name};\n\n    fn frame(&self) -> Frame<Self::Id, Self::Item> {{\n        match self {{",
            contract.event,
            id_type.rust(None)
        );
        let id = snake(&framing.id);
        let partial = framing.partial.as_deref().map(snake);
        let _ = writeln!(
            out,
            "            {}::{} {{ {id}, {} .. }} => Frame::Begin {{ id: *{id}, partial: {} }},",
            contract.event,
            pascal(&framing.begin),
            partial.as_ref().map_or(String::new(), |p| format!("{p},")),
            partial
                .as_ref()
                .map_or("false".to_owned(), |p| format!("*{p}"))
        );
        let single = framing.items.len() == 1;
        for kind in &framing.items {
            let variant = variants
                .iter()
                .find(|variant| &variant.name == kind)
                .ok_or_else(|| GenError::new(format!("no variant {kind}")))?;
            let fields: Vec<_> = variant
                .fields
                .iter()
                .filter(|field| field.name != framing.id)
                .collect();
            let binds: Vec<String> = fields.iter().map(|field| snake(&field.name)).collect();
            let inits: Vec<String> = fields
                .iter()
                .map(|field| {
                    let name = snake(&field.name);
                    if self.defs.is_copy(&Ty::from_ir(&field.ty)) {
                        format!("{name}: *{name}")
                    } else {
                        format!("{name}: {name}.clone()")
                    }
                })
                .collect();
            let item = if single {
                format!("{item_name} {{ {} }}", inits.join(", "))
            } else if inits.is_empty() {
                format!("{item_name}::{}", pascal(kind))
            } else {
                format!("{item_name}::{} {{ {} }}", pascal(kind), inits.join(", "))
            };
            let pattern = if binds.is_empty() {
                String::new()
            } else {
                format!(", {}", binds.join(", "))
            };
            let _ = writeln!(
                out,
                "            {}::{} {{ {id}{pattern} }} => Frame::Item {{ id: *{id}, item: {item} }},",
                contract.event,
                pascal(kind)
            );
        }
        let reason = snake(&framing.reason);
        let normal: Vec<String> = framing
            .normal
            .iter()
            .map(|value| format!("{reason_enum}::{}", pascal(value)))
            .collect();
        let _ = writeln!(
            out,
            "            {}::{} {{ {id}, {reason}, .. }} => Frame::End {{ id: *{id}, normal: matches!({reason}, {}) }},",
            contract.event,
            pascal(&framing.end),
            normal.join(" | ")
        );
        let others: Vec<&str> = variants
            .iter()
            .map(|variant| variant.name.as_str())
            .filter(|name| {
                *name != framing.begin
                    && *name != framing.end
                    && !framing.items.iter().any(|item| item == name)
            })
            .collect();
        if !others.is_empty() {
            return Err(GenError::new(format!(
                "{} has events outside its framing: {}",
                contract.event,
                others.join(", ")
            )));
        }
        out.push_str("        }\n    }\n");
        if let Some(unframed) = framing.unframed {
            let _ = writeln!(
                out,
                "\n    fn unframed() -> Option<Self::Id> {{\n        Some({unframed})\n    }}"
            );
        }
        out.push_str("}\n\n");
        Ok(())
    }

    /// `Default`, `validate`, `props` and `requires` on the configuration.
    fn config(&self, out: &mut String) -> Result<(), GenError> {
        let config = &self.decoder()?.config;
        let fields = self
            .defs
            .record_fields(&config.ty)
            .ok_or_else(|| GenError::new(format!("{} is not a record", config.ty)))?;
        let defaults = config
            .defaults
            .as_object()
            .ok_or_else(|| GenError::new("the defaults are an object"))?;
        let _ = writeln!(
            out,
            "impl Default for {} {{\n    fn default() -> Self {{\n        Self {{",
            config.ty
        );
        for field in fields {
            let value = defaults
                .get(&field.name)
                .ok_or_else(|| GenError::new(format!("no default for {}", field.name)))?;
            let _ = writeln!(
                out,
                "            {}: {},",
                snake(&field.name),
                self.json_literal(value, &Ty::from_ir(&field.ty))?
            );
        }
        out.push_str("        }\n    }\n}\n\n");
        let _ = writeln!(out, "impl {} {{", config.ty);
        out.push_str("    /// Checks the configuration.\n    pub fn validate(&self) -> Result<(), ConfigError> {\n");
        match &config.validate {
            Some(function) => {
                let _ = writeln!(
                    out,
                    "        match {}(self) {{\n            Some(message) => Err(ConfigError(message)),\n            None => Ok(()),\n        }}",
                    snake(function)
                );
            }
            None => out.push_str("        Ok(())\n"),
        }
        out.push_str("    }\n\n");
        let env = self.env()?;
        let config_ty = Ty::Named(config.ty.clone());
        out.push_str("    /// What the decoder's output port declares.\n    pub fn props(&self) -> Props {\n        let config = self;\n        Props::new()");
        for prop in &self.decoder()?.props {
            let val = Emitter::new(&env).with_config(config_ty.clone(), &prop.value)?;
            let _ = write!(
                out,
                "\n            .with({}, {})",
                plain_literal(&prop.key),
                self.prop_value(&val.code, &val.ty)?
            );
        }
        out.push_str("\n    }\n\n");
        out.push_str("    /// What the decoder's input requires of the output it reads.\n    pub fn requires(&self) -> Requires {\n        let config = self;\n        Requires::new()");
        for require in &self.decoder()?.requires {
            let values: Vec<_> = require
                .values
                .iter()
                .map(|value| Emitter::new(&env).with_config(config_ty.clone(), value))
                .collect::<Result<_, _>>()?;
            let key = plain_literal(&require.key);
            match require.requirement {
                RequireKind::AtLeast | RequireKind::AtMost => {
                    let method = if require.requirement == RequireKind::AtLeast {
                        "at_least"
                    } else {
                        "at_most"
                    };
                    let value = values.first().ok_or_else(|| GenError::new("a bound"))?;
                    let _ = write!(
                        out,
                        "\n            .{method}({key}, {})",
                        to_i64(&value.code, &value.ty)?
                    );
                }
                RequireKind::Within => {
                    let (min, max) = (values.first(), values.get(1));
                    let (min, max) = (
                        min.ok_or_else(|| GenError::new("a bound"))?,
                        max.ok_or_else(|| GenError::new("a bound"))?,
                    );
                    let _ = write!(
                        out,
                        "\n            .within({key}, {}, {})",
                        to_i64(&min.code, &min.ty)?,
                        to_i64(&max.code, &max.ty)?
                    );
                }
                RequireKind::Equals => {
                    let value = values.first().ok_or_else(|| GenError::new("a value"))?;
                    let _ = write!(
                        out,
                        "\n            .equals({key}, {})",
                        self.prop_value(&value.code, &value.ty)?
                    );
                }
                RequireKind::OneOf => {
                    let items: Vec<String> = values
                        .iter()
                        .map(|value| self.prop_value(&value.code, &value.ty))
                        .collect::<Result<_, _>>()?;
                    let _ = write!(out, "\n            .one_of({key}, [{}])", items.join(", "));
                }
            }
        }
        out.push_str("\n    }\n}\n\n");
        Ok(())
    }

    /// A property value as `PropValue` takes it.
    fn prop_value(&self, code: &str, ty: &Ty) -> Result<String, GenError> {
        match ty {
            Ty::Bool => Ok(code.to_owned()),
            Ty::IntLit => Ok(format!("{code}i64")),
            Ty::Text => Ok(format!("{code}.as_str()")),
            Ty::Named(name) if self.defs.is_enum(name) => Ok(format!("{code}.name()")),
            ty if ty.is_int() => to_i64(code, ty),
            other => Err(GenError::new(format!(
                "a {other:?} is not a property value"
            ))),
        }
    }

    fn json_literal(&self, value: &serde_json::Value, ty: &Ty) -> Result<String, GenError> {
        match (value, ty) {
            (serde_json::Value::Null, Ty::Option(_)) => Ok("None".into()),
            (value, Ty::Option(inner)) => Ok(format!("Some({})", self.json_literal(value, inner)?)),
            (serde_json::Value::Bool(b), Ty::Bool) => Ok(b.to_string()),
            (serde_json::Value::Number(n), ty) if ty.is_int() => Ok(n.to_string()),
            (serde_json::Value::String(text), Ty::Named(name)) if self.defs.is_enum(name) => {
                Ok(format!("{name}::{}", pascal(text)))
            }
            (serde_json::Value::String(text), Ty::Text) => {
                Ok(format!("String::from({})", plain_literal(text)))
            }
            (serde_json::Value::Array(items), Ty::Array(item, _)) => {
                let items: Vec<String> = items
                    .iter()
                    .map(|value| self.json_literal(value, item))
                    .collect::<Result<_, _>>()?;
                Ok(format!("vec![{}]", items.join(", ")))
            }
            (value, ty) => Err(GenError::new(format!("{value} is not a {ty:?}"))),
        }
    }

    /// The emitter's environment: signatures, state and consts.
    fn env(&self) -> Result<Env<'_>, GenError> {
        let mut functions = HashMap::new();
        for function in &self.module.functions {
            functions.insert(
                function.name.clone(),
                self.signature(function, false, false),
            );
        }
        let mut methods = HashMap::new();
        let mut state: HashMap<String, Ty> = HashMap::new();
        if let Some(decoder) = self.decoder {
            let mutating = self.mutating_methods();
            for method in &decoder.methods {
                methods.insert(
                    method.name.clone(),
                    self.signature(method, true, mutating.contains(&method.name)),
                );
            }
            for field in &decoder.state {
                state.insert(field.name.clone(), Ty::from_ir(&field.ty));
            }
            state.insert("config".into(), Ty::Named(decoder.config.ty.clone()));
            state.insert("rate".into(), Ty::U64);
            state.insert("#event".into(), Ty::Named(self.event.clone()));
        }
        Ok(Env {
            defs: &self.defs,
            consts: self
                .module
                .consts
                .iter()
                .map(|c| (c.name.clone(), Ty::from_ir(&c.ty)))
                .collect(),
            functions,
            methods,
            state,
            kind: self.kind,
            lower: self.lower_event(),
        })
    }

    fn signature(&self, function: &Function, method: bool, mutates: bool) -> Sig {
        Sig {
            params: function
                .params
                .iter()
                .map(|param| ParamSig {
                    name: param.name.clone(),
                    ty: Ty::from_ir(&param.ty),
                    mutable: param.mutable,
                })
                .collect(),
            ret: function.ret.as_ref().map_or(Ty::Unit, Ty::from_ir),
            is_async: function.is_async,
            mutates: method && (function.is_async || mutates),
        }
    }

    /// The methods that change the state, directly or through the methods they call.
    fn mutating_methods(&self) -> std::collections::HashSet<String> {
        let Some(decoder) = self.decoder else {
            return std::collections::HashSet::new();
        };
        let mut mutating: std::collections::HashSet<String> = decoder
            .methods
            .iter()
            .filter(|method| method.is_async || writes_self(&method.body))
            .map(|method| method.name.clone())
            .collect();
        loop {
            let before = mutating.len();
            for method in &decoder.methods {
                if !mutating.contains(&method.name) && calls_any(&method.body, &mutating) {
                    mutating.insert(method.name.clone());
                }
            }
            if mutating.len() == before {
                return mutating;
            }
        }
    }

    fn function(
        &self,
        env: &Env<'_>,
        function: &Function,
        method: bool,
    ) -> Result<String, GenError> {
        let sig = if method {
            env.methods.get(&function.name)
        } else {
            env.functions.get(&function.name)
        }
        .cloned()
        .ok_or_else(|| GenError::new(format!("no signature for {}", function.name)))?;
        let lower = self.lower_event();
        let mut params: Vec<String> = Vec::new();
        if method {
            params.push(if sig.mutates {
                "&mut self".into()
            } else {
                "&self".into()
            });
            if sig.is_async {
                params.push(format!("io: &mut {}", self.io_type()));
            }
        }
        for param in &sig.params {
            let name = snake(&param.name);
            let is_record = self
                .defs
                .record_fields(&named(&param.ty).unwrap_or_default())
                .is_some();
            let ty = if param.ty == Ty::Cursor {
                format!(
                    "&mut Transaction<'_, {}>",
                    lower.clone().unwrap_or_default()
                )
            } else if is_record {
                format!(
                    "{}{}",
                    if param.mutable { "&mut " } else { "&" },
                    param.ty.rust(lower.as_deref())
                )
            } else {
                param.ty.rust(lower.as_deref())
            };
            params.push(format!("{name}: {ty}"));
        }
        let ret = match (&sig.ret, sig.is_async) {
            (Ty::Unit, true) => " -> Result<(), Cut>".to_owned(),
            (ty, true) => format!(" -> Result<{}, Cut>", ty.rust(lower.as_deref())),
            (Ty::Unit, false) => String::new(),
            (ty, false) => format!(" -> {}", ty.rust(lower.as_deref())),
        };
        let body = Emitter::new(env).function(
            &sig.params,
            sig.ret.clone(),
            method,
            sig.is_async,
            &function.body,
        )?;
        let is_async = if sig.is_async { "async " } else { "" };
        let visibility = if method { "" } else { "pub " };
        Ok(format!(
            "{visibility}{is_async}fn {}({}){ret} {{\n{body}}}\n\n",
            snake(&function.name),
            params.join(", ")
        ))
    }

    fn node(&self, out: &mut String) -> Result<(), GenError> {
        let name = pascal(&self.module.name);
        let config = &self.decoder()?.config.ty;
        let event = &self.event;
        let (input_ty, input_port) = match self.kind {
            Kind::Logic => ("Arc<LogicBlock>".to_owned(), "logic"),
            Kind::Stream | Kind::Bus => (self.lower_event().unwrap_or_default(), "bus"),
        };
        let _ = writeln!(
            out,
            "/// The `{}` decoder node.\n#[derive(Clone, Debug, PartialEq, Eq)]\npub struct {name} {{\n    config: {config},\n}}\n\nimpl {name} {{\n    /// A decoder for `config`.\n    pub fn new(config: {config}) -> Result<Self, ConfigError> {{\n        config.validate()?;\n        Ok(Self {{ config }})\n    }}\n\n    /// The configuration.\n    pub fn config(&self) -> &{config} {{\n        &self.config\n    }}\n\n    /// The input of the decoder added as `node`.\n    pub fn input(node: NodeId) -> InputRef<{input_ty}> {{\n        InputRef::of_node(node, 0)\n    }}\n\n    /// The event output of the decoder added as `node`.\n    pub fn events(node: NodeId) -> OutputRef<{event}> {{\n        OutputRef::of_node(node, 0)\n    }}\n}}\n",
            self.module.name
        );
        let run = match self.kind {
            Kind::Logic => "run_logic(state, io.signal(0), io.output(0))",
            Kind::Stream => "run_stream(state, io.input(0), io.output(0))",
            Kind::Bus => "run_bus(state, io.input(0), io.output(0))",
        };
        let _ = writeln!(
            out,
            "impl Node for {name} {{\n    fn spec(&self) -> NodeSpec {{\n        NodeSpec {{\n            name: {}.into(),\n            inputs: vec![InputSpec::new::<{input_ty}>({}, self.config.requires())],\n            outputs: vec![OutputSpec::new::<{event}>(\"events\", self.config.props())],\n        }}\n    }}\n\n    fn start(self: Box<Self>, mut io: Io) -> NodeFuture {{\n        let rate = io.time_base().map_or(0, |base| base.rate_hz);\n        let state = State::new(self.config, rate);\n        Box::pin({run})\n    }}\n}}\n",
            plain_literal(&self.module.name),
            plain_literal(input_port)
        );
        Ok(())
    }

    fn state(&self, out: &mut String, env: &Env<'_>) -> Result<(), GenError> {
        let lower = self.lower_event();
        let config = &self.decoder()?.config.ty;
        out.push_str("/// The decoder's state: its configuration, the source's rate and its fields.\nstruct State {\n");
        let _ = writeln!(out, "    config: {config},\n    rate: u64,");
        for field in &self.decoder()?.state {
            let _ = writeln!(
                out,
                "    {}: {},",
                snake(&field.name),
                Ty::from_ir(&field.ty).rust(lower.as_deref())
            );
        }
        out.push_str("}\n\nimpl State {\n");
        let _ = writeln!(out, "    fn new(config: {config}, rate: u64) -> Self {{");
        let mut so_far: Vec<(String, Ty)> = vec![
            ("config".into(), Ty::Named(config.clone())),
            ("rate".into(), Ty::U64),
        ];
        let mut names = vec!["config".to_owned(), "rate".to_owned()];
        for field in &self.decoder()?.state {
            let ty = Ty::from_ir(&field.ty);
            let code = Emitter::new(env).initializer(&so_far, &field.init, &ty)?;
            let _ = writeln!(
                out,
                "        let {}: {} = {code};",
                snake(&field.name),
                ty.rust(lower.as_deref())
            );
            so_far.push((field.name.clone(), ty));
            names.push(snake(&field.name));
        }
        let _ = writeln!(out, "        Self {{ {} }}\n    }}\n", names.join(", "));
        if !self
            .decoder()?
            .methods
            .iter()
            .any(|method| method.name == "hold")
        {
            out.push_str("    fn hold(&self) -> Option<u64> {\n        None\n    }\n\n");
        }
        for method in &self.decoder()?.methods {
            let text = self.function(env, method, true)?;
            for line in text.lines() {
                if line.is_empty() {
                    out.push('\n');
                } else {
                    let _ = writeln!(out, "    {line}");
                }
            }
        }
        out.push_str("}\n\n");
        self.trait_impl(out, env)
    }

    /// The decoder trait of the state's kind: the hooks adapted to the runtime's signatures.
    fn trait_impl(&self, out: &mut String, env: &Env<'_>) -> Result<(), GenError> {
        let io = self.io_type();
        let event = &self.event;
        let has = |name: &str| env.methods.get(name).cloned();
        let hook = |name: &str, rust_name: &str, params: &str, args: &str| -> String {
            match has(name) {
                Some(sig) if sig.is_async => format!(
                    "    async fn {rust_name}(&mut self, io: &mut {io}, {params}) {{\n        let _ = State::{rust_name}(self, io, {args}).await;\n    }}\n\n"
                ),
                Some(_) => format!(
                    "    async fn {rust_name}(&mut self, io: &mut {io}, {params}) {{\n        State::{rust_name}(self, {args});\n    }}\n\n"
                ),
                None => format!(
                    "    async fn {rust_name}(&mut self, _io: &mut {io}, _{params}) {{}}\n\n"
                ),
            }
        };
        let hooks = hook("onGap", "on_gap", "range: TickRange", "range")
            + &hook("onEnd", "on_end", "tick: u64", "tick")
            + &hook("onProgress", "on_progress", "tick: u64", "tick");
        match self.kind {
            Kind::Logic => {
                let _ = write!(
                    out,
                    "impl LogicDecoder for State {{\n    type Event = {event};\n\n    fn hold(&self) -> Option<u64> {{\n        State::hold(self)\n    }}\n\n    async fn run(&mut self, io: &mut {io}) -> Result<(), Cut> {{\n        State::run(self, io).await\n    }}\n\n{hooks}}}\n"
                );
            }
            Kind::Stream => {
                let lower = self.lower_event().unwrap_or_default();
                let _ = write!(
                    out,
                    "impl StreamDecoder for State {{\n    type Lower = {lower};\n    type Event = {event};\n\n    fn hold(&self) -> Option<u64> {{\n        State::hold(self)\n    }}\n\n    async fn run(&mut self, io: &mut {io}) -> Result<(), Cut> {{\n        State::run(self, io).await\n    }}\n\n{hooks}}}\n"
                );
            }
            Kind::Bus => {
                let lower = self.lower_event().unwrap_or_default();
                let recovery = self
                    .decoder()?
                    .recovery
                    .as_ref()
                    .ok_or_else(|| GenError::new("a bus decoder has a recovery"))?;
                let partial = match self.decoder()?.partial {
                    Some(uiharu_ir::Partial::Read) => "Partial::Read",
                    _ => "Partial::Drain",
                };
                let reason = &recovery.reason;
                let _ = write!(
                    out,
                    "impl BusDecoder for State {{\n    type Lower = {lower};\n    type Event = {event};\n\n    const PARTIAL: Partial = {partial};\n\n    fn hold(&self) -> Option<u64> {{\n        State::hold(self)\n    }}\n\n    async fn transaction(&mut self, io: &mut {io}, tx: &mut Transaction<'_, {lower}>) -> Result<(), Cut> {{\n        State::transaction(self, io, tx).await\n    }}\n\n    async fn recover(&mut self, io: &mut {io}, cut: CutKind, at: Provenance) {{\n        let reason = match cut {{\n            CutKind::Incomplete => {reason}::{},\n            CutKind::Truncated => {reason}::{},\n            CutKind::Missing => {reason}::{},\n            CutKind::Gap => {reason}::{},\n            CutKind::End => {reason}::{},\n        }};\n        let _ = State::recover(self, io, reason, at).await;\n    }}\n\n{hooks}}}\n",
                    pascal(&recovery.incomplete),
                    pascal(&recovery.truncated),
                    pascal(&recovery.missing),
                    pascal(&recovery.gap),
                    pascal(&recovery.end)
                );
            }
        }
        Ok(())
    }
}

/// An integer as `i64`, for port properties.
fn to_i64(code: &str, ty: &Ty) -> Result<String, GenError> {
    match ty {
        Ty::IntLit => Ok(format!("{code}i64")),
        Ty::U64 => Ok(format!("i64::try_from({code}).unwrap_or(i64::MAX)")),
        ty if ty.is_int() => Ok(format!("i64::from({code})")),
        other => Err(GenError::new(format!("a {other:?} is not an integer"))),
    }
}

/// Whether a body assigns to the state, borrows it, or changes a batch of it.
fn writes_self(body: &[Stmt]) -> bool {
    fn self_rooted(expr: &Expr) -> bool {
        match expr {
            Expr::SelfField { .. } => true,
            Expr::Field { of, .. } | Expr::Index { of, .. } => self_rooted(of),
            _ => false,
        }
    }
    fn in_expr(expr: &Expr) -> bool {
        match expr {
            Expr::Intrinsic { name, args } => {
                (matches!(name.as_str(), "batch-push" | "batch-take" | "batch-clear")
                    && args.first().is_some_and(self_rooted))
                    || args.iter().any(in_expr)
            }
            Expr::Call { args, .. } | Expr::Method { args, .. } => args.iter().any(in_expr),
            Expr::Field { of, .. }
            | Expr::Tag { value: of }
            | Expr::Unary { value: of, .. }
            | Expr::Cast { value: of, .. }
            | Expr::Required { value: of } => in_expr(of),
            Expr::Index { of, at } => in_expr(of) || in_expr(at),
            Expr::Binary { left, right, .. } => in_expr(left) || in_expr(right),
            Expr::Record { fields, .. } | Expr::Variant { fields, .. } => {
                fields.values().any(in_expr)
            }
            Expr::Array { items } | Expr::Format { parts: items } => items.iter().any(in_expr),
            Expr::Ternary {
                cond,
                then,
                otherwise,
            } => in_expr(cond) || in_expr(then) || in_expr(otherwise),
            Expr::IfSome {
                value, some, none, ..
            } => in_expr(value) || in_expr(some) || in_expr(none),
            Expr::IfVariant {
                value,
                then,
                otherwise,
                ..
            } => in_expr(value) || in_expr(then) || in_expr(otherwise),
            Expr::Await { op } => match op {
                Op::Emit { event, at } => {
                    in_expr(event) || at.as_ref().is_some_and(|at| in_expr(at))
                }
                Op::Method { args, .. } => args.iter().any(in_expr),
                Op::NextChange { mask } => in_expr(mask),
                Op::NextChangeBefore { mask, until } => in_expr(mask) || in_expr(until),
                _ => false,
            },
            _ => false,
        }
    }
    body.iter().any(|stmt| match stmt {
        Stmt::Assign { target, value, .. } => self_rooted(target) || in_expr(value),
        Stmt::LetRef { .. } => true,
        Stmt::IfSome { borrow: true, .. } | Stmt::LetSome { borrow: true, .. } => true,
        Stmt::Let { value, .. } | Stmt::Expr { expr: value } => in_expr(value),
        Stmt::Return { value } => value.as_ref().is_some_and(in_expr),
        Stmt::If {
            cond,
            then,
            otherwise,
        } => in_expr(cond) || writes_self(then) || writes_self(otherwise),
        Stmt::IfSome {
            value,
            then,
            otherwise,
            ..
        } => in_expr(value) || writes_self(then) || writes_self(otherwise),
        Stmt::LetSome {
            value, otherwise, ..
        } => in_expr(value) || writes_self(otherwise),
        Stmt::Match { value, arms } => {
            in_expr(value) || arms.iter().any(|arm| writes_self(&arm.body))
        }
        Stmt::Loop { body, .. } => writes_self(body),
        Stmt::While { cond, body, .. } => in_expr(cond) || writes_self(body),
        Stmt::For { from, to, body, .. } => in_expr(from) || in_expr(to) || writes_self(body),
        Stmt::ForEach { array, body, .. } => in_expr(array) || writes_self(body),
        Stmt::Break { .. } | Stmt::Continue { .. } => false,
    })
}

/// Whether a body calls one of `names` as a method.
fn calls_any(body: &[Stmt], names: &std::collections::HashSet<String>) -> bool {
    fn in_expr(expr: &Expr, names: &std::collections::HashSet<String>) -> bool {
        match expr {
            Expr::Method { name, args } => {
                names.contains(name) || args.iter().any(|arg| in_expr(arg, names))
            }
            Expr::Call { args, .. } | Expr::Intrinsic { args, .. } => {
                args.iter().any(|arg| in_expr(arg, names))
            }
            Expr::Field { of, .. }
            | Expr::Tag { value: of }
            | Expr::Unary { value: of, .. }
            | Expr::Cast { value: of, .. }
            | Expr::Required { value: of } => in_expr(of, names),
            Expr::Index { of, at } => in_expr(of, names) || in_expr(at, names),
            Expr::Binary { left, right, .. } => in_expr(left, names) || in_expr(right, names),
            Expr::Record { fields, .. } | Expr::Variant { fields, .. } => {
                fields.values().any(|v| in_expr(v, names))
            }
            Expr::Array { items } | Expr::Format { parts: items } => {
                items.iter().any(|v| in_expr(v, names))
            }
            Expr::Ternary {
                cond,
                then,
                otherwise,
            } => in_expr(cond, names) || in_expr(then, names) || in_expr(otherwise, names),
            Expr::IfSome {
                value, some, none, ..
            } => in_expr(value, names) || in_expr(some, names) || in_expr(none, names),
            Expr::IfVariant {
                value,
                then,
                otherwise,
                ..
            } => in_expr(value, names) || in_expr(then, names) || in_expr(otherwise, names),
            Expr::Await { op } => match op {
                Op::Method { name, args } => {
                    names.contains(name) || args.iter().any(|arg| in_expr(arg, names))
                }
                Op::Emit { event, at } => {
                    in_expr(event, names) || at.as_ref().is_some_and(|at| in_expr(at, names))
                }
                Op::NextChange { mask } => in_expr(mask, names),
                Op::NextChangeBefore { mask, until } => {
                    in_expr(mask, names) || in_expr(until, names)
                }
                _ => false,
            },
            _ => false,
        }
    }
    body.iter().any(|stmt| match stmt {
        Stmt::Assign { target, value, .. } => in_expr(target, names) || in_expr(value, names),
        Stmt::Let { value, .. } | Stmt::Expr { expr: value } => in_expr(value, names),
        Stmt::LetRef { place, .. } => in_expr(place, names),
        Stmt::Return { value } => value.as_ref().is_some_and(|v| in_expr(v, names)),
        Stmt::If {
            cond,
            then,
            otherwise,
        } => in_expr(cond, names) || calls_any(then, names) || calls_any(otherwise, names),
        Stmt::IfSome {
            value,
            then,
            otherwise,
            ..
        } => in_expr(value, names) || calls_any(then, names) || calls_any(otherwise, names),
        Stmt::LetSome {
            value, otherwise, ..
        } => in_expr(value, names) || calls_any(otherwise, names),
        Stmt::Match { value, arms } => {
            in_expr(value, names) || arms.iter().any(|arm| calls_any(&arm.body, names))
        }
        Stmt::Loop { body, .. } => calls_any(body, names),
        Stmt::While { cond, body, .. } => in_expr(cond, names) || calls_any(body, names),
        Stmt::For { from, to, body, .. } => {
            in_expr(from, names) || in_expr(to, names) || calls_any(body, names)
        }
        Stmt::ForEach { array, body, .. } => in_expr(array, names) || calls_any(body, names),
        Stmt::Break { .. } | Stmt::Continue { .. } => false,
    })
}
