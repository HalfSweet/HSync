// SPDX-License-Identifier: Apache-2.0

//! The types the emitter reasons with, their Rust spelling, and the declarations of records,
//! enums and unions.

use std::collections::BTreeMap;
use std::fmt::Write;

use uiharu_ir::{Field, FieldJson, Type, TypeDef, TypeKind};

use crate::names::{plain_literal, snake, variant_name};

/// A type as the emitter sees it: the IR's, plus what the runtime's operations hand out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Ty {
    Bool,
    U8,
    U16,
    U32,
    U64,
    Text,
    Mask,
    Levels,
    Range,
    Provenance,
    Cursor,
    Option(Box<Ty>),
    Array(Box<Ty>, u32),
    FixedArray(Box<Ty>, u32),
    Batch(Box<Ty>, u32),
    /// A declared record, enum or union.
    Named(String),
    /// What `next-change` returns.
    Edge,
    /// What `next-change-before` returns.
    Timed,
    /// What `next` returns: the lower event with its provenance.
    Received(String),
    /// What `batch-take` returns.
    Taken(Box<Ty>),
    /// An integer literal, of whatever width its context has.
    IntLit,
    Unit,
}

impl Ty {
    pub fn from_ir(ty: &Type) -> Ty {
        match ty {
            Type::Name(name) => match name.as_str() {
                "bool" => Ty::Bool,
                "u8" => Ty::U8,
                "u16" => Ty::U16,
                "u32" => Ty::U32,
                "u64" => Ty::U64,
                "text" => Ty::Text,
                "mask" => Ty::Mask,
                "levels" => Ty::Levels,
                "range" => Ty::Range,
                "provenance" => Ty::Provenance,
                "cursor" => Ty::Cursor,
                other => Ty::Named(other.to_owned()),
            },
            Type::Option { option } => Ty::Option(Box::new(Ty::from_ir(option))),
            Type::Array { array, bound } => Ty::Array(Box::new(Ty::from_ir(array)), *bound),
            Type::FixedArray {
                fixed_array,
                length,
            } => Ty::FixedArray(Box::new(Ty::from_ir(fixed_array)), *length),
            Type::Batch { batch, bound } => Ty::Batch(Box::new(Ty::from_ir(batch)), *bound),
        }
    }

    /// The width of an integer type.
    pub fn width(&self) -> Option<u8> {
        match self {
            Ty::U8 => Some(8),
            Ty::U16 => Some(16),
            Ty::U32 => Some(32),
            Ty::U64 => Some(64),
            _ => None,
        }
    }

    pub fn of_width(width: u8) -> Ty {
        match width {
            8 => Ty::U8,
            16 => Ty::U16,
            32 => Ty::U32,
            _ => Ty::U64,
        }
    }

    pub fn is_int(&self) -> bool {
        self.width().is_some() || *self == Ty::IntLit
    }

    /// The Rust spelling; `lower` is the lower event type for cursors.
    pub fn rust(&self, lower: Option<&str>) -> String {
        match self {
            Ty::Bool => "bool".into(),
            Ty::U8 | Ty::IntLit => "u8".into(),
            Ty::U16 => "u16".into(),
            Ty::U32 => "u32".into(),
            Ty::U64 => "u64".into(),
            Ty::Text => "String".into(),
            Ty::Mask => "ChannelMask".into(),
            Ty::Levels => "Levels".into(),
            Ty::Range => "TickRange".into(),
            Ty::Provenance => "Provenance".into(),
            Ty::Cursor => format!("Transaction<'_, {}>", lower.unwrap_or("()")),
            Ty::Option(inner) => format!("Option<{}>", inner.rust(lower)),
            Ty::Array(item, _) => format!("Vec<{}>", item.rust(lower)),
            Ty::FixedArray(item, length) => format!("[{}; {length}]", item.rust(lower)),
            Ty::Batch(item, bound) => format!("Batch<{}, {bound}>", item.rust(lower)),
            Ty::Named(name) => name.clone(),
            Ty::Edge => "Edge".into(),
            Ty::Timed => "Timed".into(),
            Ty::Received(event) => format!("Received<{event}>"),
            Ty::Taken(item) => format!("Taken<{}>", item.rust(lower)),
            Ty::Unit => "()".into(),
        }
    }
}

/// The declared types, by name.
pub struct Defs<'a> {
    pub defs: BTreeMap<String, &'a TypeDef>,
}

