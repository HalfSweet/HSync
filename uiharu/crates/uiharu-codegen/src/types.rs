// SPDX-License-Identifier: Apache-2.0

//! The types the emitter reasons with, their Rust spelling, and the declarations of records,
//! enums and unions.

use std::collections::BTreeMap;
use std::fmt::Write;

use uiharu_ir::{Field, Type, TypeDef, TypeKind};

use crate::names::{pascal, plain_literal, snake};

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
    /// What a frame cursor's `item` returns.
    Item,
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
            Ty::Batch(item, bound) => format!("Batch<{}, {bound}>", item.rust(lower)),
            Ty::Named(name) => name.clone(),
            Ty::Edge => "Edge".into(),
            Ty::Timed => "Timed".into(),
            Ty::Received(event) => format!("Received<{event}>"),
            Ty::Taken(item) => format!("Taken<{}>", item.rust(lower)),
            Ty::Item => format!("{}Item", lower.unwrap_or("")),
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

    pub fn variant_fields(&self, union: &str, variant: &str) -> Option<&'a [Field]> {
        match &self.get(union)?.kind {
            TypeKind::Union { variants } => variants
                .iter()
                .find(|v| v.name == variant)
                .map(|v| v.fields.as_slice()),
            _ => None,
        }
    }

    /// Whether values of the type are data serde can carry: not the runtime's own types.
    pub fn is_data(&self, ty: &Ty) -> bool {
        match ty {
            Ty::Bool | Ty::U8 | Ty::U16 | Ty::U32 | Ty::U64 | Ty::IntLit | Ty::Text | Ty::Unit => {
                true
            }
            Ty::Option(inner) | Ty::Array(inner, _) | Ty::Taken(inner) => self.is_data(inner),
            Ty::Mask
            | Ty::Levels
            | Ty::Range
            | Ty::Provenance
            | Ty::Cursor
            | Ty::Batch(..)
            | Ty::Edge
            | Ty::Timed
            | Ty::Received(_)
            | Ty::Item => false,
            Ty::Named(name) => match self.get(name).map(|def| &def.kind) {
                Some(TypeKind::Enum { .. }) => true,
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
            Ty::Option(inner) => self.is_copy(inner),
            Ty::Text
            | Ty::Cursor
            | Ty::Array(..)
            | Ty::Batch(..)
            | Ty::Received(_)
            | Ty::Taken(_)
            | Ty::Item => false,
            Ty::Named(name) => match self.get(name).map(|def| &def.kind) {
                Some(TypeKind::Enum { .. }) => true,
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
                    let _ = writeln!(out, "    {},", pascal(&variant.0));
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
                        pascal(&variant.0),
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
                        let _ = writeln!(out, "    {},", pascal(&variant.name));
                    } else {
                        let _ = writeln!(out, "    {} {{", pascal(&variant.name));
                        for field in &variant.fields {
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
        }
    }
    out
}
