// SPDX-License-Identifier: Apache-2.0

//! The checks of the engine's rules on a module: names resolve, collections are bounded, the
//! only waits are the runtime's operations and each is used where its input exists, nothing
//! recurses, every loop waits or leaves, a borrowed field keeps `self` out of reach, and the
//! decoder has the hooks its kind calls for.

use std::collections::{BTreeMap, BTreeSet};

use crate::IR_VERSION;
use crate::model::{
    Contract, Decoder, Expr, Field, Framing, Function, Input, Module, Op, Pattern, Stmt, Type,
    TypeDef, TypeKind, UnionVariant,
};

/// A rule a module breaks, with the path of the item that breaks it.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{path}: {message}")]
pub struct Diagnostic {
    /// Where, as `decoder.methods.run.body[2].then[0]`.
    pub path: String,
    /// What.
    pub message: String,
}

/// Checks `module` against the engine's rules; an empty list means it passes.
pub fn validate(module: &Module) -> Vec<Diagnostic> {
    let mut checker = Checker::new(module);
    checker.module();
    checker.calls.check(&mut checker.diagnostics);
    checker.diagnostics
}

/// Which input the decoder reads, which decides the operations its body may use.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum InputClass {
    Logic,
    Stream,
    Bus,
}

/// The call graph, for the recursion check.
#[derive(Default)]
struct Calls {
    edges: BTreeMap<String, BTreeSet<String>>,
}

impl Calls {
    fn add(&mut self, from: &str, to: String) {
        self.edges.entry(from.to_owned()).or_default().insert(to);
    }

    fn check(&self, diagnostics: &mut Vec<Diagnostic>) {
        let mut done = BTreeSet::new();
        for start in self.edges.keys() {
            if done.contains(start) {
                continue;
            }
            let mut stack = vec![start.clone()];
            self.walk(start, &mut stack, &mut done, diagnostics);
        }
    }

    fn walk(
        &self,
        node: &str,
        stack: &mut Vec<String>,
        done: &mut BTreeSet<String>,
        diagnostics: &mut Vec<Diagnostic>,
    ) {
        for next in self.edges.get(node).into_iter().flatten() {
            if let Some(at) = stack.iter().position(|seen| seen == next) {
                let cycle = stack[at..].join(" → ");
                diagnostics.push(Diagnostic {
                    path: node.to_owned(),
                    message: format!("recursion is not allowed: {cycle} → {next}"),
                });
                continue;
            }
            if done.contains(next) {
                continue;
            }
            stack.push(next.clone());
            self.walk(next, stack, done, diagnostics);
            stack.pop();
        }
        done.insert(node.to_owned());
    }
}

/// What a body is checked in: whose it is, what it may do and what is in scope.
struct Cx {
    path: String,
    /// The call graph node: `fn:name` or `method:name`.
    owner: String,
    is_method: bool,
    is_async: bool,
    /// Locals by block, innermost last.
    scopes: Vec<Vec<String>>,
    /// The locals of each block that borrow a field of `self`, innermost last.
    borrows: Vec<Vec<String>>,
    /// The labels of the enclosing loops, innermost last.
    loops: Vec<Option<String>>,
}

impl Cx {
    fn new(path: String, owner: String, is_method: bool, is_async: bool) -> Self {
        Self {
            path,
            owner,
            is_method,
            is_async,
            scopes: vec![Vec::new()],
            borrows: vec![Vec::new()],
            loops: Vec::new(),
        }
    }

    fn declare(&mut self, name: &str) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.push(name.to_owned());
        }
    }

    fn resolves(&self, name: &str) -> bool {
        self.scopes
            .iter()
            .any(|scope| scope.iter().any(|have| have == name))
    }

    fn borrowing(&self) -> bool {
        self.borrows.iter().any(|block| !block.is_empty())
    }

    fn push_block(&mut self) {
        self.scopes.push(Vec::new());
        self.borrows.push(Vec::new());
    }

    fn pop_block(&mut self) {
        self.scopes.pop();
        self.borrows.pop();
    }
}

struct Checker<'a> {
    module: &'a Module,
    types: BTreeMap<&'a str, &'a TypeKind>,
    functions: BTreeMap<&'a str, &'a Function>,
    methods: BTreeMap<&'a str, &'a Function>,
    consts: BTreeSet<&'a str>,
    state: BTreeSet<&'a str>,
    /// What the decoder reads; `None` for a module without a decoder.
    input: Option<InputClass>,
    diagnostics: Vec<Diagnostic>,
    calls: Calls,
}

impl<'a> Checker<'a> {
    fn new(module: &'a Module) -> Self {
        let input = module.decoder.as_ref().map(|decoder| match decoder.input {
            Input::Logic => InputClass::Logic,
            Input::Stream { .. } => InputClass::Stream,
            Input::Bus { .. } => InputClass::Bus,
        });
        Self {
            module,
            types: BTreeMap::new(),
            functions: BTreeMap::new(),
            methods: BTreeMap::new(),
            consts: BTreeSet::new(),
            state: BTreeSet::new(),
            input,
            diagnostics: Vec::new(),
            calls: Calls::default(),
        }
    }

    fn report(&mut self, path: impl Into<String>, message: impl Into<String>) {
        self.diagnostics.push(Diagnostic {
            path: path.into(),
            message: message.into(),
        });
    }