impl<'a> Defs<'a> {
    pub fn new(defs: impl IntoIterator<Item = &'a TypeDef>) -> Self {
        Self {
            defs: defs
                .into_iter()
                .map(|def| (def.name.clone(), def))
                .collect(),
        }
    }

    pub fn get(&self, name: &str) -> Option<&'a TypeDef> {
        self.defs.get(name).copied()
    }

    /// The legal, collision-free Rust spelling of a wire enum/union value.
    pub fn variant_name(&self, ty: &str, name: &str) -> String {
        let names: Vec<&str> = match self.get(ty).map(|def| &def.kind) {
            Some(TypeKind::Enum { variants }) => variants.iter().map(|v| v.0.as_str()).collect(),
            Some(TypeKind::NumericEnum { variants }) => {
                variants.iter().map(|v| v.name.as_str()).collect()
            }
            Some(TypeKind::ExternalEnum { variants }) => {
                variants.iter().map(|v| v.name.as_str()).collect()
            }
            Some(TypeKind::Union { variants }) => {
                variants.iter().map(|v| v.name.as_str()).collect()
            }
            _ => Vec::new(),
        };
        variant_name(name, names)
    }

    pub fn record_fields(&self, name: &str) -> Option<&'a [Field]> {
        match &self.get(name)?.kind {
            TypeKind::Record { fields } => Some(fields),
            _ => None,
        }
    }

    pub fn is_enum(&self, name: &str) -> bool {
        matches!(
            self.get(name),
            Some(TypeDef {
                kind: TypeKind::Enum { .. },
                ..
            })
        )
    }

    pub fn is_numeric_enum(&self, name: &str) -> bool {
        matches!(
            self.get(name).map(|def| &def.kind),
            Some(TypeKind::NumericEnum { .. })
        )
    }

    pub fn is_external_enum(&self, name: &str) -> bool {
        matches!(
            self.get(name).map(|def| &def.kind),
            Some(TypeKind::ExternalEnum { .. })
        )
    }

    pub fn variant_fields(&self, union: &str, variant: &str) -> Option<Vec<Field>> {
        match &self.get(union)?.kind {
            TypeKind::Union { variants } => variants
                .iter()
                .find(|v| v.name == variant)
                .map(|v| v.fields.clone()),
            TypeKind::ExternalEnum { variants } => {
                variants.iter().find(|v| v.name == variant).map(|v| {
                    v.payload
                        .iter()
                        .map(|ty| Field {
                            name: "value".into(),
                            ty: ty.clone(),
                            channel: false,
                            json: None,
                        })
                        .collect()
                })
            }
            _ => None,
        }
    }

    /// Whether values of the type are data serde can carry: not the runtime's own types.
    pub fn is_data(&self, ty: &Ty) -> bool {
        match ty {
            Ty::Bool | Ty::U8 | Ty::U16 | Ty::U32 | Ty::U64 | Ty::IntLit | Ty::Text | Ty::Unit => {
                true
            }
            Ty::Option(inner)
            | Ty::Array(inner, _)
            | Ty::FixedArray(inner, _)
            | Ty::Taken(inner) => self.is_data(inner),
            Ty::Mask
            | Ty::Levels
            | Ty::Range
            | Ty::Provenance
            | Ty::Cursor
            | Ty::Batch(..)
            | Ty::Edge
            | Ty::Timed
            | Ty::Received(_) => false,
            Ty::Named(name) => match self.get(name).map(|def| &def.kind) {
                Some(TypeKind::Enum { .. } | TypeKind::NumericEnum { .. }) => true,
                Some(TypeKind::ExternalEnum { variants }) => variants.iter().all(|variant| {
                    variant
                        .payload
                        .as_ref()
                        .is_none_or(|ty| self.is_data(&Ty::from_ir(ty)))
                }),
                Some(TypeKind::Record { fields }) => {
                    fields.iter().all(|f| self.is_data(&Ty::from_ir(&f.ty)))
                }
                Some(TypeKind::Union { variants }) => variants
                    .iter()
                    .all(|v| v.fields.iter().all(|f| self.is_data(&Ty::from_ir(&f.ty)))),
                None => false,
            },
        }
    }

    /// Whether values of the type are `Copy` in Rust.
    pub fn is_copy(&self, ty: &Ty) -> bool {
        match ty {
            Ty::Bool
            | Ty::U8
            | Ty::U16
            | Ty::U32
            | Ty::U64
            | Ty::IntLit
            | Ty::Mask
            | Ty::Levels
            | Ty::Range
            | Ty::Provenance
            | Ty::Edge
            | Ty::Timed
            | Ty::Unit => true,
            Ty::Option(inner) | Ty::FixedArray(inner, _) => self.is_copy(inner),
            Ty::Text
            | Ty::Cursor
            | Ty::Array(..)
            | Ty::Batch(..)
            | Ty::Received(_)
            | Ty::Taken(_) => false,
            Ty::Named(name) => match self.get(name).map(|def| &def.kind) {
                Some(TypeKind::Enum { .. } | TypeKind::NumericEnum { .. }) => true,
                Some(TypeKind::ExternalEnum { variants }) => variants.iter().all(|variant| {
                    variant
                        .payload
                        .as_ref()
                        .is_none_or(|ty| self.is_copy(&Ty::from_ir(ty)))
                }),
                Some(TypeKind::Record { fields }) => {
                    fields.iter().all(|f| self.is_copy(&Ty::from_ir(&f.ty)))
                }
                Some(TypeKind::Union { variants }) => variants
                    .iter()
                    .all(|v| v.fields.iter().all(|f| self.is_copy(&Ty::from_ir(&f.ty)))),
                None => false,
            },
        }
    }
}

