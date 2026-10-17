// SPDX-License-Identifier: Apache-2.0

//! The types the compiler reasons with — the IR's, plus what the runtime's operations hand
//! out — and the declared records, enums and unions of a module, by name and by index.

use std::collections::HashMap;

use uiharu_ir::{Framing, Module, Type, TypeDef, TypeKind, UnionVariant};

use crate::value::Width;

/// A type as the compiler sees it.
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
    /// What `next-change` returns: a record of `tick`, `changed` and `levels`.
    Edge,
    /// What `next-change-before` returns: a union of `edge` and `quiet`.
    Timed,
    /// What `next` returns: a record of the lower `event` and where it came from (`at`).
    Received(String),
    /// What `batch-take` returns: a record of the `items` and their provenance (`at`).
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
            Type::Batch { batch, bound } => Ty::Batch(Box::new(Ty::from_ir(batch)), *bound),
        }
    }

    /// The width of an integer type.
    pub fn width(&self) -> Option<Width> {
        match self {
            Ty::U8 => Some(Width::W8),
            Ty::U16 => Some(Width::W16),
            Ty::U32 => Some(Width::W32),
            Ty::U64 => Some(Width::W64),
            _ => None,
        }
    }

    pub fn of_width(width: Width) -> Ty {
        match width {
            Width::W8 => Ty::U8,
            Width::W16 => Ty::U16,
            Width::W32 => Ty::U32,
            Width::W64 => Ty::U64,
        }
    }

    pub fn is_int(&self) -> bool {
        self.width().is_some() || *self == Ty::IntLit
    }

    /// The type as diagnostics name it.
    pub fn describe(&self) -> String {
        match self {
            Ty::Bool => "bool".into(),
            Ty::U8 => "u8".into(),
            Ty::U16 => "u16".into(),
            Ty::U32 => "u32".into(),
            Ty::U64 => "u64".into(),
            Ty::Text => "text".into(),
            Ty::Mask => "mask".into(),
            Ty::Levels => "levels".into(),
            Ty::Range => "range".into(),
            Ty::Provenance => "provenance".into(),
            Ty::Cursor => "cursor".into(),
            Ty::Option(inner) => format!("option<{}>", inner.describe()),
            Ty::Array(item, bound) => format!("array<{}, {bound}>", item.describe()),
            Ty::Batch(item, bound) => format!("batch<{}, {bound}>", item.describe()),
            Ty::Named(name) => name.clone(),
            Ty::Edge => "edge".into(),
            Ty::Timed => "edge or quiet".into(),
            Ty::Received(event) => format!("received<{event}>"),
            Ty::Taken(item) => format!("taken<{}>", item.describe()),
            Ty::IntLit => "an integer literal".into(),
            Ty::Unit => "nothing".into(),
        }
    }
}

/// A field of a record or a union variant.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FieldDef {
    pub name: String,
    pub ty: Ty,
    /// Whether a configuration field is a channel of the source.
    pub channel: bool,
}

/// A record: fields in declaration order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecordDef {
    pub name: String,
    pub fields: Vec<FieldDef>,
}

/// An enum: its variants' names, as events spell them, in declaration order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EnumDef {
    pub name: String,
    pub variants: Vec<String>,
}

/// A variant of a union.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VariantDef {
    pub name: String,
    pub fields: Vec<FieldDef>,
}

/// A union: variants in declaration order, tagged by `kind`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnionDef {
    pub name: String,
    pub variants: Vec<VariantDef>,
}

/// A declared type, by its index in the table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TypeRef {
    Record(usize),
    Enum(usize),
    Union(usize),
}

/// The declared types of a module: its own, the ones its imports carry, and the item type of
/// every framed contract in scope (`<Event>Item`: the fields of the one kind of item without
/// the frame's id, or the union of several kinds).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TypeTable {
    pub records: Vec<RecordDef>,
    pub enums: Vec<EnumDef>,
    pub unions: Vec<UnionDef>,
    by_name: HashMap<String, TypeRef>,
}