    fn module(&mut self) {
        let module = self.module;
        if module.ir != IR_VERSION {
            self.report(
                "ir",
                format!("IR version {} is not {IR_VERSION}", module.ir),
            );
        }
        let declared: Vec<(String, &TypeDef)> = module
            .types
            .iter()
            .enumerate()
            .map(|(index, def)| (format!("types[{index}]"), def))
            .chain(module.imports.iter().enumerate().flat_map(|(i, import)| {
                import
                    .types
                    .iter()
                    .enumerate()
                    .map(move |(j, def)| (format!("imports[{i}].types[{j}]"), def))
            }))
            .collect();
        for (path, def) in &declared {
            if Type::is_primitive_name(&def.name) {
                self.report(path.clone(), format!("{} is a primitive type", def.name));
            }
            if self.types.insert(&def.name, &def.kind).is_some() {
                self.report(path.clone(), format!("{} is declared twice", def.name));
            }
        }
        for (path, def) in &declared {
            self.type_def(path, def);
        }
        for (index, constant) in module.consts.iter().enumerate() {
            let path = format!("consts[{index}]");
            self.type_ref(&path, &constant.ty);
            if !self.consts.insert(&constant.name) {
                self.report(path, format!("{} is declared twice", constant.name));
            }
        }
        for (index, function) in module.functions.iter().enumerate() {
            let path = format!("functions[{index}]");
            if function.is_async {
                self.report(&path, "a free function is synchronous");
            }
            if self.functions.insert(&function.name, function).is_some() {
                self.report(path, format!("{} is declared twice", function.name));
            }
        }
        if let Some(decoder) = &module.decoder {
            for (index, method) in decoder.methods.iter().enumerate() {
                if self.methods.insert(&method.name, method).is_some() {
                    self.report(
                        format!("decoder.methods[{index}]"),
                        format!("{} is declared twice", method.name),
                    );
                }
            }
            for (index, field) in decoder.state.iter().enumerate() {
                if ["config", "rate", "signal", "input", "out"].contains(&field.name.as_str())
                    || !self.state.insert(&field.name)
                {
                    self.report(
                        format!("decoder.state[{index}]"),
                        format!("{} is declared twice or reserved", field.name),
                    );
                }
            }
        } else if module.contracts.is_empty() {
            self.report("decoder", "a module declares a decoder or a contract");
        }
        let mut schemas = BTreeMap::new();
        for (path, contract) in
            module
                .contracts
                .iter()
                .enumerate()
                .map(|(index, contract)| (format!("contracts[{index}]"), contract))
                .chain(module.imports.iter().enumerate().map(|(index, import)| {
                    (format!("imports[{index}].contract"), &import.contract)
                }))
        {
            let identity = (&contract.schema.name, contract.schema.version);
            if schemas
                .insert(identity, contract)
                .is_some_and(|prior| prior != contract)
            {
                self.report(
                    path,
                    format!(
                        "{}@{} has conflicting contract declarations",
                        contract.schema.name, contract.schema.version
                    ),
                );
            }
        }
        for (index, contract) in module.contracts.iter().enumerate() {
            self.contract(&format!("contracts[{index}]"), contract);
        }
        for (index, import) in module.imports.iter().enumerate() {
            self.contract(&format!("imports[{index}].contract"), &import.contract);
        }
        for (index, function) in module.functions.iter().enumerate() {
            self.function(&format!("functions[{index}]"), function, false);
        }
        if let Some(decoder) = &module.decoder {
            self.decoder(decoder);
        }
    }

    fn type_def(&mut self, path: &str, def: &TypeDef) {
        match &def.kind {
            TypeKind::Record { fields } => {
                for (index, field) in fields.iter().enumerate() {
                    self.field(&format!("{path}.fields[{index}]"), field);
                }
            }
            TypeKind::Enum { variants } => {
                if variants.is_empty() {
                    self.report(path, "an enum has at least one variant");
                }
            }
            TypeKind::NumericEnum { variants } => {
                if variants.is_empty() {
                    self.report(path, "an enum has at least one variant");
                }
                let mut names = BTreeSet::new();
                let mut values = BTreeSet::new();
                for variant in variants {
                    if !names.insert(&variant.name) {
                        self.report(path, format!("{} is declared twice", variant.name));
                    }
                    if !values.insert(variant.value) {
                        self.report(
                            path,
                            format!("numeric value {} is declared twice", variant.value),
                        );
                    }
                }
            }
            TypeKind::ExternalEnum { variants } => {
                if variants.is_empty() {
                    self.report(path, "an enum has at least one variant");
                }
                let mut names = BTreeSet::new();
                for (index, variant) in variants.iter().enumerate() {
                    if !names.insert(&variant.name) {
                        self.report(path, format!("{} is declared twice", variant.name));
                    }
                    if let Some(payload) = &variant.payload {
                        self.type_ref(&format!("{path}.variants[{index}].payload"), payload);
                    }
                }
            }
            TypeKind::Union { variants } => {
                if variants.is_empty() {
                    self.report(path, "a union has at least one variant");
                }
                for (i, variant) in variants.iter().enumerate() {
                    for (j, field) in variant.fields.iter().enumerate() {
                        self.field(&format!("{path}.variants[{i}].fields[{j}]"), field);
                    }
                }
            }
        }
    }

    fn field(&mut self, path: &str, field: &Field) {
        self.type_ref(path, &field.ty);
        if field.json == Some(crate::model::FieldJson::HexU64) && !field.ty.is("u64") {
            self.report(path, "hex-u64 requires a u64 field");
        }
    }

    /// Checks that a type exists and that every collection in it is bounded.
    fn type_ref(&mut self, path: &str, ty: &Type) {
        match ty {
            Type::Name(name) => {
                if !Type::is_primitive_name(name) && !self.types.contains_key(name.as_str()) {
                    self.report(path, format!("type {name} is not declared"));
                }
            }
            Type::Option { option } => self.type_ref(path, option),
            Type::FixedArray {
                fixed_array,
                length,
            } => {
                if !(1..=32).contains(length) {
                    self.report(path, "a fixed array has 1 to 32 items, the native array lengths supported by serde");
                }
                self.type_ref(path, fixed_array);
            }
            Type::Array { array, bound }
            | Type::Batch {
                batch: array,
                bound,
            } => {
                if *bound == 0 {
                    self.report(path, "a collection has a bound of at least one item");
                }
                self.type_ref(path, array);
            }
        }
    }

