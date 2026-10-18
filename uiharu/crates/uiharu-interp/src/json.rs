// SPDX-License-Identifier: Apache-2.0

//! Values as JSON, the way the contracts' Rust types serialize them: a union value as an object
//! with its `kind` and its fields by name, a record as an object with its fields in snake
//! case (as a struct without `rename_all` is written), an enum as its name, a missing value as
//! `null`. The configuration is read the same way, with the module's defaults for what is
//! left out.

use serde_json::{Map, Value as Json};
use uiharu_core::PropValue;

use crate::program::Program;
use crate::types::{FieldDef, Ty, TypeRef, TypeTable};
use crate::value::{Value, Width};

/// `camelCase` to `snake_case`: the name serde gives a record's field, under which a
/// configuration's fields are read and written as JSON.
pub fn snake(name: &str) -> String {
    let mut out = String::with_capacity(name.len() + 4);
    let chars: Vec<char> = name.chars().collect();
    for (index, &c) in chars.iter().enumerate() {
        if c == '-' || c == ' ' {
            out.push('_');
        } else if c.is_ascii_uppercase() {
            let prev_lower = index > 0 && chars[index - 1].is_ascii_lowercase();
            let prev_digit = index > 0 && chars[index - 1].is_ascii_digit();
            let next_lower = chars.get(index + 1).is_some_and(|n| n.is_ascii_lowercase());
            let prev_upper = index > 0 && chars[index - 1].is_ascii_uppercase();
            if index > 0 && (prev_lower || prev_digit || (prev_upper && next_lower)) {
                out.push('_');
            }
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

/// How a record's fields are named in JSON: as serde names a struct's (snake case), or as the
/// contract spells a union variant's (the IR's names).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Keys {
    Struct,
    Variant,
}

fn key(field: &FieldDef, keys: Keys) -> String {
    match keys {
        Keys::Struct => snake(&field.name),
        Keys::Variant => field.name.clone(),
    }
}

/// `names` as serde lists what it expected: `a`, `a` or `b`, one of `a`, `b`, `c`.
pub(crate) fn expected_list(names: &[String], what: &str) -> String {
    match names {
        [] => format!("there are no {what}"),
        [one] => format!("`{one}`"),
        [a, b] => format!("`{a}` or `{b}`"),
        many => format!(
            "one of {}",
            many.iter()
                .map(|name| format!("`{name}`"))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

fn describe_json(json: &Json) -> String {
    match json {
        Json::Null => "null".into(),
        Json::Bool(b) => format!("boolean `{b}`"),
        Json::Number(n) => {
            if n.is_f64() {
                format!("floating point `{n}`")
            } else {
                format!("integer `{n}`")
            }
        }
        Json::String(s) => format!("string {s:?}"),
        Json::Array(_) => "sequence".into(),
        Json::Object(_) => "map".into(),
    }
}

fn invalid_type(json: &Json, expected: &str) -> String {
    format!("invalid type: {}, expected {expected}", describe_json(json))
}

/// `value` of type `ty` as JSON.
pub(crate) fn to_json(types: &TypeTable, ty: &Ty, value: &Value) -> Json {
    match (ty, value) {
        (_, Value::None) => Json::Null,
        (Ty::Option(inner), value) => to_json(types, inner, value),
        (Ty::Bool, Value::Bool(b)) => Json::Bool(*b),
        (ty, Value::Int(n)) if ty.is_int() => Json::from(*n),
        (Ty::Text, Value::Text(text)) => Json::String(text.to_string()),
        (Ty::Array(item, _), Value::Array(items)) => {
            Json::Array(items.iter().map(|v| to_json(types, item, v)).collect())
        }
        (Ty::Named(name), value) => match (types.get(name), value) {
            (Some(TypeRef::Enum(index)), Value::Enum(variant)) => Json::String(
                types.enums[index]
                    .variants
                    .get(usize::from(*variant))
                    .cloned()
                    .unwrap_or_default(),
            ),
            (Some(TypeRef::Record(index)), Value::Record(fields)) => {
                let mut object = Map::new();
                for (field, value) in types.records[index].fields.iter().zip(fields.iter()) {
                    object.insert(key(field, Keys::Struct), to_json(types, &field.ty, value));
                }
                Json::Object(object)
            }
            (Some(TypeRef::Union(index)), Value::Variant(tag, fields)) => {
                let Some(variant) = types.unions[index].variants.get(usize::from(*tag)) else {
                    return Json::Null;
                };
                let mut object = Map::new();
                object.insert("kind".into(), Json::String(variant.name.clone()));
                for (field, value) in variant.fields.iter().zip(fields.iter()) {
                    object.insert(key(field, Keys::Variant), to_json(types, &field.ty, value));
                }
                Json::Object(object)
            }
            _ => Json::Null,
        },
        _ => Json::Null,
    }
}

/// The fields of a record or variant from `object`, by name; a missing option is the missing
/// value, any other missing field an error.
pub(crate) fn fields_from_object(
    types: &TypeTable,
    fields: &[FieldDef],
    object: &Map<String, Json>,
    keys: Keys,
) -> Result<Vec<Value>, String> {
    let mut values = Vec::with_capacity(fields.len());
    for field in fields {
        let name = key(field, keys);
        match object.get(&name) {
            Some(json) => values.push(from_json(types, &field.ty, json)?),
            None if matches!(field.ty, Ty::Option(_)) => values.push(Value::None),
            None => return Err(format!("missing field `{name}`")),
        }
    }
    Ok(values)
}

/// A value of type `ty` read from JSON.
pub(crate) fn from_json(types: &TypeTable, ty: &Ty, json: &Json) -> Result<Value, String> {
    match ty {
        Ty::Option(inner) => {
            if json.is_null() {
                Ok(Value::None)
            } else {
                from_json(types, inner, json)
            }
        }
        Ty::Bool => json
            .as_bool()
            .map(Value::Bool)
            .ok_or_else(|| invalid_type(json, "a boolean")),
        ty if ty.is_int() => {
            let width = ty.width().unwrap_or(Width::W64);
            let n = json
                .as_u64()
                .ok_or_else(|| invalid_type(json, width.name()))?;
            if !width.fits(n) {
                return Err(format!(
                    "invalid value: integer `{n}`, expected {}",
                    width.name()
                ));
            }
            Ok(Value::Int(n))
        }
        Ty::Text => json
            .as_str()
            .map(|text| Value::Text(text.into()))
            .ok_or_else(|| invalid_type(json, "a string")),
        Ty::Array(item, _) => {
            let items = json
                .as_array()
                .ok_or_else(|| invalid_type(json, "a sequence"))?;
            let values = items
                .iter()
                .map(|json| from_json(types, item, json))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(Value::Array(std::rc::Rc::new(values)))
        }
        Ty::Named(name) => match types.get(name) {
            Some(TypeRef::Enum(index)) => {
                let text = json
                    .as_str()
                    .ok_or_else(|| invalid_type(json, "a string"))?;
                let variants = &types.enums[index].variants;
                let variant = variants
                    .iter()
                    .position(|variant| variant == text)
                    .ok_or_else(|| {
                        format!(
                            "unknown variant `{text}`, expected {}",
                            expected_list(variants, "variants")
                        )
                    })?;
                Ok(Value::Enum(u16::try_from(variant).unwrap_or(u16::MAX)))
            }
            Some(TypeRef::Record(index)) => {
                let object = json
                    .as_object()
                    .ok_or_else(|| invalid_type(json, &format!("struct {name}")))?;
                let fields =
                    fields_from_object(types, &types.records[index].fields, object, Keys::Struct)?;
                Ok(Value::record(fields))
            }
            Some(TypeRef::Union(index)) => {
                let object = json
                    .as_object()
                    .ok_or_else(|| invalid_type(json, &format!("enum {name}")))?;
                let kind = object
                    .get("kind")
                    .and_then(Json::as_str)
                    .ok_or_else(|| "missing field `kind`".to_owned())?;
                let def = &types.unions[index];
                let tag = def
                    .variants
                    .iter()
                    .position(|variant| variant.name == kind)
                    .ok_or_else(|| {
                        let names: Vec<String> =
                            def.variants.iter().map(|v| v.name.clone()).collect();
                        format!(
                            "unknown variant `{kind}`, expected {}",
                            expected_list(&names, "variants")
                        )
                    })?;
                let fields =
                    fields_from_object(types, &def.variants[tag].fields, object, Keys::Variant)?;
                Ok(Value::variant(
                    u16::try_from(tag).unwrap_or(u16::MAX),
                    fields,
                ))
            }
            None => Err(format!("{name} is not a type")),
        },
        other => Err(format!("{} is not data", other.describe())),
    }
}

/// The item of a frame from the event that carries it: the item type's fields are the
/// event's, by the names the contract spells them.
pub(crate) fn item_from_event(types: &TypeTable, item: &Ty, event: &Json) -> Result<Value, String> {
    let Ty::Named(name) = item else {
        return Err(format!("{} is not an item type", item.describe()));
    };
    match types.get(name) {
        Some(TypeRef::Record(index)) => {
            let object = event
                .as_object()
                .ok_or_else(|| invalid_type(event, "an event"))?;
            let fields =
                fields_from_object(types, &types.records[index].fields, object, Keys::Variant)?;
            Ok(Value::record(fields))
        }
        Some(TypeRef::Union(_)) => from_json(types, item, event),
        _ => Err(format!("{name} is not an item type")),
    }
}

/// The configuration of `program` from `given`, which may leave fields to their defaults.
pub(crate) fn config_from_json(program: &Program, given: &Json) -> Result<Value, String> {
    let types = program.types();
    let (_, record) = types
        .record(program.config_type())
        .ok_or_else(|| "the configuration is not a record".to_owned())?;
    let empty = Map::new();
    let given = match given {
        Json::Null => &empty,
        Json::Object(object) => object,
        other => return Err(invalid_type(other, &format!("struct {}", record.name))),
    };
    let names: Vec<String> = record
        .fields
        .iter()
        .map(|field| snake(&field.name))
        .collect();
    for key in given.keys() {
        if !names.contains(key) {
            return Err(format!(
                "unknown field `{key}`, expected {}",
                expected_list(&names, "fields")
            ));
        }
    }
    let defaults = program.defaults().as_object().cloned().unwrap_or_default();
    let mut fields = Vec::with_capacity(record.fields.len());
    for field in &record.fields {
        let name = snake(&field.name);
        let json = given
            .get(&name)
            .or_else(|| defaults.get(&field.name))
            .unwrap_or(&Json::Null);
        fields.push(from_json(types, &field.ty, json)?);
    }
    Ok(Value::record(fields))
}

/// A port property from a value of `ty`: integers, booleans, text and enum names.
pub(crate) fn prop_value(types: &TypeTable, ty: &Ty, value: &Value) -> Result<PropValue, String> {
    match (ty, value) {
        (_, Value::Bool(b)) => Ok(PropValue::Bool(*b)),
        (_, Value::Int(n)) => Ok(PropValue::Int(i64::try_from(*n).unwrap_or(i64::MAX))),
        (_, Value::Text(text)) => Ok(PropValue::Text(text.to_string())),
        (Ty::Named(name), Value::Enum(variant)) => types
            .enumeration(name)
            .and_then(|(_, def)| def.variants.get(usize::from(*variant)))
            .map(|name| PropValue::Text(name.clone()))
            .ok_or_else(|| format!("{name} is not an enum")),
        (ty, _) => Err(format!("a {} is not a property value", ty.describe())),
    }
}

/// An integer property bound from a value.
pub(crate) fn bound(value: &Value) -> Result<i64, String> {
    match value {
        Value::Int(n) => Ok(i64::try_from(*n).unwrap_or(i64::MAX)),
        other => Err(format!("{} is not an integer", other.describe())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_convert_as_serde_names_fields() {
        assert_eq!(snake("periodTicks"), "period_ticks");
        assert_eq!(snake("wordBits"), "word_bits");
        assert_eq!(snake("csActiveLow"), "cs_active_low");
        assert_eq!(snake("line"), "line");
        assert_eq!(expected_list(&["a".into()], "fields"), "`a`");
        assert_eq!(
            expected_list(&["a".into(), "b".into()], "fields"),
            "`a` or `b`"
        );
        assert_eq!(
            expected_list(&["a".into(), "b".into(), "c".into()], "fields"),
            "one of `a`, `b`, `c`"
        );
        assert_eq!(expected_list(&[], "fields"), "there are no fields");
    }

    #[test]
    fn json_types_are_described_as_serde_describes_them() {
        assert_eq!(
            invalid_type(&Json::from(1.5), "u8"),
            "invalid type: floating point `1.5`, expected u8"
        );
        assert_eq!(
            invalid_type(&Json::from("x"), "u8"),
            "invalid type: string \"x\", expected u8"
        );
        assert_eq!(describe_json(&Json::Null), "null");
        assert_eq!(describe_json(&Json::Array(vec![])), "sequence");
    }
}