fn fields_of(fields: &[uiharu_ir::Field]) -> Vec<FieldDef> {
    fields
        .iter()
        .map(|field| FieldDef {
            name: field.name.clone(),
            ty: Ty::from_ir(&field.ty),
            channel: field.channel,
        })
        .collect()
}

/// The name of the item type of a framed contract whose events are `event`.
pub fn item_type_name(event: &str) -> String {
    format!("{event}Item")
}

/// The item type of a framed contract, as the cursor hands it out.
fn item_def(event: &str, variants: &[UnionVariant], framing: &Framing) -> Result<TypeDef, String> {
    let mut items = Vec::new();
    for kind in &framing.items {
        let variant = variants
            .iter()
            .find(|variant| &variant.name == kind)
            .ok_or_else(|| format!("{event} has no variant {kind}"))?;
        items.push(UnionVariant {
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
    Ok(TypeDef {
        name: item_type_name(event),
        kind,
    })
}

impl TypeTable {
    /// The types in scope in `module`.
    pub fn from_module(module: &Module) -> Result<Self, String> {
        let declared: Vec<&TypeDef> = module
            .types
            .iter()
            .chain(module.imports.iter().flat_map(|import| import.types.iter()))
            .collect();
        let mut items = Vec::new();
        let contracts = module
            .contracts
            .iter()
            .chain(module.imports.iter().map(|import| &import.contract));
        for contract in contracts {
            let Some(framing) = &contract.framing else {
                continue;
            };
            let variants = declared
                .iter()
                .find(|def| def.name == contract.event)
                .and_then(|def| match &def.kind {
                    TypeKind::Union { variants } => Some(variants),
                    _ => None,
                })
                .ok_or_else(|| format!("{} is not a union", contract.event))?;
            items.push(item_def(&contract.event, variants, framing)?);
        }
        let mut table = Self::default();
        for def in declared.into_iter().chain(items.iter()) {
            table.add(def)?;
        }
        Ok(table)
    }

    fn add(&mut self, def: &TypeDef) -> Result<(), String> {
        let reference = match &def.kind {
            TypeKind::Record { fields } => {
                self.records.push(RecordDef {
                    name: def.name.clone(),
                    fields: fields_of(fields),
                });
                TypeRef::Record(self.records.len() - 1)
            }
            TypeKind::Enum { variants } => {
                self.enums.push(EnumDef {
                    name: def.name.clone(),
                    variants: variants.iter().map(|variant| variant.0.clone()).collect(),
                });
                TypeRef::Enum(self.enums.len() - 1)
            }
            TypeKind::Union { variants } => {
                self.unions.push(UnionDef {
                    name: def.name.clone(),
                    variants: variants
                        .iter()
                        .map(|variant| VariantDef {
                            name: variant.name.clone(),
                            fields: fields_of(&variant.fields),
                        })
                        .collect(),
                });
                TypeRef::Union(self.unions.len() - 1)
            }
        };
        if self.by_name.insert(def.name.clone(), reference).is_some() {
            return Err(format!("{} is declared twice", def.name));
        }
        Ok(())
    }

    /// The type named `name`.
    pub fn get(&self, name: &str) -> Option<TypeRef> {
        self.by_name.get(name).copied()
    }

    /// The record named `name`, with its index.
    pub fn record(&self, name: &str) -> Option<(usize, &RecordDef)> {
        match self.get(name)? {
            TypeRef::Record(index) => Some((index, &self.records[index])),
            _ => None,
        }
    }

    /// The enum named `name`, with its index.
    pub fn enumeration(&self, name: &str) -> Option<(usize, &EnumDef)> {
        match self.get(name)? {
            TypeRef::Enum(index) => Some((index, &self.enums[index])),
            _ => None,
        }
    }

    /// The union named `name`, with its index.
    pub fn union(&self, name: &str) -> Option<(usize, &UnionDef)> {
        match self.get(name)? {
            TypeRef::Union(index) => Some((index, &self.unions[index])),
            _ => None,
        }
    }

    /// The index of `variant` in the enum `name`.
    pub fn enum_variant(&self, name: &str, variant: &str) -> Option<u16> {
        let (_, def) = self.enumeration(name)?;
        let index = def.variants.iter().position(|have| have == variant)?;
        u16::try_from(index).ok()
    }

    /// The variant `variant` of the union `name`, with its tag.
    pub fn variant(&self, name: &str, variant: &str) -> Option<(u16, &VariantDef)> {
        let (_, def) = self.union(name)?;
        let index = def.variants.iter().position(|have| have.name == variant)?;
        Some((u16::try_from(index).ok()?, &def.variants[index]))
    }

    /// Whether values of the type are data a contract can carry: not the runtime's own types.
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
            | Ty::Received(_) => false,
            Ty::Named(name) => match self.get(name) {
                Some(TypeRef::Enum(_)) => true,
                Some(TypeRef::Record(index)) => self.records[index]
                    .fields
                    .iter()
                    .all(|field| self.is_data(&field.ty)),
                Some(TypeRef::Union(index)) => self.unions[index]
                    .variants
                    .iter()
                    .all(|variant| variant.fields.iter().all(|field| self.is_data(&field.ty))),
                None => false,
            },
        }
    }
}

/// The fields of the runtime's own record types, as `field` reads them.
pub fn builtin_field(ty: &Ty, name: &str) -> Option<(u32, Ty)> {
    match (ty, name) {
        (Ty::Edge, "tick") => Some((0, Ty::U64)),
        (Ty::Edge, "changed") => Some((1, Ty::Mask)),
        (Ty::Edge, "levels") => Some((2, Ty::Levels)),
        (Ty::Received(event), "event") => Some((0, Ty::Named(event.clone()))),
        (Ty::Received(_), "at") => Some((1, Ty::Provenance)),
        (Ty::Taken(item), "items") => Some((0, Ty::Array(item.clone(), u32::MAX))),
        (Ty::Taken(_), "at") => Some((1, Ty::Option(Box::new(Ty::Provenance)))),
        (Ty::Provenance, "span") => Some((0, Ty::Range)),
        (Ty::Provenance, "settled") => Some((1, Ty::U64)),
        (Ty::Range, "start") => Some((0, Ty::U64)),
        (Ty::Range, "end") => Some((1, Ty::U64)),
        _ => None,
    }
}

/// The variants of the runtime's own union type, what `next-change-before` returns: `edge`
/// with `tick`, `changed` and `levels`, and `quiet` with `tick`.
pub fn timed_variant(name: &str) -> Option<(u16, Vec<FieldDef>)> {
    let field = |name: &str, ty: Ty| FieldDef {
        name: name.to_owned(),
        ty,
        channel: false,
    };
    match name {
        "edge" => Some((
            0,
            vec![
                field("tick", Ty::U64),
                field("changed", Ty::Mask),
                field("levels", Ty::Levels),
            ],
        )),
        "quiet" => Some((1, vec![field("tick", Ty::U64)])),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn types_come_from_the_ir_with_their_widths() {
        assert_eq!(Ty::from_ir(&Type::named("u16")).width(), Some(Width::W16));
        assert_eq!(
            Ty::from_ir(&Type::option(Type::named("Found"))),
            Ty::Option(Box::new(Ty::Named("Found".into())))
        );
        assert!(Ty::IntLit.is_int());
        assert!(!Ty::Text.is_int());
        assert_eq!(Ty::of_width(Width::W32), Ty::U32);
        assert_eq!(builtin_field(&Ty::Edge, "levels"), Some((2, Ty::Levels)));
        assert_eq!(builtin_field(&Ty::Edge, "kind"), None);
        assert_eq!(timed_variant("quiet").map(|(tag, _)| tag), Some(1));
    }
}