    fn union(&self, name: &str) -> Option<&'a [crate::model::UnionVariant]> {
        match self.types.get(name) {
            Some(TypeKind::Union { variants }) => Some(variants),
            _ => None,
        }
    }

    fn enum_variants(&self, name: &str) -> Option<&'a [crate::model::EnumVariant]> {
        match self.types.get(name) {
            Some(TypeKind::Enum { variants }) => Some(variants),
            _ => None,
        }
    }

    fn contract(&mut self, path: &str, contract: &Contract) {
        let Some(variants) = self.union(&contract.event) else {
            self.report(
                path,
                format!("event type {} is not a union", contract.event),
            );
            return;
        };
        self.roles(path, contract, variants);
        let Some(framing) = &contract.framing else {
            return;
        };
        self.framing(path, framing, variants);
    }

    /// Roles, when the contract declares them: one for every kind of event, and conditions
    /// on fields the kind has, with values those fields can take.
    fn roles(&mut self, path: &str, contract: &Contract, variants: &[crate::model::UnionVariant]) {
        if contract.roles.is_empty() {
            return;
        }
        for variant in variants {
            if !contract.roles.contains_key(&variant.name) {
                self.report(path, format!("roles: {} has no role", variant.name));
            }
        }
        for (kind, role) in &contract.roles {
            let Some(variant) = variants.iter().find(|variant| &variant.name == kind) else {
                self.report(path, format!("roles: {kind} is not a variant"));
                continue;
            };
            for condition in &role.error_when {
                if condition.is_empty() {
                    self.report(
                        path,
                        format!("roles.{kind}.errorWhen: a condition names a field"),
                    );
                }
                for (name, value) in condition {
                    match variant.fields.iter().find(|field| &field.name == name) {
                        None => self.report(
                            path,
                            format!("roles.{kind}.errorWhen: {kind} has no field {name}"),
                        ),
                        Some(field) if !self.field_takes(field, value) => self.report(
                            path,
                            format!("roles.{kind}.errorWhen: {name} is never {value}"),
                        ),
                        Some(_) => {}
                    }
                }
            }
        }
    }

    /// Whether a field of type `ty` can serialize as `value`.
    fn takes(&self, ty: &Type, value: &serde_json::Value) -> bool {
        match ty {
            Type::Option { option } => value.is_null() || self.takes(option, value),
            Type::Name(name) => match name.as_str() {
                "bool" => value.is_boolean(),
                "u8" | "u16" | "u32" | "u64" => value.is_u64(),
                "text" => value.is_string(),
                other => match self.types.get(other) {
                    Some(TypeKind::Enum { variants }) => value
                        .as_str()
                        .is_some_and(|value| variants.iter().any(|variant| variant.0 == value)),
                    Some(TypeKind::NumericEnum { variants }) => value
                        .as_u64()
                        .is_some_and(|value| variants.iter().any(|variant| variant.value == value)),
                    Some(TypeKind::ExternalEnum { variants }) => {
                        if let Some(name) = value.as_str() {
                            variants
                                .iter()
                                .any(|variant| variant.name == name && variant.payload.is_none())
                        } else if let Some(object) =
                            value.as_object().filter(|object| object.len() == 1)
                        {
                            let (name, payload) = object.iter().next().expect("one key");
                            variants
                                .iter()
                                .find(|variant| &variant.name == name)
                                .and_then(|variant| variant.payload.as_ref())
                                .is_some_and(|ty| self.takes(ty, payload))
                        } else {
                            false
                        }
                    }
                    _ => false,
                },
            },
            Type::Array { .. } | Type::FixedArray { .. } | Type::Batch { .. } => false,
        }
    }

    fn field_takes(&self, field: &Field, value: &serde_json::Value) -> bool {
        if field.json == Some(crate::model::FieldJson::HexU64) {
            return value.as_str().is_some_and(|text| {
                let digits = text
                    .strip_prefix("0x")
                    .or_else(|| text.strip_prefix("0X"))
                    .unwrap_or(text);
                u64::from_str_radix(digits, 16).is_ok()
            });
        }
        self.takes(&field.ty, value)
    }

    fn framing(&mut self, path: &str, framing: &Framing, variants: &[crate::model::UnionVariant]) {
        let find = |name: &str| variants.iter().find(|variant| variant.name == name);
        if framing.items.is_empty() {
            self.report(
                path,
                "framing.items: a frame carries at least one kind of item",
            );
        }
        let mut framed: Vec<(&str, &String)> = vec![("begin", &framing.begin)];
        framed.extend(framing.items.iter().map(|item| ("items", item)));
        framed.push(("end", &framing.end));
        for (what, name) in framed {
            let Some(variant) = find(name) else {
                self.report(path, format!("framing.{what}: {name} is not a variant"));
                continue;
            };
            if !variant.fields.iter().any(|field| field.name == framing.id) {
                self.report(
                    path,
                    format!("framing.id: {name} has no field {}", framing.id),
                );
            }
        }
        if let Some(partial) = &framing.partial
            && let Some(begin) = find(&framing.begin)
            && !begin
                .fields
                .iter()
                .any(|field| &field.name == partial && field.ty.is("bool"))
        {
            self.report(
                path,
                format!(
                    "framing.partial: {} has no bool field {partial}",
                    framing.begin
                ),
            );
        }
        if let Some(end) = find(&framing.end) {
            match end.fields.iter().find(|field| field.name == framing.reason) {
                Some(field) => {
                    let variants = match &field.ty {
                        Type::Name(name) => self.enum_variants(name),
                        _ => None,
                    };
                    match variants {
                        Some(variants) => {
                            if framing.normal.is_empty() {
                                self.report(path, "framing.normal: a frame has a normal end");
                            }
                            for normal in &framing.normal {
                                if !variants.iter().any(|v| &v.0 == normal) {
                                    self.report(
                                        path,
                                        format!("framing.normal: {normal} is not a reason"),
                                    );
                                }
                            }
                        }
                        None => self.report(path, "framing.reason: the field is not an enum"),
                    }
                }
                None => self.report(
                    path,
                    format!(
                        "framing.reason: {} has no field {}",
                        framing.end, framing.reason
                    ),
                ),
            }
        }
    }

    fn schema_known(&self, schema: &crate::model::SchemaRef) -> Option<&'a Contract> {
        self.module
            .imports
            .iter()
            .map(|import| &import.contract)
            .chain(self.module.contracts.iter())
            .find(|contract| &contract.schema == schema)
    }

    fn decoder(&mut self, decoder: &Decoder) {
        let path = "decoder";
        match self.types.get(decoder.config.ty.as_str()) {
            Some(TypeKind::Record { .. }) => {}
            _ => self.report(
                format!("{path}.config"),
                format!("{} is not a record", decoder.config.ty),
            ),
        }
        if !decoder.config.defaults.is_object() {
            self.report(
                format!("{path}.config.defaults"),
                "the defaults are an object",
            );
        }
        if let Some(validate) = &decoder.config.validate {
            match self.functions.get(validate.as_str()) {
                Some(function) if function.params.len() == 1 => {}
                Some(_) => self.report(
                    format!("{path}.config.validate"),
                    "validate takes the configuration alone",
                ),
                None => self.report(
                    format!("{path}.config.validate"),
                    format!("{validate} is not a free function"),
                ),
            }
        }
        match &decoder.input {
            Input::Logic => {}
            Input::Stream { schema } => {
                if self.schema_known(schema).is_none() {
                    self.report(
                        format!("{path}.input"),
                        format!("{} is not imported", schema.name),
                    );
                }
            }
            Input::Bus { schema } => match self.schema_known(schema) {
                Some(contract) if contract.framing.is_some() => {}
                Some(_) => self.report(
                    format!("{path}.input"),
                    format!("{} is not framed; a bus input reads frames", schema.name),
                ),
                None => self.report(
                    format!("{path}.input"),
                    format!("{} is not imported", schema.name),
                ),
            },
        }
        if self.schema_known(&decoder.output.schema).is_none() {
            self.report(
                format!("{path}.output"),
                format!("{} is not declared", decoder.output.schema.name),
            );
        }
        let mut config_cx = Cx::new(
            format!("{path}.requires"),
            "decoder".to_owned(),
            false,
            false,
        );
        config_cx.declare("config");
        for (index, require) in decoder.requires.iter().enumerate() {
            config_cx.path = format!("{path}.requires[{index}]");
            let wanted = match require.requirement {
                crate::model::RequireKind::Within => 2,
                crate::model::RequireKind::OneOf => require.values.len().max(1),
                _ => 1,
            };
            if require.values.len() != wanted {
                self.report(
                    config_cx.path.clone(),
                    "the requirement has the wrong number of values",
                );
            }
            for value in &require.values {
                self.expr(&mut config_cx, value);
            }
        }
        for (index, prop) in decoder.props.iter().enumerate() {
            config_cx.path = format!("{path}.props[{index}]");
            self.expr(&mut config_cx, &prop.value);
        }
        if self.input == Some(InputClass::Bus) {
            match &decoder.recovery {
                Some(recovery) => {
                    let names = [
                        &recovery.incomplete,
                        &recovery.truncated,
                        &recovery.missing,
                        &recovery.gap,
                        &recovery.end,
                    ];
                    match self.enum_variants(&recovery.reason) {
                        Some(variants) => {
                            for name in names {
                                if !variants.iter().any(|v| &v.0 == name) {
                                    self.report(
                                        format!("{path}.recovery"),
                                        format!("{name} is not a {}", recovery.reason),
                                    );
                                }
                            }
                        }
                        None => self.report(
                            format!("{path}.recovery"),
                            format!("{} is not an enum", recovery.reason),
                        ),
                    }
                }
                None => self.report(
                    format!("{path}.recovery"),
                    "a bus decoder declares its recovery",
                ),
            }
        } else {
            if decoder.recovery.is_some() {
                self.report(
                    format!("{path}.recovery"),
                    "only a bus decoder has a recovery",
                );
            }
            if decoder.partial.is_some() {
                self.report(format!("{path}.partial"), "only a bus decoder reads frames");
            }
        }
        let mut state_cx = Cx::new(format!("{path}.state"), "decoder".to_owned(), true, false);
        for (index, field) in decoder.state.iter().enumerate() {
            state_cx.path = format!("{path}.state[{index}]");
            self.type_ref(&state_cx.path, &field.ty);
            self.expr(&mut state_cx, &field.init);
        }
        for (index, method) in decoder.methods.iter().enumerate() {
            self.function(&format!("{path}.methods[{index}]"), method, true);
        }
        self.hooks(path, decoder);
        self.locales(path, decoder);
    }

    /// Checks that the copy the decoder carries names its configuration fields, the values of
    /// its enumerated fields, the kinds of event it emits and their fields.
    fn locales(&mut self, path: &str, decoder: &Decoder) {
        let config = &decoder.config.ty;
        let config_fields: &'a [Field] = match self.types.get(config.as_str()).copied() {
            Some(TypeKind::Record { fields }) => fields,
            _ => &[],
        };
        let variants: &'a [UnionVariant] = self
            .schema_known(&decoder.output.schema)
            .and_then(|contract| self.union(&contract.event))
            .unwrap_or(&[]);
        for (tag, locale) in &decoder.locales {
            let path = format!("{path}.locales.{tag}");
            if tag.is_empty() || !tag.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
                self.report(
                    path.clone(),
                    "a locale is named by its language tag, such as en-US",
                );
            }
            for name in locale.fields.keys() {
                if !config_fields.iter().any(|field| &field.name == name) {
                    self.report(
                        format!("{path}.fields.{name}"),
                        format!("{name} is not a field of {config}"),
                    );
                }
            }
            for (name, values) in &locale.values {
                let Some(field) = config_fields.iter().find(|field| &field.name == name) else {
                    self.report(
                        format!("{path}.values.{name}"),
                        format!("{name} is not a field of {config}"),
                    );
                    continue;
                };
                let ty = match &field.ty {
                    Type::Option { option } => option.as_ref(),
                    other => other,
                };
                let variants: Option<Vec<String>> = match ty {
                    Type::Name(name) => match self.types.get(name.as_str()) {
                        Some(TypeKind::Enum { variants }) => {
                            Some(variants.iter().map(|variant| variant.0.clone()).collect())
                        }
                        Some(TypeKind::NumericEnum { variants }) => Some(
                            variants
                                .iter()
                                .map(|variant| variant.value.to_string())
                                .collect(),
                        ),
                        _ => None,
                    },
                    _ => None,
                };
                let Some(variants) = variants else {
                    self.report(
                        format!("{path}.values.{name}"),
                        format!("{name} is not an enumerated field"),
                    );
                    continue;
                };
                for value in values.keys() {
                    if !variants.iter().any(|variant| variant == value) {
                        self.report(
                            format!("{path}.values.{name}.{value}"),
                            format!("{value} is not a value of {name}"),
                        );
                    }
                }
            }
            for kind in locale.events.keys() {
                if !variants.iter().any(|variant| &variant.name == kind) {
                    self.report(
                        format!("{path}.events.{kind}"),
                        format!("{kind} is not a kind of event the decoder emits"),
                    );
                }
            }
            for name in locale.event_fields.keys() {
                if !variants
                    .iter()
                    .any(|variant| variant.fields.iter().any(|field| &field.name == name))
                {
                    self.report(
                        format!("{path}.eventFields.{name}"),
                        format!("{name} is not a field of any event"),
                    );
                }
            }
        }
    }

    /// Checks that the hooks the decoder's kind calls for are there with the right shape.
    fn hooks(&mut self, path: &str, decoder: &Decoder) {
        match self.input {
            Some(InputClass::Logic | InputClass::Stream) => {
                self.hook(path, "run", Some(true), &[], None, true);
                for name in ["transaction", "recover"] {
                    if self.methods.contains_key(name) {
                        self.report(
                            format!("{path}.methods.{name}"),
                            "only a bus decoder has it",
                        );
                    }
                }
            }
            Some(InputClass::Bus) => {
                self.hook(path, "transaction", Some(true), &["cursor"], None, true);
                let reason = decoder
                    .recovery
                    .as_ref()
                    .map(|recovery| recovery.reason.clone())
                    .unwrap_or_default();
                self.hook(
                    path,
                    "recover",
                    Some(true),
                    &[&reason, "provenance"],
                    None,
                    true,
                );
                if self.methods.contains_key("run") {
                    self.report(
                        format!("{path}.methods.run"),
                        "a bus decoder has a transaction body, not a run",
                    );
                }
            }
            None => {}
        }
        self.hook(
            path,
            "hold",
            Some(false),
            &[],
            Some(Type::option(Type::named("u64"))),
            false,
        );
        self.hook(path, "onGap", None, &["range"], None, false);
        self.hook(path, "onEnd", None, &["u64"], None, false);
        self.hook(path, "onProgress", None, &["u64"], None, false);
    }

    /// Checks one hook: whether it is async (`None` for either), its parameter types and its
    /// return type; `required` says whether it must be there at all.
    fn hook(
        &mut self,
        path: &str,
        name: &str,
        is_async: Option<bool>,
        params: &[&str],
        ret: Option<Type>,
        required: bool,
    ) {
        let Some(function) = self.methods.get(name).copied() else {
            if required {
                self.report(
                    format!("{path}.methods"),
                    format!("a {name} method is required"),
                );
            }
            return;
        };
        let where_ = format!("{path}.methods.{name}");
        if let Some(is_async) = is_async
            && function.is_async != is_async
        {
            self.report(
                where_.clone(),
                if is_async {
                    "must be async"
                } else {
                    "must be synchronous"
                },
            );
        }
        let params_match = function.params.len() == params.len()
            && function
                .params
                .iter()
                .zip(params)
                .all(|(param, ty)| param.ty.is(ty));
        if !params_match {
            self.report(where_.clone(), format!("takes ({})", params.join(", ")));
        }
        if function.ret != ret {
            self.report(where_, "returns the wrong type");
        }
    }

    fn function(&mut self, path: &str, function: &Function, is_method: bool) {
        let owner = format!(
            "{}:{}",
            if is_method { "method" } else { "fn" },
            function.name
        );
        let mut cx = Cx::new(path.to_owned(), owner, is_method, function.is_async);
        for (index, param) in function.params.iter().enumerate() {
            self.type_ref(&format!("{path}.params[{index}]"), &param.ty);
            if param.ty.is("cursor") && self.input != Some(InputClass::Bus) {
                self.report(
                    format!("{path}.params[{index}]"),
                    "only a bus decoder has a cursor",
                );
            }
            cx.declare(&param.name);
        }
        if let Some(ret) = &function.ret {
            self.type_ref(&format!("{path}.ret"), ret);
        }
        self.block(&mut cx, &format!("{path}.body"), &function.body);
    }

    fn block(&mut self, cx: &mut Cx, path: &str, body: &[Stmt]) {
        cx.push_block();
        for (index, stmt) in body.iter().enumerate() {
            self.stmt(cx, &format!("{path}[{index}]"), stmt);
        }
        cx.pop_block();
    }

    fn loop_body(
        &mut self,
        cx: &mut Cx,
        path: &str,
        label: &Option<String>,
        body: &[Stmt],
        bounded: bool,
    ) {
        if !bounded && !yields(body, &mut Vec::new()) {
            self.report(path, "a loop waits, breaks or returns in every iteration");
        }
        cx.loops.push(label.clone());
        self.block(cx, path, body);
        cx.loops.pop();
    }

    fn stmt(&mut self, cx: &mut Cx, path: &str, stmt: &Stmt) {
        cx.path = path.to_owned();
        match stmt {
            Stmt::Let {
                name, ty, value, ..
            } => {
                if let Some(ty) = ty {
                    self.type_ref(path, ty);
                }
                self.expr(cx, value);
                cx.declare(name);
            }
            Stmt::LetRef { name, place } => {
                if !cx.is_method {
                    self.report(path, "only a method borrows a field of self");
                }
                if !is_self_place(place) {
                    self.report(path, "a reference borrows a field of self");
                }
                self.expr(cx, place);
                cx.declare(name);
                if let Some(block) = cx.borrows.last_mut() {
                    block.push(name.clone());
                }
            }
            Stmt::Assign { target, value, .. } => {
                if !is_place(target) {
                    self.report(
                        path,
                        "the target of an assignment is a local, a field or an item",
                    );
                }
                self.expr(cx, target);
                self.expr(cx, value);
            }
            Stmt::If {
                cond,
                then,
                otherwise,
            } => {
                self.expr(cx, cond);
                self.block(cx, &format!("{path}.then"), then);
                self.block(cx, &format!("{path}.else"), otherwise);
            }
            Stmt::IfSome {
                value,
                bind,
                borrow,
                then,
                otherwise,
            } => {
                self.expr(cx, value);
                if *borrow && !is_self_place(value) {
                    self.report(path, "a borrow binds a field of self");
                }
                cx.push_block();
                cx.declare(bind);
                if *borrow && let Some(block) = cx.borrows.last_mut() {
                    block.push(bind.clone());
                }
                self.block(cx, &format!("{path}.then"), then);
                cx.pop_block();
                self.block(cx, &format!("{path}.else"), otherwise);
            }
            Stmt::LetSome {
                bind,
                value,
                borrow,
                otherwise,
            } => {
                self.expr(cx, value);
                if *borrow && !is_self_place(value) {
                    self.report(path, "a borrow binds a field of self");
                }
                if !leaves(otherwise) {
                    self.report(path, "the else of a let-some leaves the block");
                }
                self.block(cx, &format!("{path}.else"), otherwise);
                cx.declare(bind);
                if *borrow && let Some(block) = cx.borrows.last_mut() {
                    block.push(bind.clone());
                }
            }
            Stmt::Match { value, arms } => {
                self.expr(cx, value);
                for (index, arm) in arms.iter().enumerate() {
                    cx.push_block();
                    match &arm.pattern {
                        Pattern::Variant { binds, .. } => {
                            for local in binds.values() {
                                cx.declare(local);
                            }
                        }
                        Pattern::Values { values } if values.is_empty() => {
                            self.report(
                                format!("{path}.arms[{index}]"),
                                "an arm matches at least one value",
                            );
                        }
                        Pattern::Values { .. } | Pattern::Default => {}
                    }
                    self.block(cx, &format!("{path}.arms[{index}].body"), &arm.body);
                    cx.pop_block();
                }
            }
            Stmt::Loop { label, body } => {
                self.loop_body(cx, &format!("{path}.body"), label, body, false)
            }
            Stmt::While { label, cond, body } => {
                self.expr(cx, cond);
                self.loop_body(cx, &format!("{path}.body"), label, body, false);
            }
            Stmt::For {
                label,
                var,
                from,
                to,
                body,
            } => {
                self.expr(cx, from);
                self.expr(cx, to);
                cx.push_block();
                cx.declare(var);
                self.loop_body(cx, &format!("{path}.body"), label, body, true);
                cx.pop_block();
            }
            Stmt::ForEach {
                label,
                var,
                array,
                body,
            } => {
                self.expr(cx, array);
                cx.push_block();
                cx.declare(var);
                self.loop_body(cx, &format!("{path}.body"), label, body, true);
                cx.pop_block();
            }
            Stmt::Break { label } | Stmt::Continue { label } => {
                if cx.loops.is_empty() {
                    self.report(path, "outside a loop");
                } else if let Some(label) = label
                    && !cx.loops.iter().any(|have| have.as_ref() == Some(label))
                {
                    self.report(path, format!("no enclosing loop is labelled {label}"));
                }
            }
            Stmt::Return { value } => {
                if let Some(value) = value {
                    self.expr(cx, value);
                }
            }
            Stmt::Expr { expr } => self.expr(cx, expr),
        }
    }

    fn expr(&mut self, cx: &mut Cx, expr: &Expr) {
        let path = cx.path.clone();
        match expr {
            Expr::Lit { .. } | Expr::None => {}
            Expr::Text { .. } | Expr::Format { .. } => {
                if cx.is_method {
                    self.report(
                        path.clone(),
                        "text is for messages: only free functions make it",
                    );
                }
                if let Expr::Format { parts } = expr {
                    for part in parts {
                        self.expr(cx, part);
                    }
                }
            }
            Expr::Local { name } => {
                if !cx.resolves(name) && !self.consts.contains(name.as_str()) {
                    self.report(path, format!("{name} is not in scope"));
                }
            }
            Expr::SelfField { field } => {
                if !cx.is_method {
                    self.report(path.clone(), "self is only in scope in a method");
                }
                if !["config", "rate"].contains(&field.as_str())
                    && !self.state.contains(field.as_str())
                {
                    self.report(path, format!("self has no field {field}"));
                }
            }
            Expr::Field { of, .. } | Expr::Tag { value: of } | Expr::Unary { value: of, .. } => {
                self.expr(cx, of);
            }
            Expr::Index { of, at } => {
                self.expr(cx, of);
                self.expr(cx, at);
            }
            Expr::Binary { left, right, .. } => {
                self.expr(cx, left);
                self.expr(cx, right);
            }
            Expr::Cast { value, to } => {
                self.type_ref(&path, to);
                self.expr(cx, value);
            }
            Expr::Record { ty, fields } => {
                if !matches!(self.types.get(ty.as_str()), Some(TypeKind::Record { .. })) {
                    self.report(path.clone(), format!("{ty} is not a record"));
                }
                for value in fields.values() {
                    self.expr(cx, value);
                }
            }
            Expr::Variant { ty, kind, fields } => {
                match self.types.get(ty.as_str()) {
                    Some(TypeKind::ExternalEnum { variants }) => match variants
                        .iter()
                        .find(|variant| &variant.name == kind)
                    {
                        Some(variant)
                            if variant.payload.is_some()
                                && (fields.len() != 1 || !fields.contains_key("value")) =>
                        {
                            self.report(
                                path.clone(),
                                format!("{ty}::{kind} requires exactly the value payload"),
                            )
                        }
                        Some(variant) if variant.payload.is_none() && !fields.is_empty() => self
                            .report(
                                path.clone(),
                                format!("{ty}::{kind} unit variant takes no payload"),
                            ),
                        Some(_) => {}
                        None => self.report(path.clone(), format!("{ty} has no variant {kind}")),
                    },
                    _ => match self.union(ty) {
                        Some(variants) if variants.iter().any(|variant| &variant.name == kind) => {}
                        Some(_) => self.report(path.clone(), format!("{ty} has no variant {kind}")),
                        None => self.report(path.clone(), format!("{ty} is not a union")),
                    },
                }
                for value in fields.values() {
                    self.expr(cx, value);
                }
            }
            Expr::Enum { ty, value } => match self.types.get(ty.as_str()) {
                Some(TypeKind::Enum { variants })
                    if variants.iter().any(|variant| &variant.0 == value) => {}
                Some(TypeKind::NumericEnum { variants })
                    if variants.iter().any(|variant| &variant.name == value) => {}
                Some(TypeKind::Enum { .. } | TypeKind::NumericEnum { .. }) => {
                    self.report(path, format!("{ty} has no variant {value}"))
                }
                _ => self.report(path, format!("{ty} is not an enum")),
            },
            Expr::Array { items } => {
                if items.is_empty() {
                    self.report(path.clone(), "an array literal has at least one item");
                }
                for item in items {
                    self.expr(cx, item);
                }
            }
            Expr::Call { name, args } => {
                if self.functions.contains_key(name.as_str()) {
                    self.calls.add(&cx.owner, format!("fn:{name}"));
                } else {
                    self.report(path.clone(), format!("{name} is not a free function"));
                }
                for arg in args {
                    self.expr(cx, arg);
                }
            }
            Expr::Method { name, args } => {
                self.method_call(cx, &path, name, false);
                for arg in args {
                    self.expr(cx, arg);
                }
            }
            Expr::Intrinsic { name, args } => {
                let bus_only = ["consumed", "tx-id", "tx-partial"];
                let logic_only = ["position", "seek"];
                if bus_only.contains(&name.as_str()) && self.input != Some(InputClass::Bus) {
                    self.report(path.clone(), format!("{name} is for bus decoders"));
                }
                if logic_only.contains(&name.as_str()) && self.input != Some(InputClass::Logic) {
                    self.report(path.clone(), format!("{name} is for logic decoders"));
                }
                if !INTRINSICS.contains(&name.as_str()) {
                    self.report(path.clone(), format!("{name} is not an intrinsic"));
                }
                for arg in args {
                    self.expr(cx, arg);
                }
            }
            Expr::Ternary {
                cond,
                then,
                otherwise,
            } => {
                self.expr(cx, cond);
                self.expr(cx, then);
                self.expr(cx, otherwise);
            }
            Expr::IfSome {
                value,
                bind,
                some,
                none,
            } => {
                self.expr(cx, value);
                cx.push_block();
                cx.declare(bind);
                self.expr(cx, some);
                cx.pop_block();
                self.expr(cx, none);
            }
            Expr::IfVariant {
                value,
                binds,
                then,
                otherwise,
                ..
            } => {
                self.expr(cx, value);
                cx.push_block();
                for local in binds.values() {
                    cx.declare(local);
                }
                self.expr(cx, then);
                cx.pop_block();
                self.expr(cx, otherwise);
            }
            Expr::Required { value } => {
                if self.input != Some(InputClass::Bus) {
                    self.report(path.clone(), "a required value (`!`) is for bus decoders, whose recovery closes a missing one");
                }
                self.expr(cx, value);
            }
            Expr::Await { op } => {
                if !cx.is_async {
                    self.report(path.clone(), "only an async method awaits");
                }
                self.op(cx, &path, op);
            }
        }
    }

    fn method_call(&mut self, cx: &mut Cx, path: &str, name: &str, is_async: bool) {
        if !cx.is_method {
            self.report(path, "a method is called on self, in a method");
            return;
        }
        if cx.borrowing() {
            self.report(
                path,
                "no method of self is called while a local borrows a field of self",
            );
        }
        match self.methods.get(name) {
            Some(method) if method.is_async == is_async => {
                self.calls.add(&cx.owner, format!("method:{name}"));
            }
            Some(_) if is_async => self.report(
                path,
                format!("{name} is synchronous: call it without await"),
            ),
            Some(_) => self.report(path, format!("{name} is async: await it")),
            None => self.report(path, format!("{name} is not a method")),
        }
    }

    fn op(&mut self, cx: &mut Cx, path: &str, op: &Op) {
        let input = self.input;
        let wrong = |this: &mut Self, what: &str, class: InputClass| {
            if input != Some(class) {
                let kind = match class {
                    InputClass::Logic => "logic",
                    InputClass::Stream => "stream",
                    InputClass::Bus => "bus",
                };
                this.report(path, format!("{what} is a {kind} operation"));
            }
        };
        match op {
            Op::NextChange { mask } => {
                wrong(self, "next-change", InputClass::Logic);
                self.expr(cx, mask);
            }
            Op::NextChangeBefore { mask, until } => {
                wrong(self, "next-change-before", InputClass::Logic);
                self.expr(cx, mask);
                self.expr(cx, until);
            }
            Op::Sample => wrong(self, "sample", InputClass::Logic),
            Op::Next => wrong(self, "next", InputClass::Stream),
            Op::Item => wrong(self, "item", InputClass::Bus),
            Op::ItemOrEnd => wrong(self, "item-or-end", InputClass::Bus),
            Op::Drain => wrong(self, "drain", InputClass::Bus),
            Op::Emit { event, at } => {
                self.expr(cx, event);
                match at {
                    Some(at) => self.expr(cx, at),
                    None if input != Some(InputClass::Bus) => {
                        self.report(path, "an event states its provenance; only a bus decoder may leave it to the cursor");
                    }
                    None => {}
                }
            }
            Op::Method { name, args } => {
                self.method_call(cx, path, name, true);
                for arg in args {
                    self.expr(cx, arg);
                }
            }
        }
    }
}

