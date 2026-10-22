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

fn field_to_json(types: &TypeTable, field: &FieldDef, value: &Value) -> Json {
    match (field.json, value) {
        (Some(uiharu_ir::FieldJson::HexU64), Value::Int(value)) => {
            Json::String(format!("0x{value:x}"))
        }
        _ => to_json(types, &field.ty, value),
    }
}

fn field_from_json(types: &TypeTable, field: &FieldDef, json: &Json) -> Result<Value, String> {
    match field.json {
        Some(uiharu_ir::FieldJson::HexU64) => uiharu_core::hex_u64::deserialize(json)
            .map(Value::Int)
            .map_err(|error| error.to_string()),
        None => from_json(types, &field.ty, json),
    }
}

/// `value` of type `ty` as JSON.
pub(crate) fn to_json(types: &TypeTable, ty: &Ty, value: &Value) -> Json {
    match (ty, value) {
        (_, Value::None) => Json::Null,
        (Ty::Option(inner), value) => to_json(types, inner, value),
        (Ty::Bool, Value::Bool(b)) => Json::Bool(*b),
        (ty, Value::Int(n)) if ty.is_int() => Json::from(*n),
        (Ty::Text, Value::Text(text)) => Json::String(text.to_string()),
        (Ty::Array(item, _) | Ty::FixedArray(item, _), Value::Array(items)) => {
            Json::Array(items.iter().map(|v| to_json(types, item, v)).collect())
        }
        (Ty::Named(name), value) => match (types.get(name), value) {
            (Some(TypeRef::Enum(index)), Value::Enum(variant)) => {
                let def = &types.enums[index];
                let index = usize::from(*variant);
                match &def.numbers {
                    Some(numbers) => numbers
                        .get(index)
                        .map_or(Json::Null, |value| Json::from(*value)),
                    None => def
                        .variants
                        .get(index)
                        .cloned()
                        .map_or(Json::Null, Json::String),
                }
            }
            (Some(TypeRef::Record(index)), Value::Record(fields)) => {
                let mut object = Map::new();
                for (field, value) in types.records[index].fields.iter().zip(fields.iter()) {
                    object.insert(key(field, Keys::Struct), field_to_json(types, field, value));
                }
                Json::Object(object)
            }
            (Some(TypeRef::Union(index)), Value::Variant(tag, fields)) => {
                let Some(variant) = types.unions[index].variants.get(usize::from(*tag)) else {
                    return Json::Null;
                };
                if types.unions[index].external {
                    return match variant.fields.first().zip(fields.first()) {
                        Some((field, value)) => {
                            let mut object = Map::new();
                            object.insert(variant.name.clone(), field_to_json(types, field, value));
                            Json::Object(object)
                        }
                        None => Json::String(variant.name.clone()),
                    };
                }
                let mut object = Map::new();
                object.insert("kind".into(), Json::String(variant.name.clone()));
                for (field, value) in variant.fields.iter().zip(fields.iter()) {
                    object.insert(
                        key(field, Keys::Variant),
                        field_to_json(types, field, value),
                    );
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
            Some(json) => values.push(field_from_json(types, field, json)?),
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
        Ty::Array(item, _) | Ty::FixedArray(item, _) => {
            let items = json
                .as_array()
                .ok_or_else(|| invalid_type(json, "a sequence"))?;
            if let Ty::FixedArray(_, length) = ty
                && items.len() != *length as usize
            {
                return Err(format!(
                    "invalid length {}, expected exactly {length} items",
                    items.len()
                ));
            }
            let values = items
                .iter()
                .map(|json| from_json(types, item, json))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(Value::Array(std::rc::Rc::new(values)))
        }
        Ty::Named(name) => match types.get(name) {
            Some(TypeRef::Enum(index)) => {
                if let Some(numbers) = &types.enums[index].numbers {
                    let number = json
                        .as_u64()
                        .ok_or_else(|| invalid_type(json, "a numeric enum value"))?;
                    let variant = numbers
                        .iter()
                        .position(|value| *value == number)
                        .ok_or_else(|| format!("unknown numeric variant `{number}` of {name}"))?;
                    return Ok(Value::Enum(
                        u16::try_from(variant)
                            .map_err(|_| format!("too many variants in {name}"))?,
                    ));
                }
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
                let def = &types.unions[index];
                if def.external {
                    let (kind, payload) = match json {
                        Json::String(kind) => (kind.as_str(), None),
                        Json::Object(object) if object.len() == 1 => {
                            let (kind, value) =
                                object.iter().next().expect("one entry was checked");
                            (kind.as_str(), Some(value))
                        }
                        _ => {
                            return Err(invalid_type(
                                json,
                                "an enum string or a single-key enum object",
                            ));
                        }
                    };
                    let tag = def
                        .variants
                        .iter()
                        .position(|variant| variant.name == kind)
                        .ok_or_else(|| format!("unknown variant `{kind}` of {name}"))?;
                    let fields = match (def.variants[tag].fields.first(), payload) {
                        (None, None | Some(Json::Null)) => Vec::new(),
                        (Some(field), Some(json)) => vec![field_from_json(types, field, json)?],
                        (Some(_), None) => {
                            return Err(format!("variant `{kind}` needs its payload"));
                        }
                        (None, Some(_)) => {
                            return Err(format!("unit variant `{kind}` has no payload"));
                        }
                    };
                    return Ok(Value::variant(
                        u16::try_from(tag).map_err(|_| format!("too many variants in {name}"))?,
                        fields,
                    ));
                }
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

/// Source defaults use declared field names; user JSON uses the contract's wire names.
/// Normalize only the former, recursively, before applying the same wire validation.
fn default_json(types: &TypeTable, ty: &Ty, json: &Json) -> Json {
    match ty {
        Ty::Option(inner) if !json.is_null() => default_json(types, inner, json),
        Ty::Array(item, _) | Ty::FixedArray(item, _) => json.as_array().map_or_else(
            || json.clone(),
            |items| {
                Json::Array(
                    items
                        .iter()
                        .map(|value| default_json(types, item, value))
                        .collect(),
                )
            },
        ),
        Ty::Named(name) => match (types.get(name), json.as_object()) {
            (Some(TypeRef::Record(index)), Some(object)) => {
                let mut normalized = object.clone();
                for field in &types.records[index].fields {
                    let wire = key(field, Keys::Struct);
                    if let Some(value) = object.get(&field.name).or_else(|| object.get(&wire)) {
                        normalized.remove(&field.name);
                        normalized.insert(wire, default_json(types, &field.ty, value));
                    }
                }
                Json::Object(normalized)
            }
            (Some(TypeRef::Union(index)), Some(object)) => {
                let def = &types.unions[index];
                let mut normalized = object.clone();
                if def.external {
                    if object.len() == 1
                        && let Some((kind, value)) = object.iter().next()
                        && let Some(field) = def
                            .variants
                            .iter()
                            .find(|variant| variant.name == *kind)
                            .and_then(|variant| variant.fields.first())
                    {
                        normalized.insert(kind.clone(), default_json(types, &field.ty, value));
                    }
                } else if let Some(kind) = object.get("kind").and_then(Json::as_str)
                    && let Some(variant) = def.variants.iter().find(|variant| variant.name == kind)
                {
                    for field in &variant.fields {
                        if let Some(value) = object.get(&field.name) {
                            normalized
                                .insert(field.name.clone(), default_json(types, &field.ty, value));
                        }
                    }
                }
                Json::Object(normalized)
            }
            _ => json.clone(),
        },
        _ => json.clone(),
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
        if let Some(json) = given.get(&name) {
            fields.push(field_from_json(types, field, json)?);
        } else {
            let json = default_json(
                types,
                &field.ty,
                defaults.get(&field.name).unwrap_or(&Json::Null),
            );
            fields.push(field_from_json(types, field, &json)?);
        }
    }
    Ok(Value::record(fields))
}

/// A port property from a value of `ty`: integers, booleans, text and enum names.
pub(crate) fn prop_value(types: &TypeTable, ty: &Ty, value: &Value) -> Result<PropValue, String> {
    match (ty, value) {
        (_, Value::Bool(b)) => Ok(PropValue::Bool(*b)),
        (_, Value::Int(n)) => Ok(PropValue::Int(i64::try_from(*n).unwrap_or(i64::MAX))),
        (_, Value::Text(text)) => Ok(PropValue::Text(text.to_string())),
        (Ty::Named(name), Value::Enum(variant)) => {
            let (_, def) = types
                .enumeration(name)
                .ok_or_else(|| format!("{name} is not an enum"))?;
            let index = usize::from(*variant);
            if let Some(numbers) = &def.numbers {
                numbers
                    .get(index)
                    .map(|value| PropValue::Int(i64::try_from(*value).unwrap_or(i64::MAX)))
                    .ok_or_else(|| format!("{name} has no variant at {index}"))
            } else {
                def.variants
                    .get(index)
                    .map(|name| PropValue::Text(name.clone()))
                    .ok_or_else(|| format!("{name} has no variant at {index}"))
            }
        }
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

    fn contract_types() -> TypeTable {
        let module = serde_json::from_value(serde_json::json!({
            "ir": 1, "name": "native-contract-types", "types": [
                { "name": "Clause", "type": "numeric-enum", "variants": [
                    { "name": "c22", "value": 22 }, { "name": "c45", "value": 45 }
                ] },
                { "name": "Ack", "type": "external-enum", "variants": [
                    { "name": "ok" }, { "name": "wait" }, { "name": "fault" },
                    { "name": "other", "payload": "u8" }
                ] },
                { "name": "Reading", "type": "record", "fields": [
                    { "name": "romId", "type": "u64", "json": "hex-u64" },
                    { "name": "bytes", "type": { "fixedArray": "u8", "length": 5 } },
                    { "name": "clause", "type": "Clause" },
                    { "name": "ack", "type": "Ack" }
                ] }
            ]
        }))
        .expect("the native contract shapes parse");
        TypeTable::from_module(&module).expect("the contract types resolve")
    }

    #[test]
    fn native_contract_shapes_round_trip_without_changing_wire_values() {
        let types = contract_types();
        let ty = Ty::Named("Reading".into());
        let wire = serde_json::json!({
            "rom_id": "0xffffffffffffffff", "bytes": [1, 2, 3, 4, 5],
            "clause": 45, "ack": { "other": 7 }
        });
        let value = from_json(&types, &ty, &wire).expect("all exact native shapes decode");
        assert_eq!(to_json(&types, &ty, &value), wire);
        for ack in [
            serde_json::json!("ok"),
            serde_json::json!("wait"),
            serde_json::json!("fault"),
        ] {
            let value = from_json(&types, &Ty::Named("Ack".into()), &ack).expect("unit variant");
            assert_eq!(to_json(&types, &Ty::Named("Ack".into()), &value), ack);
        }
    }

    #[test]
    fn fixed_arrays_reject_short_long_and_invalid_elements() {
        let types = contract_types();
        let ty = Ty::FixedArray(Box::new(Ty::U8), 5);
        for wire in [
            serde_json::json!([]),
            serde_json::json!([1, 2, 3, 4]),
            serde_json::json!([1, 2, 3, 4, 5, 6]),
            serde_json::json!([1, 2, 3, 4, 256]),
        ] {
            assert!(from_json(&types, &ty, &wire).is_err(), "accepted {wire}");
        }
    }

    #[test]
    fn source_defaults_normalize_record_names_without_changing_user_wire_json() {
        let types = contract_types();
        let ty = Ty::Named("Reading".into());
        let source = serde_json::json!({ "romId": "0xff", "bytes": [1, 2, 3, 4, 5], "clause": 22, "ack": "ok" });
        assert!(
            from_json(&types, &ty, &source).is_err(),
            "a user record must use its wire keys"
        );
        let normalized = default_json(&types, &ty, &source);
        let value = from_json(&types, &ty, &normalized).expect("source defaults use declared keys");
        assert_eq!(
            to_json(&types, &ty, &value),
            serde_json::json!({ "rom_id": "0xff", "bytes": [1, 2, 3, 4, 5], "clause": 22, "ack": "ok" })
        );
    }

    #[test]
    fn numeric_and_external_enums_reject_invalid_wire_values() {
        let types = contract_types();
        for wire in [
            serde_json::json!(23),
            serde_json::json!("c22"),
            serde_json::json!(22.5),
            serde_json::json!(-1),
        ] {
            assert!(
                from_json(&types, &Ty::Named("Clause".into()), &wire).is_err(),
                "accepted {wire}"
            );
        }
        for wire in [
            serde_json::json!("other"),
            serde_json::json!({ "other": 256 }),
            serde_json::json!({ "other": 7, "ok": null }),
            serde_json::json!({}),
            serde_json::json!({ "unknown": 7 }),
            serde_json::json!({ "ok": 7 }),
        ] {
            assert!(
                from_json(&types, &Ty::Named("Ack".into()), &wire).is_err(),
                "accepted {wire}"
            );
        }
    }

    #[test]
    fn hex_fields_follow_the_native_serializer_without_relaxing_plain_integers() {
        #[derive(serde::Deserialize, serde::Serialize)]
        struct Native {
            #[serde(with = "uiharu_core::hex_u64")]
            rom_id: u64,
        }
        let types = contract_types();
        let field = &types.records[0].fields[0];
        for wire in [
            serde_json::json!(0),
            serde_json::json!(u64::MAX),
            serde_json::json!("FF"),
            serde_json::json!("0XfF"),
            serde_json::json!("0xffffffffffffffff"),
        ] {
            let native: Native = serde_json::from_value(serde_json::json!({ "rom_id": wire }))
                .expect("native accepts");
            let value = field_from_json(&types, field, &wire).expect("interpreter accepts");
            assert_eq!(value, Value::Int(native.rom_id));
            assert_eq!(
                field_to_json(&types, field, &value),
                serde_json::to_value(native).expect("native encodes")["rom_id"]
            );
        }
        for wire in [
            serde_json::json!("0x10000000000000000"),
            serde_json::json!("0xzz"),
            serde_json::json!(-1),
            serde_json::json!(1.5),
            Json::Null,
        ] {
            assert!(
                field_from_json(&types, field, &wire).is_err(),
                "accepted {wire}"
            );
        }
        assert!(from_json(&types, &Ty::U64, &serde_json::json!("0xff")).is_err());
    }

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