/// The item type of a framed contract, as the cursor hands it out: the fields of the one item
/// kind without the frame's id, or a union of the item kinds without it.
pub fn item_def(
    event: &str,
    variants: &[uiharu_ir::UnionVariant],
    framing: &uiharu_ir::Framing,
) -> Result<TypeDef, String> {
    let name = format!("{event}Item");
    let mut items = Vec::new();
    for kind in &framing.items {
        let variant = variants
            .iter()
            .find(|variant| &variant.name == kind)
            .ok_or_else(|| format!("{event} has no variant {kind}"))?;
        items.push(uiharu_ir::UnionVariant {
            name: kind.clone(),
            fields: variant
                .fields
                .iter()
                .filter(|field| field.name != framing.id)
                .cloned()
                .collect(),
        });
    }
    let kind = match items.as_slice() {
        [single] => TypeKind::Record {
            fields: single.fields.clone(),
        },
        _ => TypeKind::Union { variants: items },
    };
    Ok(TypeDef { name, kind })
}

/// The declarations of a module's types. `config` names the record that is the configuration,
/// which gets its serde attributes; `lower` the lower event type, for cursors.
pub fn emit_types(defs: &Defs<'_>, types: &[TypeDef], config: &str, lower: Option<&str>) -> String {
    let mut out = String::new();
    for def in types {
        match &def.kind {
            TypeKind::Record { fields } => {
                let copy = fields.iter().all(|f| defs.is_copy(&Ty::from_ir(&f.ty)));
                let data = fields.iter().all(|f| defs.is_data(&Ty::from_ir(&f.ty)));
                let mut derives = vec!["Clone"];
                if copy {
                    derives.push("Copy");
                }
                derives.extend(["Debug", "PartialEq", "Eq"]);
                if data {
                    derives.extend(["Serialize", "Deserialize"]);
                }
                let _ = writeln!(out, "#[derive({})]", derives.join(", "));
                if def.name == config {
                    out.push_str("#[serde(default, deny_unknown_fields)]\n");
                }
                let _ = writeln!(out, "pub struct {} {{", def.name);
                for field in fields {
                    if field.json == Some(FieldJson::HexU64) {
                        out.push_str("    #[serde(with = \"uiharu_core::hex_u64\")]\n");
                    }
                    let _ = writeln!(
                        out,
                        "    pub {}: {},",
                        snake(&field.name),
                        Ty::from_ir(&field.ty).rust(lower)
                    );
                }
                out.push_str("}\n\n");
            }
            TypeKind::Enum { variants } => {
                out.push_str(
                    "#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]\n",
                );
                let _ = writeln!(out, "pub enum {} {{", def.name);
                for variant in variants {
                    let _ = writeln!(out, "    #[serde(rename = {})]", plain_literal(&variant.0));
                    let _ = writeln!(out, "    {},", defs.variant_name(&def.name, &variant.0));
                }
                out.push_str("}\n\n");
                let _ = writeln!(out, "impl {} {{", def.name);
                out.push_str("    /// The name the contract spells the value with.\n");
                out.push_str(
                    "    pub const fn name(self) -> &'static str {\n        match self {\n",
                );
                for variant in variants {
                    let _ = writeln!(
                        out,
                        "            {}::{} => {},",
                        def.name,
                        defs.variant_name(&def.name, &variant.0),
                        plain_literal(&variant.0)
                    );
                }
                out.push_str("        }\n    }\n}\n\n");
            }
            TypeKind::Union { variants } => {
                let copy = variants
                    .iter()
                    .all(|v| v.fields.iter().all(|f| defs.is_copy(&Ty::from_ir(&f.ty))));
                let derives = if copy {
                    "Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize"
                } else {
                    "Clone, Debug, PartialEq, Eq, Serialize, Deserialize"
                };
                let _ = writeln!(out, "#[derive({derives})]");
                out.push_str("#[serde(tag = \"kind\", rename_all_fields = \"camelCase\")]\n");
                let _ = writeln!(out, "pub enum {} {{", def.name);
                for variant in variants {
                    let _ = writeln!(
                        out,
                        "    #[serde(rename = {})]",
                        plain_literal(&variant.name)
                    );
                    if variant.fields.is_empty() {
                        let _ =
                            writeln!(out, "    {},", defs.variant_name(&def.name, &variant.name));
                    } else {
                        let _ = writeln!(
                            out,
                            "    {} {{",
                            defs.variant_name(&def.name, &variant.name)
                        );
                        for field in &variant.fields {
                            if field.json == Some(FieldJson::HexU64) {
                                out.push_str("        #[serde(with = \"uiharu_core::hex_u64\")]\n");
                            }
                            let _ = writeln!(
                                out,
                                "        {}: {},",
                                snake(&field.name),
                                Ty::from_ir(&field.ty).rust(lower)
                            );
                        }
                        out.push_str("    },\n");
                    }
                }
                out.push_str("}\n\n");
            }
            TypeKind::NumericEnum { variants } => {
                out.push_str("#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]\n");
                let _ = writeln!(out, "pub enum {} {{", def.name);
                for variant in variants {
                    let _ = writeln!(out, "    {},", defs.variant_name(&def.name, &variant.name));
                }
                out.push_str("}\n\n");
                let _ = writeln!(out, "impl {} {{", def.name);
                out.push_str("    /// The number the contract spells the value with.\n    pub const fn value(self) -> u64 {\n        match self {\n");
                for variant in variants {
                    let _ = writeln!(
                        out,
                        "            Self::{} => {},",
                        defs.variant_name(&def.name, &variant.name),
                        variant.value
                    );
                }
                out.push_str("        }\n    }\n}\n\n");
                let _ = writeln!(
                    out,
                    "impl Serialize for {} {{\n    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {{\n        serializer.serialize_u64(self.value())\n    }}\n}}\n",
                    def.name
                );
                let _ = writeln!(
                    out,
                    "impl<'de> Deserialize<'de> for {} {{\n    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {{\n        match u64::deserialize(deserializer)? {{",
                    def.name
                );
                for variant in variants {
                    let _ = writeln!(
                        out,
                        "            {} => Ok(Self::{}),",
                        variant.value,
                        defs.variant_name(&def.name, &variant.name)
                    );
                }
                out.push_str("            value => Err(serde::de::Error::custom(format!(\"unknown numeric enum value {value}\"))),\n        }\n    }\n}\n\n");
            }
            TypeKind::ExternalEnum { variants } => {
                let copy = variants.iter().all(|variant| {
                    variant
                        .payload
                        .as_ref()
                        .is_none_or(|ty| defs.is_copy(&Ty::from_ir(ty)))
                });
                let data = variants.iter().all(|variant| {
                    variant
                        .payload
                        .as_ref()
                        .is_none_or(|ty| defs.is_data(&Ty::from_ir(ty)))
                });
                let mut derives = vec!["Clone"];
                if copy {
                    derives.push("Copy");
                }
                derives.extend(["Debug", "PartialEq", "Eq"]);
                if data {
                    derives.extend(["Serialize", "Deserialize"]);
                }
                let _ = writeln!(
                    out,
                    "#[derive({})]\npub enum {} {{",
                    derives.join(", "),
                    def.name
                );
                for variant in variants {
                    if data {
                        let _ = writeln!(
                            out,
                            "    #[serde(rename = {})]",
                            plain_literal(&variant.name)
                        );
                    }
                    let name = defs.variant_name(&def.name, &variant.name);
                    match &variant.payload {
                        Some(ty) => {
                            let _ = writeln!(out, "    {name}({}),", Ty::from_ir(ty).rust(lower));
                        }
                        None => {
                            let _ = writeln!(out, "    {name},");
                        }
                    }
                }
                out.push_str("}\n\n");
            }
        }
    }
    out
}