/// The runtime helpers a body may call.
const INTRINSICS: [&str; 30] = [
    "min",
    "max",
    "saturating-sub",
    "count-ones",
    "from-bool",
    "mul-div",
    "mul-div-ceil",
    "at",
    "one",
    "over",
    "join",
    "mask",
    "mask-has",
    "mask-with",
    "levels-high",
    "max-channels",
    "position",
    "seek",
    "batch",
    "batch-length",
    "batch-is-empty",
    "batch-is-full",
    "batch-at",
    "batch-push",
    "batch-take",
    "batch-clear",
    "consumed",
    "tx-id",
    "tx-partial",
    "unwrap-or",
];

/// Whether the expression names a place: a local, a field of one or of `self`, an item.
fn is_place(expr: &Expr) -> bool {
    match expr {
        Expr::Local { .. } | Expr::SelfField { .. } => true,
        Expr::Field { of, .. } | Expr::Index { of, .. } => is_place(of),
        _ => false,
    }
}

/// Whether the expression is a field of `self`, or a field or item within one.
fn is_self_place(expr: &Expr) -> bool {
    match expr {
        Expr::SelfField { .. } => true,
        Expr::Field { of, .. } | Expr::Index { of, .. } => is_self_place(of),
        _ => false,
    }
}

/// Whether every path through the block leaves it: its last statement returns, breaks or
/// continues (or is an if whose branches both do).
fn leaves(block: &[Stmt]) -> bool {
    match block.last() {
        Some(Stmt::Return { .. } | Stmt::Break { .. } | Stmt::Continue { .. }) => true,
        Some(Stmt::If {
            then, otherwise, ..
        }) => leaves(then) && leaves(otherwise),
        _ => false,
    }
}

/// Whether a loop's body waits, breaks out of this loop or returns on some path. `inner` holds
/// the labels of the loops nested between this loop and the statements looked at: an
/// unlabelled break inside one of them does not leave this loop, and a labelled one leaves it
/// unless it names one of them.
fn yields(body: &[Stmt], inner: &mut Vec<Option<String>>) -> bool {
    body.iter().any(|stmt| match stmt {
        Stmt::Return { .. } => true,
        Stmt::Break { label: target } => match target {
            None => inner.is_empty(),
            Some(target) => !inner.iter().any(|label| label.as_ref() == Some(target)),
        },
        Stmt::Continue { .. } | Stmt::LetRef { .. } => false,
        Stmt::Let { value, .. } | Stmt::Expr { expr: value } => awaits(value),
        Stmt::Assign { target, value, .. } => awaits(target) || awaits(value),
        Stmt::If {
            cond,
            then,
            otherwise,
        } => awaits(cond) || yields(then, inner) || yields(otherwise, inner),
        Stmt::IfSome {
            value,
            then,
            otherwise,
            ..
        } => awaits(value) || yields(then, inner) || yields(otherwise, inner),
        Stmt::LetSome {
            value, otherwise, ..
        } => awaits(value) || yields(otherwise, inner),
        Stmt::Match { value, arms } => {
            awaits(value) || arms.iter().any(|arm| yields(&arm.body, inner))
        }
        Stmt::Loop { label, body } => nested(label, body, inner),
        Stmt::While { label, cond, body } => awaits(cond) || nested(label, body, inner),
        Stmt::For {
            label,
            from,
            to,
            body,
            ..
        } => awaits(from) || awaits(to) || nested(label, body, inner),
        Stmt::ForEach {
            label, array, body, ..
        } => awaits(array) || nested(label, body, inner),
    })
}

/// [`yields`] for the body of a loop nested in the one looked at.
fn nested(label: &Option<String>, body: &[Stmt], inner: &mut Vec<Option<String>>) -> bool {
    inner.push(label.clone());
    let found = yields(body, inner);
    inner.pop();
    found
}

/// Whether the expression awaits anywhere in it.
fn awaits(expr: &Expr) -> bool {
    match expr {
        Expr::Await { .. } => true,
        Expr::Lit { .. }
        | Expr::Text { .. }
        | Expr::None
        | Expr::Local { .. }
        | Expr::SelfField { .. }
        | Expr::Enum { .. } => false,
        Expr::Format { parts } | Expr::Array { items: parts } => parts.iter().any(awaits),
        Expr::Field { of, .. } | Expr::Tag { value: of } | Expr::Unary { value: of, .. } => {
            awaits(of)
        }
        Expr::Cast { value, .. } | Expr::Required { value } => awaits(value),
        Expr::Index { of, at } => awaits(of) || awaits(at),
        Expr::Binary { left, right, .. } => awaits(left) || awaits(right),
        Expr::Record { fields, .. } | Expr::Variant { fields, .. } => fields.values().any(awaits),
        Expr::Call { args, .. } | Expr::Method { args, .. } | Expr::Intrinsic { args, .. } => {
            args.iter().any(awaits)
        }
        Expr::Ternary {
            cond,
            then,
            otherwise,
        } => awaits(cond) || awaits(then) || awaits(otherwise),
        Expr::IfSome {
            value, some, none, ..
        } => awaits(value) || awaits(some) || awaits(none),
        Expr::IfVariant {
            value,
            then,
            otherwise,
            ..
        } => awaits(value) || awaits(then) || awaits(otherwise),
    }
}
